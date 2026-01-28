pub mod extract;
pub mod middleware;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use axum::middleware as axum_mw;
use axum::routing::get;
use ipnet::IpNet;
use sea_orm::DatabaseConnection;

use middleware::require_https;

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    /// PEM-encoded CA certificate served at `/api/v1/ca.crt`.
    pub ca_pem: String,
    /// IP networks whose `X-Forwarded-*` headers are trusted.
    pub trusted_proxies: Arc<[IpNet]>,
    /// Database connection pool.
    pub db: DatabaseConnection,
}

/// Build the application router.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/ws/agent", get(routes::agent_ws::agent_ws))
        .route_layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            require_https::require_https,
        ))
        .route("/healthz", get(routes::health::healthz))
        .route("/api/v1/ca.crt", get(routes::ca::ca_cert))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    use axum::body::Body;
    use http::Request;
    use http_body_util::BodyExt;
    use ipnet::IpNet;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use tower::ServiceExt;

    use crate::extract::Protocol;
    use crate::{AppState, build_router};

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn test_state() -> Arc<AppState> {
        test_state_with_proxies(vec![]).await
    }

    async fn test_state_with_proxies(trusted_proxies: Vec<IpNet>) -> Arc<AppState> {
        Arc::new(AppState {
            ca_pem: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n".into(),
            trusted_proxies: trusted_proxies.into(),
            db: test_db().await,
        })
    }

    #[tokio::test]
    async fn healthz_returns_ok() {
        let app = build_router(test_state().await);
        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn agent_ws_without_https_returns_403() {
        let app = build_router(test_state().await);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 12345);
        let mut req = Request::builder()
            .uri("/api/v1/ws/agent")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(addr);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 403);
    }

    #[tokio::test]
    async fn agent_ws_with_tls_protocol_not_403() {
        let app = build_router(test_state().await);
        let mut req = Request::builder()
            .uri("/api/v1/ws/agent")
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Protocol::Tls);
        let resp = app.oneshot(req).await.unwrap();
        // Should be 101 Switching Protocols (or at least not 403)
        assert_ne!(resp.status(), 403);
    }

    #[tokio::test]
    async fn agent_ws_via_trusted_proxy_not_403() {
        let proxy_net: IpNet = "192.168.1.0/24".parse().unwrap();
        let app = build_router(test_state_with_proxies(vec![proxy_net]).await);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 12345);
        let mut req = Request::builder()
            .uri("/api/v1/ws/agent")
            .header("x-forwarded-proto", "https")
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(addr);
        let resp = app.oneshot(req).await.unwrap();
        assert_ne!(resp.status(), 403);
    }

    #[tokio::test]
    async fn ca_cert_returns_pem() {
        let app = build_router(test_state().await);
        let req = Request::builder()
            .uri("/api/v1/ca.crt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/x-pem-file"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.starts_with(b"-----BEGIN CERTIFICATE-----"));
    }

    #[tokio::test]
    async fn unknown_path_returns_404_not_https_error() {
        let app = build_router(test_state().await);

        // Test root path
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        // Should return 404 Not Found, not 403 Forbidden
        assert_eq!(resp.status(), 404);

        // Test another unknown path
        let req = Request::builder()
            .uri("/unknown/path")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 404);
    }
}
