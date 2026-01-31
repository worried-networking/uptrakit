pub mod agent_connections;
pub mod auth;
pub mod cert_signer;
pub mod extract;
pub mod middleware;
pub mod routes;
pub mod settings;
pub mod settings_store;

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware as axum_mw;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use ipnet::IpNet;
use sea_orm::DatabaseConnection;
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use agent_connections::AgentConnectionRegistry;
use auth::jwt::JwtManager;
use auth::oidc_state::{AccountLinkStore, OidcFlowStore, OidcTokenExchangeStore};
use middleware::require_https;
use settings::Settings;

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    /// PEM-encoded CA certificate served at `/api/v1/ca.crt`.
    pub ca_pem: String,
    /// IP networks whose `X-Forwarded-*` headers are trusted.
    pub trusted_proxies: Arc<[IpNet]>,
    /// Header to extract the real client IP from when behind a trusted proxy.
    pub real_ip_header: String,
    /// Database connection pool.
    pub db: DatabaseConnection,
    /// Application settings catalogue.
    pub settings: Settings,
    /// Agent certificate signer for mTLS enrollment.
    pub cert_signer: Arc<dyn cert_signer::AgentCertSigner>,
    /// Registry of connected agents for push notifications.
    pub agent_connections: AgentConnectionRegistry,
    /// Notify channel: fire after any certificate revocation to trigger CRL rebuild.
    pub revocation_notify: Arc<tokio::sync::Notify>,
    /// In-memory store for pending OIDC authorization flows.
    pub oidc_flow_store: OidcFlowStore,
    /// In-memory store for pending OIDC account links.
    pub account_link_store: AccountLinkStore,
    /// JWT signing/validation manager for access tokens.
    pub jwt: Arc<JwtManager>,
    /// In-memory store for pending OIDC token exchanges.
    pub oidc_token_exchange_store: OidcTokenExchangeStore,
}

/// OpenAPI documentation
#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "Authentication", description = "User authentication endpoints"),
        (name = "Settings", description = "Application settings management"),
        (name = "Agents", description = "Agent enrollment and management"),
        (name = "OIDC Providers", description = "OIDC provider configuration")
    ),
    paths(
        routes::auth::register,
        routes::auth::login,
        routes::auth::logout,
        routes::auth::me,
        routes::auth::refresh,
        routes::oidc_auth::auth_methods,
        routes::oidc_auth::oidc_authorize,
        routes::oidc_auth::oidc_callback,
        routes::oidc_auth::oidc_link,
        routes::oidc_auth::oidc_exchange,
        routes::oidc_providers::create_provider,
        routes::oidc_providers::list_providers,
        routes::oidc_providers::get_provider,
        routes::oidc_providers::update_provider,
        routes::oidc_providers::delete_provider,
        routes::oidc_providers::activate_provider,
        routes::oidc_providers::deactivate_provider,
        routes::settings::get_registration_settings,
        routes::settings::update_registration_settings,
        routes::settings_auth::get_authentication_settings,
        routes::settings_auth::update_authentication_settings,
        routes::agents::list_agents,
        routes::agents::approve_agent,
        routes::agents::reject_agent,
        routes::agents::deactivate_agent,
        routes::agents::create_enrollment_token,
        routes::agents::revoke_enrollment_token,
        routes::agents::merge_agent,
        routes::agents::enrollment_token_status,
        routes::settings_agent_certs::get_agent_certificate_settings,
        routes::settings_agent_certs::update_agent_certificate_settings
    ),
    components(
        schemas(
            routes::auth::RegisterRequest,
            routes::auth::LoginRequest,
            routes::auth::LogoutRequest,
            routes::auth::RefreshRequest,
            routes::auth::AuthResponse,
            routes::auth::RefreshResponse,
            routes::auth::UserResponse,
            routes::oidc_auth::AuthMethodsResponse,
            routes::oidc_auth::OidcProviderInfo,
            routes::oidc_auth::OidcAuthorizeResponse,
            routes::oidc_auth::OidcLinkRequest,
            routes::oidc_auth::OidcExchangeRequest,
            routes::oidc_providers::CreateOidcProviderRequest,
            routes::oidc_providers::UpdateOidcProviderRequest,
            routes::oidc_providers::OidcProviderResponse,
            routes::settings::RegistrationSettingsResponse,
            routes::settings::UpdateRegistrationSettingsRequest,
            routes::settings_auth::AuthenticationSettingsResponse,
            routes::settings_auth::UpdateAuthenticationSettingsRequest,
            auth::registration::RegistrationMode,
            routes::agents::AgentStatus,
            routes::agents::AgentResponse,
            routes::agents::EnrollmentTokenResponse,
            routes::agents::MessageResponse,
            routes::agents::MergeAgentRequest,
            routes::agents::EnrollmentTokenStatusResponse,
            routes::settings_agent_certs::AgentCertificateSettingsResponse,
            routes::settings_agent_certs::UpdateAgentCertificateSettingsRequest
        )
    ),
    info(
        title = "Uptrakit API",
        version = "0.0.1",
        description = "Uptrakit update tracking toolkit API"
    ),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_token",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}

/// Content-negotiated 404 handler for unmatched API paths.
pub async fn api_not_found(headers: HeaderMap) -> Response {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let wants_json = accept.contains("application/json") || accept.contains("text/json");

    if wants_json {
        (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"not found"}"#,
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Html(concat!(
                "<!doctype html><html><head><title>404</title>",
                "<style>body{font-family:system-ui,sans-serif;display:flex;",
                "justify-content:center;align-items:center;height:100vh;margin:0;",
                "color:#334155;background:#f8fafc}",
                "h1{font-size:4rem;margin:0}p{color:#64748b}</style></head>",
                "<body><div style=\"text-align:center\">",
                "<h1>404</h1><p>Not Found</p>",
                "</div></body></html>",
            )),
        )
            .into_response()
    }
}

/// Build the application router.
pub fn build_router(state: Arc<AppState>) -> Router {
    // Authenticated OpenAPI routes (require_auth middleware applied before merge)
    let auth_routes = OpenApiRouter::new()
        .routes(routes!(routes::auth::me))
        .routes(routes!(
            routes::settings::get_registration_settings,
            routes::settings::update_registration_settings
        ))
        .routes(routes!(
            routes::settings_auth::get_authentication_settings,
            routes::settings_auth::update_authentication_settings
        ))
        .routes(routes!(
            routes::settings_agent_certs::get_agent_certificate_settings,
            routes::settings_agent_certs::update_agent_certificate_settings
        ))
        .routes(routes!(routes::oidc_providers::create_provider))
        .routes(routes!(routes::oidc_providers::list_providers))
        .routes(routes!(routes::oidc_providers::get_provider))
        .routes(routes!(routes::oidc_providers::update_provider))
        .routes(routes!(routes::oidc_providers::delete_provider))
        .routes(routes!(routes::oidc_providers::activate_provider))
        .routes(routes!(routes::oidc_providers::deactivate_provider))
        .routes(routes!(routes::agents::list_agents))
        .routes(routes!(routes::agents::enrollment_token_status))
        .routes(routes!(
            routes::agents::create_enrollment_token,
            routes::agents::revoke_enrollment_token
        ))
        .routes(routes!(routes::agents::approve_agent))
        .routes(routes!(routes::agents::reject_agent))
        .routes(routes!(routes::agents::deactivate_agent))
        .routes(routes!(routes::agents::merge_agent))
        .route_layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            middleware::require_auth::require_auth,
        ));

    // All OpenAPI routes merged into a single router so the spec is complete
    let (api_router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(routes::auth::register))
        .routes(routes!(routes::auth::login))
        .routes(routes!(routes::auth::logout))
        .routes(routes!(routes::auth::refresh))
        .routes(routes!(routes::oidc_auth::auth_methods))
        .routes(routes!(routes::oidc_auth::oidc_authorize))
        .routes(routes!(routes::oidc_auth::oidc_callback))
        .routes(routes!(routes::oidc_auth::oidc_link))
        .routes(routes!(routes::oidc_auth::oidc_exchange))
        .merge(auth_routes)
        .split_for_parts();

    let mut router = api_router
        .route("/api/v1/ws/agent", get(routes::agent_ws::agent_ws))
        .route_layer(axum_mw::from_fn(require_https::require_https))
        .route("/healthz", get(routes::health::healthz))
        .route("/api/v1/ca.crt", get(routes::ca::ca_cert));

    #[cfg(feature = "swagger-ui")]
    {
        use utoipa_swagger_ui::SwaggerUi;
        router = router.merge(SwaggerUi::new("/api/docs").url("/api/openapi.json", api));
    }

    #[cfg(not(feature = "swagger-ui"))]
    {
        router = router.route("/api/openapi.json", get(|| async move { axum::Json(api) }));
    }

    router
        .layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            middleware::resolve_ip::resolve_ip,
        ))
        .layer(axum_mw::from_fn(middleware::request_log::request_log))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use http::Request;
    use http_body_util::BodyExt;
    use ipnet::IpNet;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use tower::ServiceExt;

    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::cert_signer::{AgentCertBundle, AgentCertSigner};
    use crate::extract::Protocol;
    use crate::settings::Settings;
    use crate::{AppState, build_router};

    struct NoopCertSigner;
    impl AgentCertSigner for NoopCertSigner {
        fn sign_agent_cert(
            &self,
            _: &uuid::Uuid,
            _: time::Duration,
        ) -> Result<AgentCertBundle, String> {
            unimplemented!("not used in tests")
        }
    }

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
            real_ip_header: "X-Forwarded-For".into(),
            db: test_db().await,
            settings: Settings::new(
                RegistrationSettings {
                    mode: RegistrationMode::Open,
                    token_hash: None,
                },
                7,
            ),
            cert_signer: Arc::new(NoopCertSigner),
            agent_connections: crate::agent_connections::AgentConnectionRegistry::new(),
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(),
            account_link_store: crate::auth::oidc_state::AccountLinkStore::new(),
            jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                b"test-secret-lib",
            )),
            oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(),
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
        req.extensions_mut().insert(ConnectInfo(addr));
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
            .header("x-forwarded-for", "203.0.113.45")
            .header("x-forwarded-proto", "https")
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
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

    /// Verify that `into_make_service_with_connect_info` properly injects
    /// `ConnectInfo<SocketAddr>` so the `resolve_ip` middleware can resolve
    /// the client IP — this is the production code path via `axum-server`.
    #[tokio::test]
    async fn make_service_with_connect_info_resolves_client_ip() {
        let router = build_router(test_state().await);
        let mut make_svc = router.into_make_service_with_connect_info::<SocketAddr>();

        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 45)), 12345);
        // Simulate what axum-server does: call the make service with the peer
        // SocketAddr to obtain a per-connection service.
        let svc = <_ as tower::Service<SocketAddr>>::call(&mut make_svc, addr)
            .await
            .unwrap();

        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        // The resolve_ip middleware should have read ConnectInfo<SocketAddr>,
        // created ClientIp, and copied it onto the response extensions.
        let client_ip = resp.extensions().get::<crate::extract::ClientIp>();
        assert!(
            client_ip.is_some(),
            "ClientIp should be present in response extensions"
        );
        assert_eq!(
            client_ip.unwrap().0,
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 45))
        );
    }
}
