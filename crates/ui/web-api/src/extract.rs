use std::net::IpAddr;

/// The resolved client IP address.
/// Set by the `resolve_ip` middleware.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

/// The trusted proxy's IP address.
/// Only present when the peer is a known trusted proxy.
/// Set by the `resolve_ip` middleware.
#[derive(Debug, Clone, Copy)]
pub struct ProxyIp(pub IpAddr);

/// The verified service identity from a client TLS certificate.
/// Injected by the mTLS acceptor when the peer presents a valid cert
/// signed by the internal CA, with a parseable UUID as CN.
#[derive(Debug, Clone)]
pub struct ServiceIdentity {
    pub service_id: uuid::Uuid,
    pub cert_serial: String,
}

/// The external base URL resolved from proxy headers or the Host header.
/// Used for constructing redirect URLs (OIDC callbacks, device auth, etc.)
/// when the controller is behind a reverse proxy.
#[derive(Debug, Clone)]
pub struct ExternalBaseUrl(pub String);

/// Parse [`ServiceIdentity`] from DER-encoded X.509 certificate bytes.
pub fn service_identity_from_der(der: &[u8]) -> Option<ServiceIdentity> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).ok()?;
    let cn = cert.subject().iter_common_name().next()?.as_str().ok()?;
    let service_id = uuid::Uuid::parse_str(cn).ok()?;
    let cert_serial = cert.raw_serial_as_string();
    Some(ServiceIdentity {
        service_id,
        cert_serial,
    })
}

/// Parse [`ServiceIdentity`] from structured info header fields.
///
/// Returns `ServiceIdentity` with `cert_serial = ""` if `serial` is absent,
/// signalling service-id-only lookup.
pub fn service_identity_from_info(
    subject_dn: &str,
    serial: Option<&str>,
) -> Option<ServiceIdentity> {
    let cn = extract_cn_from_dn(subject_dn)?;
    let service_id = uuid::Uuid::parse_str(cn).ok()?;
    let cert_serial = match serial {
        Some(s) => normalize_serial(s),
        None => String::new(),
    };
    Some(ServiceIdentity {
        service_id,
        cert_serial,
    })
}

/// Extract the CN value from a Distinguished Name string.
///
/// Handles formats:
/// - RFC 2253: `CN=uuid,O=Org`
/// - Single: `CN=uuid`
/// - OpenSSL: `/CN=uuid/O=Org`
/// - URL-encoded: `CN%3Duuid`
pub fn extract_cn_from_dn(dn: &str) -> Option<&str> {
    // Try URL-decoded matching first by checking for %3D (case-insensitive)
    if dn.contains("%3D") || dn.contains("%3d") {
        // URL-encoded: find CN%3D or cn%3d
        let lower = dn.to_ascii_lowercase();
        if let Some(start) = lower.find("cn%3d") {
            let value_start = start + 5; // len("cn%3d")
            let value = &dn[value_start..];
            // Value ends at comma, slash, or end
            let end = value.find([',', '/', '+']).unwrap_or(value.len());
            let cn = &value[..end];
            if !cn.is_empty() {
                return Some(cn);
            }
        }
        return None;
    }

    // OpenSSL format: /CN=uuid/O=Org
    if dn.starts_with('/') {
        for part in dn.split('/') {
            if let Some(value) = part.strip_prefix("CN=")
                && !value.is_empty()
            {
                return Some(value);
            }
        }
        return None;
    }

    // RFC 2253 format: CN=uuid,O=Org or CN=uuid
    for part in dn.split(',') {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix("CN=")
            && !value.is_empty()
        {
            return Some(value);
        }
    }

    None
}

/// Normalize a hex serial number to colon-separated lowercase format
/// matching x509-parser's `raw_serial_as_string()` output.
///
/// Examples:
/// - `"01ABCDEF"` → `"01:ab:cd:ef"`
/// - `"01:AB:CD:EF"` → `"01:ab:cd:ef"`
/// - `"01:ab:cd:ef"` → `"01:ab:cd:ef"` (passthrough)
pub fn normalize_serial(hex: &str) -> String {
    // Strip colons and lowercase
    let clean: String = hex
        .chars()
        .filter(|c| *c != ':')
        .flat_map(|c| c.to_lowercase())
        .collect();

    // Insert colons every 2 chars
    let mut result = String::with_capacity(clean.len() + clean.len() / 2);
    for (i, ch) in clean.chars().enumerate() {
        if i > 0 && i % 2 == 0 {
            result.push(':');
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_cn_rfc2253_with_org() {
        assert_eq!(
            extract_cn_from_dn("CN=550e8400-e29b-41d4-a716-446655440000,O=Uptrakit"),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn extract_cn_rfc2253_only() {
        assert_eq!(
            extract_cn_from_dn("CN=550e8400-e29b-41d4-a716-446655440000"),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn extract_cn_openssl_format() {
        assert_eq!(
            extract_cn_from_dn("/CN=550e8400-e29b-41d4-a716-446655440000/O=Uptrakit"),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn extract_cn_url_encoded() {
        assert_eq!(
            extract_cn_from_dn("CN%3D550e8400-e29b-41d4-a716-446655440000"),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn extract_cn_empty_returns_none() {
        assert_eq!(extract_cn_from_dn(""), None);
        assert_eq!(extract_cn_from_dn("O=Org"), None);
        assert_eq!(extract_cn_from_dn("/O=Org"), None);
    }

    #[test]
    fn normalize_serial_compact_hex() {
        assert_eq!(normalize_serial("01ABCDEF"), "01:ab:cd:ef");
    }

    #[test]
    fn normalize_serial_already_colon_separated() {
        assert_eq!(normalize_serial("01:AB:CD:EF"), "01:ab:cd:ef");
    }

    #[test]
    fn normalize_serial_passthrough() {
        assert_eq!(normalize_serial("01:ab:cd:ef"), "01:ab:cd:ef");
    }

    #[test]
    fn normalize_serial_single_byte() {
        assert_eq!(normalize_serial("0A"), "0a");
    }

    #[test]
    fn identity_from_info_with_serial() {
        let id = service_identity_from_info(
            "CN=550e8400-e29b-41d4-a716-446655440000,O=Uptrakit",
            Some("01ABCDEF"),
        )
        .expect("should parse");
        assert_eq!(
            id.service_id,
            uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
        assert_eq!(id.cert_serial, "01:ab:cd:ef");
    }

    #[test]
    fn identity_from_info_without_serial() {
        let id = service_identity_from_info("CN=550e8400-e29b-41d4-a716-446655440000", None)
            .expect("should parse");
        assert_eq!(
            id.service_id,
            uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
        assert_eq!(id.cert_serial, "");
    }

    #[test]
    fn identity_from_info_non_uuid_cn() {
        assert!(service_identity_from_info("CN=not-a-uuid,O=Org", Some("01")).is_none());
    }

    #[test]
    fn identity_from_der_valid_cert() {
        // Generate a test certificate with a UUID CN
        let service_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, service_id.to_string());
        let cert = params.self_signed(&key_pair).expect("self-sign");

        let identity = service_identity_from_der(cert.der()).expect("should parse DER cert");
        assert_eq!(identity.service_id, service_id);
        assert!(!identity.cert_serial.is_empty());
    }

    #[test]
    fn identity_from_der_non_uuid_cn() {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "not-a-uuid");
        let cert = params.self_signed(&key_pair).expect("self-sign");

        assert!(service_identity_from_der(cert.der()).is_none());
    }
}
