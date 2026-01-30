use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode;

use crate::extract::{Protocol, ProxyIp};

/// Returns `true` if the request should be considered secure (HTTPS).
///
/// A request is secure if any of:
/// 1. The [`Protocol::Tls`] extension is present (direct TLS connection).
/// 2. A [`ProxyIp`] extension is present (set by `resolve_ip` middleware
///    for trusted proxies) AND the `X-Forwarded-Proto` header equals `"https"`.
pub fn is_secure_request<B>(req: &http::Request<B>) -> bool {
    if req.extensions().get::<Protocol>() == Some(&Protocol::Tls) {
        return true;
    }

    // ProxyIp is only present when the peer is a known trusted proxy
    // (injected by resolve_ip middleware).
    if req.extensions().get::<ProxyIp>().is_none() {
        return false;
    }

    // TODO: support `proto=https` inside the RFC 7239 `Forwarded` header.
    req.headers()
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

/// Middleware that rejects requests that are not secure (HTTPS).
/// Returns 403 Forbidden with a plain-text body.
pub async fn require_https(req: Request, next: Next) -> Response {
    if is_secure_request(&req) {
        next.run(req).await
    } else {
        (
            StatusCode::FORBIDDEN,
            "HTTPS is required for this endpoint\n",
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use http::Request;

    use crate::extract::{Protocol, ProxyIp};

    use super::is_secure_request;

    fn build_request() -> http::request::Builder {
        Request::builder().uri("/api/v1/ws/agent")
    }

    #[test]
    fn tls_protocol_extension() {
        let mut req = build_request().body(()).unwrap();
        req.extensions_mut().insert(Protocol::Tls);
        assert!(is_secure_request(&req));
    }

    #[test]
    fn plain_protocol_extension() {
        let mut req = build_request().body(()).unwrap();
        req.extensions_mut().insert(Protocol::Plain);
        assert!(!is_secure_request(&req));
    }

    #[test]
    fn no_extension_no_proxy() {
        let req = build_request().body(()).unwrap();
        assert!(!is_secure_request(&req));
    }

    #[test]
    fn trusted_proxy_with_forwarded_https() {
        let mut req = build_request()
            .header("x-forwarded-proto", "https")
            .body(())
            .unwrap();
        // ProxyIp presence indicates a trusted proxy
        req.extensions_mut()
            .insert(ProxyIp("192.168.1.10".parse::<IpAddr>().unwrap()));
        assert!(is_secure_request(&req));
    }

    #[test]
    fn trusted_proxy_no_forwarded_header() {
        let mut req = build_request().body(()).unwrap();
        req.extensions_mut()
            .insert(ProxyIp("192.168.1.10".parse::<IpAddr>().unwrap()));
        assert!(!is_secure_request(&req));
    }

    #[test]
    fn trusted_proxy_forwarded_http() {
        let mut req = build_request()
            .header("x-forwarded-proto", "http")
            .body(())
            .unwrap();
        req.extensions_mut()
            .insert(ProxyIp("192.168.1.10".parse::<IpAddr>().unwrap()));
        assert!(!is_secure_request(&req));
    }

    #[test]
    fn no_proxy_extension_with_forwarded_https_is_spoofing() {
        // No ProxyIp extension means untrusted — header should be ignored
        let req = build_request()
            .header("x-forwarded-proto", "https")
            .body(())
            .unwrap();
        assert!(!is_secure_request(&req));
    }

    #[test]
    fn forwarded_proto_case_insensitive() {
        let mut req = build_request()
            .header("x-forwarded-proto", "HTTPS")
            .body(())
            .unwrap();
        req.extensions_mut()
            .insert(ProxyIp("192.168.1.10".parse::<IpAddr>().unwrap()));
        assert!(is_secure_request(&req));
    }

    #[test]
    fn ipv6_trusted_proxy() {
        let mut req = build_request()
            .header("x-forwarded-proto", "https")
            .body(())
            .unwrap();
        req.extensions_mut()
            .insert(ProxyIp("::1".parse::<IpAddr>().unwrap()));
        assert!(is_secure_request(&req));
    }
}
