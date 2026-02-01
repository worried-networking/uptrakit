use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use http::HeaderMap;

use crate::AppState;
use crate::extract::{AgentIdentity, ExternalBaseUrl, ProxyIp};

/// Middleware that handles reverse proxy forwarded headers:
///
/// **Part A — Client certificate identity:**
/// Parses agent identity from forwarded certificate headers when the request
/// comes from a trusted proxy. Supports both structured info headers (Traefik,
/// Nginx, HAProxy) and raw PEM headers (Caddy, Envoy XFCC fallback).
///
/// **Part B — External base URL:**
/// Resolves the external-facing base URL from `Origin`, `X-Forwarded-Proto` +
/// `X-Forwarded-Host`, or the `Host` header. Used for OIDC redirect URLs and
/// device auth verification URLs.
///
/// Runs **after** `resolve_ip` (which sets `ProxyIp` for trusted proxies).
/// Header stripping prevents spoofing from non-proxy clients.
pub async fn resolve_proxy_headers(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let network = state.settings.network().await;
    let from_trusted_proxy = req.extensions().get::<ProxyIp>().is_some();
    let has_mtls_identity = req.extensions().get::<AgentIdentity>().is_some();

    // --- Part A: Client certificate identity ---
    if !has_mtls_identity {
        if from_trusted_proxy {
            // Try info header first, then PEM as fallback
            let identity = try_info_header(
                req.headers(),
                network.forwarded_client_cert_info_header.as_deref(),
                &state,
            )
            .or_else(|| {
                try_pem_header(
                    req.headers(),
                    network.forwarded_client_cert_pem_header.as_deref(),
                    &state,
                )
            });

            if let Some(id) = identity {
                req.extensions_mut().insert(id);
            }
        } else {
            // Not from trusted proxy: strip all cert-related headers to prevent
            // spoofing. Also strip X-Forwarded-Proto and X-Forwarded-Host.
            strip_proxy_headers(
                req.headers_mut(),
                network.forwarded_client_cert_info_header.as_deref(),
                network.forwarded_client_cert_pem_header.as_deref(),
            );
        }
    }

    // --- Part B: External base URL ---
    let base_url = resolve_external_base_url(req.headers(), from_trusted_proxy);
    if let Some(url) = base_url {
        req.extensions_mut().insert(ExternalBaseUrl(url));
    }

    next.run(req).await
}

/// Try to extract agent identity from a structured info header.
///
/// The header value is URL-decoded, then parsed as semicolon-separated
/// `key="value"` pairs. Expected fields: `Subject`, `Issuer`,
/// `SerialNumber`/`Serial`.
///
/// When a `Cert` field is present (Envoy XFCC), it is preferred because
/// it carries the full certificate — providing both the agent UUID and the
/// serial number. Falls back to Subject/SerialNumber/Issuer fields used
/// by Traefik, Nginx, and HAProxy.
fn try_info_header(
    headers: &HeaderMap,
    header_name: Option<&str>,
    state: &AppState,
) -> Option<AgentIdentity> {
    let header_name = header_name?;
    let raw = headers.get(header_name)?.to_str().ok()?;
    // Traefik uses form-URL-encoding (+ = space); urlencoding only handles
    // percent-encoding, so normalise '+' first.
    let raw_normalised = raw.replace('+', " ");
    let decoded = urlencoding::decode(&raw_normalised).ok()?;

    // Parse semicolon-separated key="value" or key=value pairs
    let fields = parse_info_fields(&decoded);

    // 1. Cert field (Envoy XFCC) — provides complete identity
    if let Some(cert_field) = fields.get("Cert").or(fields.get("cert")) {
        if let Some(identity) = try_parse_cert_field(cert_field, state) {
            return Some(identity);
        }
    }

    // 2. Subject/SerialNumber/Issuer (Traefik, Nginx, HAProxy) — fallback
    if let Some(subject) = fields.get("Subject").or(fields.get("subject")) {
        let subject_decoded = urlencoding::decode(subject).ok()?;
        let issuer_raw = fields
            .get("Issuer")
            .or(fields.get("issuer"))
            .map(|s| s.as_str());

        let serial = fields
            .get("SerialNumber")
            .or(fields.get("serialNumber"))
            .or(fields.get("Serial"))
            .or(fields.get("serial"))
            .map(|s| s.as_str());

        // Verify issuer CN against known CA CNs
        if let Some(issuer) = issuer_raw {
            let issuer_decoded = urlencoding::decode(issuer).ok()?;
            if !verify_issuer_cn(&issuer_decoded, state) {
                tracing::warn!(
                    issuer = %issuer_decoded,
                    "forwarded cert issuer CN does not match any known CA"
                );
                return None;
            }
        }

        let identity = crate::extract::agent_identity_from_info(&subject_decoded, serial);
        return identity;
    }

    None
}

/// Parse a Cert field value (URL-encoded PEM or base64-DER).
fn try_parse_cert_field(cert_field: &str, state: &AppState) -> Option<AgentIdentity> {
    let cert_decoded = urlencoding::decode(cert_field).ok()?;

    // Try PEM first (Envoy sends URL-encoded PEM)
    if let Ok((_, pem_block)) = x509_parser::pem::parse_x509_pem(cert_decoded.as_bytes()) {
        let identity = crate::extract::agent_identity_from_der(&pem_block.contents)?;
        let (_, cert) = x509_parser::parse_x509_certificate(&pem_block.contents).ok()?;
        let issuer_cn = cert.issuer().iter_common_name().next()?.as_str().ok()?;
        if !verify_issuer_cn_str(issuer_cn, state) {
            tracing::warn!(issuer = issuer_cn, "forwarded cert issuer CN does not match any known CA");
            return None;
        }
        return Some(identity);
    }

    // Fallback: raw base64-DER
    let der = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        cert_decoded.as_bytes(),
    )
    .ok()?;
    let identity = crate::extract::agent_identity_from_der(&der)?;
    let (_, cert) = x509_parser::parse_x509_certificate(&der).ok()?;
    let issuer_cn = cert.issuer().iter_common_name().next()?.as_str().ok()?;
    if !verify_issuer_cn_str(issuer_cn, state) {
        tracing::warn!(issuer = issuer_cn, "forwarded cert issuer CN does not match any known CA");
        return None;
    }
    Some(identity)
}

/// Try to extract agent identity from a PEM-encoded (or base64-DER) certificate header.
///
/// Supports URL-encoded PEM (Caddy `certificate_pem`) and raw base64-DER
/// (Caddy `certificate_der_base64`, Envoy fallback).
fn try_pem_header(
    headers: &HeaderMap,
    header_name: Option<&str>,
    state: &AppState,
) -> Option<AgentIdentity> {
    let header_name = header_name?;
    let raw = headers.get(header_name)?.to_str().ok()?;
    let decoded = urlencoding::decode(raw).ok()?;

    // Try PEM first
    let der = if let Ok((_, pem_block)) = x509_parser::pem::parse_x509_pem(decoded.as_bytes()) {
        pem_block.contents
    } else {
        // Fallback: base64-DER (Caddy certificate_der_base64)
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            decoded.as_bytes(),
        )
        .ok()?
    };

    let identity = crate::extract::agent_identity_from_der(&der)?;

    // Verify issuer CN from the parsed cert
    let (_, cert) = x509_parser::parse_x509_certificate(&der).ok()?;
    let issuer_cn = cert.issuer().iter_common_name().next()?.as_str().ok()?;
    if !verify_issuer_cn_str(issuer_cn, state) {
        tracing::warn!(
            issuer = issuer_cn,
            "forwarded PEM cert issuer CN does not match any known CA"
        );
        return None;
    }

    Some(identity)
}

/// Parse semicolon-separated `key="value"` or `key=value` pairs from info
/// header content.
fn parse_info_fields(input: &str) -> std::collections::HashMap<String, String> {
    let mut fields = std::collections::HashMap::new();
    // Handle both semicolon-separated and comma-separated (XFCC) formats
    let separator = if input.contains(';') { ';' } else { ',' };
    for part in input.split(separator) {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            fields.insert(key.to_string(), value.to_string());
        }
    }
    fields
}

/// Verify that the issuer CN (from a DN string) matches a known CA CN.
fn verify_issuer_cn(issuer_dn: &str, state: &AppState) -> bool {
    let issuer_cn = match crate::extract::extract_cn_from_dn(issuer_dn) {
        Some(cn) => cn,
        None => return false,
    };
    verify_issuer_cn_str(issuer_cn, state)
}

/// Verify that the issuer CN string matches a known CA CN.
fn verify_issuer_cn_str(issuer_cn: &str, state: &AppState) -> bool {
    let snapshot = state.ca_snapshot.borrow().clone();

    // Check active CA CN
    if let Some(active_cn) = ca_cn_from_pem(&snapshot.active_cert_pem)
        && active_cn == issuer_cn
    {
        return true;
    }

    // Check previous CA CN (only if not expired)
    if let Some(ref prev_pem) = snapshot.previous_cert_pem
        && let Some(prev_cn) = ca_cn_from_pem(prev_pem)
        && prev_cn == issuer_cn
        && !is_cert_expired(prev_pem)
    {
        return true;
    }

    false
}

/// Extract the CN from a PEM-encoded certificate.
fn ca_cn_from_pem(pem: &str) -> Option<String> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(&pem_block.contents).ok()?;
    let cn = cert.subject().iter_common_name().next()?.as_str().ok()?;
    Some(cn.to_string())
}

/// Check if a PEM-encoded certificate is expired.
fn is_cert_expired(pem: &str) -> bool {
    let Ok((_, pem_block)) = x509_parser::pem::parse_x509_pem(pem.as_bytes()) else {
        return true;
    };
    let Ok((_, cert)) = x509_parser::parse_x509_certificate(&pem_block.contents) else {
        return true;
    };
    let not_after = cert.validity().not_after.to_datetime();
    let now = time::OffsetDateTime::now_utc();
    now > not_after
}

/// Strip proxy-related headers from non-proxy requests to prevent spoofing.
fn strip_proxy_headers(
    headers: &mut HeaderMap,
    info_header: Option<&str>,
    pem_header: Option<&str>,
) {
    if let Some(h) = info_header {
        headers.remove(h);
    }
    if let Some(h) = pem_header {
        headers.remove(h);
    }
    headers.remove("x-forwarded-proto");
    headers.remove("x-forwarded-host");
}

/// Resolve the external base URL from headers.
///
/// Priority:
/// 1. `Origin` header (includes protocol)
/// 2. `X-Forwarded-Proto` + `X-Forwarded-Host` (trusted proxy only)
/// 3. `X-Forwarded-Proto` + `Host` (trusted proxy only)
/// 4. `Host` header with `https://`
fn resolve_external_base_url(headers: &HeaderMap, from_trusted_proxy: bool) -> Option<String> {
    // 1. Origin header
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim_end_matches('/'));
    if let Some(o) = origin
        && !o.is_empty()
    {
        return Some(o.to_string());
    }

    // 2. X-Forwarded-Proto + X-Forwarded-Host (trusted proxy only)
    if from_trusted_proxy {
        let proto = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok());
        let fwd_host = headers
            .get("x-forwarded-host")
            .and_then(|v| v.to_str().ok());

        if let (Some(proto), Some(host)) = (proto, fwd_host) {
            return Some(format!("{}://{}", proto, host.trim_end_matches('/')));
        }

        // 3. X-Forwarded-Proto + Host
        if let Some(proto) = proto
            && let Some(host) = headers.get("host").and_then(|v| v.to_str().ok())
        {
            return Some(format!("{}://{}", proto, host.trim_end_matches('/')));
        }
    }

    // 4. Host with https://
    headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .map(|h| format!("https://{}", h.trim_end_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    fn parse_info_fields_semicolon_separated() {
        let input = r#"Subject="CN=550e8400-e29b-41d4-a716-446655440000,O=Uptrakit";Issuer="CN=Uptrakit CA";SerialNumber="01ABCDEF""#;
        let fields = parse_info_fields(input);
        assert_eq!(
            fields.get("Subject").map(|s| s.as_str()),
            Some("CN=550e8400-e29b-41d4-a716-446655440000,O=Uptrakit")
        );
        assert_eq!(
            fields.get("Issuer").map(|s| s.as_str()),
            Some("CN=Uptrakit CA")
        );
        assert_eq!(
            fields.get("SerialNumber").map(|s| s.as_str()),
            Some("01ABCDEF")
        );
    }

    #[test]
    fn parse_info_fields_xfcc_comma_separated() {
        let input = r#"Subject="CN=test",Issuer="CN=CA""#;
        let fields = parse_info_fields(input);
        assert_eq!(fields.get("Subject").map(|s| s.as_str()), Some("CN=test"));
        assert_eq!(fields.get("Issuer").map(|s| s.as_str()), Some("CN=CA"));
    }

    #[test]
    fn strip_proxy_headers_removes_configured() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Custom-Info", HeaderValue::from_static("val"));
        headers.insert("X-Custom-Pem", HeaderValue::from_static("val"));
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert("x-forwarded-host", HeaderValue::from_static("example.com"));
        headers.insert("host", HeaderValue::from_static("internal"));

        strip_proxy_headers(&mut headers, Some("X-Custom-Info"), Some("X-Custom-Pem"));

        assert!(headers.get("X-Custom-Info").is_none());
        assert!(headers.get("X-Custom-Pem").is_none());
        assert!(headers.get("x-forwarded-proto").is_none());
        assert!(headers.get("x-forwarded-host").is_none());
        // Host header should not be stripped
        assert!(headers.get("host").is_some());
    }

    #[test]
    fn strip_proxy_headers_no_headers_configured() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));

        strip_proxy_headers(&mut headers, None, None);

        // x-forwarded-proto should still be stripped
        assert!(headers.get("x-forwarded-proto").is_none());
    }

    #[test]
    fn external_base_url_from_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "origin",
            HeaderValue::from_static("https://app.example.com/"),
        );
        headers.insert("host", HeaderValue::from_static("internal:8443"));

        let url = resolve_external_base_url(&headers, false);
        assert_eq!(url, Some("https://app.example.com".to_string()));
    }

    #[test]
    fn external_base_url_from_forwarded_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("app.example.com"),
        );
        headers.insert("host", HeaderValue::from_static("internal:8443"));

        let url = resolve_external_base_url(&headers, true);
        assert_eq!(url, Some("https://app.example.com".to_string()));
    }

    #[test]
    fn external_base_url_forwarded_ignored_for_non_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert("x-forwarded-host", HeaderValue::from_static("attacker.com"));
        headers.insert("host", HeaderValue::from_static("internal:8443"));

        // from_trusted_proxy = false, so X-Forwarded-* should be ignored
        let url = resolve_external_base_url(&headers, false);
        assert_eq!(url, Some("https://internal:8443".to_string()));
    }

    #[test]
    fn external_base_url_proto_plus_host() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("http"));
        headers.insert("host", HeaderValue::from_static("app.example.com:9090"));

        let url = resolve_external_base_url(&headers, true);
        assert_eq!(url, Some("http://app.example.com:9090".to_string()));
    }

    #[test]
    fn external_base_url_host_only() {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("internal:8443"));

        let url = resolve_external_base_url(&headers, false);
        assert_eq!(url, Some("https://internal:8443".to_string()));
    }

    #[test]
    fn external_base_url_no_headers() {
        let headers = HeaderMap::new();
        let url = resolve_external_base_url(&headers, false);
        assert_eq!(url, None);
    }

    #[test]
    fn is_cert_expired_future_cert() {
        // Generate a self-signed cert valid for 365 days
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let cert = rcgen::CertificateParams::new(vec!["test".into()])
            .expect("params")
            .self_signed(&key_pair)
            .expect("self-sign");
        let pem = cert.pem();
        assert!(!is_cert_expired(&pem));
    }

    #[test]
    fn ca_cn_from_pem_extracts_cn() {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params = rcgen::CertificateParams::new(vec![]).expect("params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        let cert = params.self_signed(&key_pair).expect("self-sign");
        let pem = cert.pem();

        assert_eq!(ca_cn_from_pem(&pem), Some("Test CA".to_string()));
    }
}
