use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use ipnet::IpNet;

use crate::AppState;
use crate::extract::Protocol;

/// Returns `true` if the request should be considered secure (HTTPS).
///
/// A request is secure if any of:
/// 1. The [`Protocol::Tls`] extension is present (direct TLS connection).
/// 2. The peer IP is in `trusted_proxies` AND the `X-Forwarded-Proto` header
///    equals `"https"` (case-insensitive).
pub fn is_secure_request<B>(req: &http::Request<B>, trusted_proxies: &[IpNet]) -> bool {
    if req.extensions().get::<Protocol>() == Some(&Protocol::Tls) {
        return true;
    }

    let peer_ip = match req.extensions().get::<std::net::SocketAddr>() {
        Some(addr) => addr.ip(),
        None => return false,
    };

    let from_trusted_proxy = trusted_proxies.iter().any(|net| net.contains(&peer_ip));
    if !from_trusted_proxy {
        return false;
    }

    req.headers()
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

/// Middleware that rejects requests that are not secure (HTTPS).
/// Returns 403 Forbidden with a plain-text body.
pub async fn require_https(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    if is_secure_request(&req, &state.trusted_proxies) {
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
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    use http::Request;
    use ipnet::IpNet;

    use crate::extract::Protocol;

    use super::is_secure_request;

    fn build_request() -> http::request::Builder {
        Request::builder().uri("/api/v1/ws/agent")
    }

    #[test]
    fn tls_protocol_extension() {
        let mut req = build_request().body(()).unwrap();
        req.extensions_mut().insert(Protocol::Tls);
        assert!(is_secure_request(&req, &[]));
    }

    #[test]
    fn plain_protocol_extension() {
        let mut req = build_request().body(()).unwrap();
        req.extensions_mut().insert(Protocol::Plain);
        assert!(!is_secure_request(&req, &[]));
    }

    #[test]
    fn no_extension_no_proxy() {
        let mut req = build_request().body(()).unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 12345);
        req.extensions_mut().insert(addr);
        assert!(!is_secure_request(&req, &[]));
    }

    #[test]
    fn trusted_proxy_with_forwarded_https() {
        let proxy_net: IpNet = "192.168.1.0/24".parse().unwrap();
        let mut req = build_request()
            .header("x-forwarded-proto", "https")
            .body(())
            .unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 12345);
        req.extensions_mut().insert(addr);
        assert!(is_secure_request(&req, &[proxy_net]));
    }

    #[test]
    fn trusted_proxy_no_forwarded_header() {
        let proxy_net: IpNet = "192.168.1.0/24".parse().unwrap();
        let mut req = build_request().body(()).unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 12345);
        req.extensions_mut().insert(addr);
        assert!(!is_secure_request(&req, &[proxy_net]));
    }

    #[test]
    fn trusted_proxy_forwarded_http() {
        let proxy_net: IpNet = "192.168.1.0/24".parse().unwrap();
        let mut req = build_request()
            .header("x-forwarded-proto", "http")
            .body(())
            .unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 12345);
        req.extensions_mut().insert(addr);
        assert!(!is_secure_request(&req, &[proxy_net]));
    }

    #[test]
    fn untrusted_ip_with_forwarded_https_is_spoofing() {
        let proxy_net: IpNet = "192.168.1.0/24".parse().unwrap();
        let mut req = build_request()
            .header("x-forwarded-proto", "https")
            .body(())
            .unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50)), 12345);
        req.extensions_mut().insert(addr);
        assert!(!is_secure_request(&req, &[proxy_net]));
    }

    #[test]
    fn cidr_range_matching() {
        let proxy_net: IpNet = "10.0.0.0/8".parse().unwrap();
        let mut req = build_request()
            .header("x-forwarded-proto", "https")
            .body(())
            .unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 255, 255, 1)), 12345);
        req.extensions_mut().insert(addr);
        assert!(is_secure_request(&req, &[proxy_net]));
    }

    #[test]
    fn forwarded_proto_case_insensitive() {
        let proxy_net: IpNet = "192.168.1.0/24".parse().unwrap();
        let mut req = build_request()
            .header("x-forwarded-proto", "HTTPS")
            .body(())
            .unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 12345);
        req.extensions_mut().insert(addr);
        assert!(is_secure_request(&req, &[proxy_net]));
    }

    #[test]
    fn ipv6_trusted_proxy() {
        let proxy_net: IpNet = "::1/128".parse().unwrap();
        let mut req = build_request()
            .header("x-forwarded-proto", "https")
            .body(())
            .unwrap();
        let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 12345);
        req.extensions_mut().insert(addr);
        assert!(is_secure_request(&req, &[proxy_net]));
    }

    #[test]
    fn ipv6_cidr_trusted_proxy() {
        let proxy_net: IpNet = "fd00::/8".parse().unwrap();
        let mut req = build_request()
            .header("x-forwarded-proto", "https")
            .body(())
            .unwrap();
        let addr = SocketAddr::new(
            IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1)),
            12345,
        );
        req.extensions_mut().insert(addr);
        assert!(is_secure_request(&req, &[proxy_net]));
    }
}
