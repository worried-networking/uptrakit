pub mod auth;
pub mod cert_signer;
pub mod error_response;
pub mod event_poller;
pub mod extract;
pub mod middleware;
pub mod mqtt_client_store;
pub mod mqtt_lease_coordinator;
pub mod notification_service;
pub mod ocsp;
pub mod pki_utils;
pub mod routes;
pub mod service_connections;
pub mod setting_key;
pub mod settings;
pub mod settings_store;
pub mod update_hooks;

pub use setting_key::SettingKey;

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware as axum_mw;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use sea_orm::DatabaseConnection;
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use auth::device_flow::DeviceFlowStore;
use auth::jwt::JwtManager;
#[cfg(feature = "oidc")]
use auth::oidc_state::{
    AccountLinkStore, OidcFlowStore, OidcRegistrationStore, OidcTokenExchangeStore,
};
use auth::rate_limit::RateLimitStore;
use notification_service::NotificationService;
use service_connections::ServiceConnectionRegistry;
use settings::Settings;

/// Public CA snapshot data and key store types. Re-exported for use by consumers.
pub use ca_snapshot::{CaKeyStoreRef, CaSnapshotReceiver};

pub mod ca_snapshot {
    use std::fmt;
    use std::sync::Arc;

    use zeroize::Zeroizing;

    /// Type alias for the watch receiver carrying public CA snapshot data.
    pub type CaSnapshotReceiver = tokio::sync::watch::Receiver<CaPublicSnapshot>;

    /// Public-facing CA material — safe to clone, debug, and distribute to all handlers.
    #[derive(Clone, Debug)]
    pub struct CaPublicSnapshot {
        pub active_cert_pem: String,
        pub active_fingerprint: String,
        pub previous_cert_pem: Option<String>,
        pub previous_fingerprint: Option<String>,
        pub trusted_cas: Vec<TrustedCaPublic>,
        pub trusted_ca_cns: Vec<String>,
        pub bundle_pem: String,
        pub bundle_hash: String,
        pub managed: bool,
        pub active_not_after: time::OffsetDateTime,
        pub pki_addr: Option<String>,
    }

    /// Public part of a trusted CA (no private key).
    #[derive(Clone, Debug)]
    pub struct TrustedCaPublic {
        pub cert_pem: String,
        pub fingerprint: String,
        pub not_after: time::OffsetDateTime,
    }

    /// Private key material for a single trusted CA.
    pub struct TrustedCaKey {
        pub fingerprint: String,
        pub key_pem: Zeroizing<String>,
    }

    /// Private key store — NOT Clone, NOT Debug. Only accessible by OCSP, CRL, and cert signers.
    pub struct CaKeyStore {
        pub active_key_pem: Zeroizing<String>,
        pub previous_key_pem: Option<Zeroizing<String>>,
        pub trusted_ca_keys: Vec<TrustedCaKey>,
    }

    impl fmt::Debug for CaKeyStore {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CaKeyStore")
                .field("active_key_pem", &"[REDACTED]")
                .field(
                    "previous_key_pem",
                    &self.previous_key_pem.as_ref().map(|_| "[REDACTED]"),
                )
                .field(
                    "trusted_ca_keys",
                    &format!("[{} keys REDACTED]", self.trusted_ca_keys.len()),
                )
                .finish()
        }
    }

    /// Type alias for the shared key store.
    pub type CaKeyStoreRef = Arc<tokio::sync::RwLock<CaKeyStore>>;

    /// Input parameters for building both a public snapshot and a key store.
    pub struct SplitSnapshotInput {
        pub active_cert_pem: String,
        pub active_key_pem: String,
        pub active_fingerprint: String,
        pub previous_cert_pem: Option<String>,
        pub previous_key_pem: Option<String>,
        pub previous_fingerprint: Option<String>,
        pub trusted_cas_public: Vec<TrustedCaPublic>,
        pub trusted_ca_keys: Vec<TrustedCaKey>,
        pub trusted_ca_cns: Vec<String>,
        pub bundle_pem: String,
        pub bundle_hash: String,
        pub managed: bool,
        pub active_not_after: time::OffsetDateTime,
        pub pki_addr: Option<String>,
    }

    /// Build both a public snapshot and a key store from component parts.
    pub fn split_snapshot(input: SplitSnapshotInput) -> (CaPublicSnapshot, CaKeyStore) {
        let public = CaPublicSnapshot {
            active_cert_pem: input.active_cert_pem,
            active_fingerprint: input.active_fingerprint,
            previous_cert_pem: input.previous_cert_pem,
            previous_fingerprint: input.previous_fingerprint,
            trusted_cas: input.trusted_cas_public,
            trusted_ca_cns: input.trusted_ca_cns,
            bundle_pem: input.bundle_pem,
            bundle_hash: input.bundle_hash,
            managed: input.managed,
            active_not_after: input.active_not_after,
            pki_addr: input.pki_addr,
        };
        let keys = CaKeyStore {
            active_key_pem: Zeroizing::new(input.active_key_pem),
            previous_key_pem: input.previous_key_pem.map(Zeroizing::new),
            trusted_ca_keys: input.trusted_ca_keys,
        };
        (public, keys)
    }
}

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    /// Watch receiver for the current CA snapshot (bundle PEM, fingerprints, etc.).
    pub ca_snapshot: CaSnapshotReceiver,
    /// Private CA key store — only for OCSP, CRL, and cert signing operations.
    pub ca_key_store: CaKeyStoreRef,
    /// Database connection pool.
    pub db: DatabaseConnection,
    /// Application settings catalogue (includes network settings).
    pub settings: Settings,
    /// Agent certificate signer for mTLS enrollment.
    pub cert_signer: Arc<dyn cert_signer::AgentCertSigner>,
    /// Unified registry of connected services (agents and MQTT) for push notifications.
    pub service_connections: ServiceConnectionRegistry,
    /// Notify channel: fire after any certificate revocation to trigger CRL rebuild.
    pub revocation_notify: Arc<tokio::sync::Notify>,
    /// Database-backed store for pending OIDC authorization flows.
    #[cfg(feature = "oidc")]
    pub oidc_flow_store: OidcFlowStore,
    /// Database-backed store for pending OIDC account links.
    #[cfg(feature = "oidc")]
    pub account_link_store: AccountLinkStore,
    /// JWT signing/validation manager for access tokens.
    pub jwt: Arc<JwtManager>,
    /// Database-backed store for pending OIDC token exchanges.
    #[cfg(feature = "oidc")]
    pub oidc_token_exchange_store: OidcTokenExchangeStore,
    /// Database-backed store for pending OIDC registrations (token-gated).
    #[cfg(feature = "oidc")]
    pub oidc_registration_store: OidcRegistrationStore,
    /// Database-backed store for pending device authorization flows.
    pub device_flow_store: DeviceFlowStore,
    /// Database-backed rate limiter for public authentication endpoints.
    pub rate_limit_store: RateLimitStore,
    /// Path to the PKI directory (for server cert renewal).
    pub pki_path: std::path::PathBuf,
    /// RustlsConfig handle for hot-reloading TLS.
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    /// Cached PEM-encoded CRL bundle, updated by the CRL manager.
    pub crl_pem_cache: Arc<tokio::sync::RwLock<String>>,
    /// Trigger for immediate CA rotation (fired by the rotate-ca API endpoint).
    pub ca_rotation_trigger: Arc<tokio::sync::Notify>,
    /// UUID of the default (seeded) tenant. Used as fallback when no tenant header is present.
    pub default_tenant_id: uuid::Uuid,
    /// Unique identifier for this controller instance (used for cross-controller notification delivery).
    pub controller_id: uuid::Uuid,
    /// Cross-controller notification service for push message delivery via outbox pattern.
    pub notification_service: NotificationService,
    /// In-memory denylist for immediate JWT access token revocation.
    pub token_denylist: Arc<auth::token_denylist::TokenDenylist>,
}

/// OpenAPI documentation (core — always available)
#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "Authentication", description = "User authentication endpoints"),
        (name = "Settings", description = "Application settings management"),
        (name = "Services", description = "Unified service (agent and MQTT) enrollment and management"),
        (name = "OIDC Providers", description = "OIDC provider configuration"),
        (name = "API Tokens", description = "Personal access token management"),
        (name = "Hosts", description = "Host machine management"),
        (name = "Provider Configs", description = "Provider configuration management"),
        (name = "Software Items", description = "Software item tracking and host assignment"),
        (name = "Update History", description = "Software update history tracking")
    ),
    paths(
        routes::auth::register,
        routes::auth::login,
        routes::auth::logout,
        routes::auth::me,
        routes::auth::refresh,
        routes::settings::get_registration_settings,
        routes::settings::update_registration_settings,
        routes::settings_combined::get_combined_settings,
        routes::settings_auth::get_authentication_settings,
        routes::settings_auth::update_authentication_settings,
        routes::services::list_services,
        routes::services::get_service,
        routes::services::update_service,
        routes::services::approve_service,
        routes::services::reject_service,
        routes::services::deactivate_service,
        routes::services::merge_service,
        routes::services::create_enrollment_token,
        routes::services::revoke_enrollment_token,
        routes::services::enrollment_token_status,
        routes::settings_agent_certs::get_agent_certificate_settings,
        routes::settings_agent_certs::update_agent_certificate_settings,
        routes::system_alerts::get_system_alerts,
        routes::server_cert::renew_server_certificate,
        routes::api_tokens::create_api_token,
        routes::api_tokens::list_api_tokens,
        routes::api_tokens::revoke_api_token,
        routes::device_auth::device_auth_start,
        routes::device_auth::device_auth_poll,
        routes::device_auth::device_auth_approve,
        routes::settings_network::get_network_settings,
        routes::settings_network::update_network_settings,
        routes::settings_mqtt::list_mqtt_settings,
        routes::settings_mqtt::create_mqtt_settings,
        routes::settings_mqtt::get_mqtt_limit,
        routes::settings_mqtt::update_mqtt_limit,
        routes::settings_mqtt::get_mqtt_settings,
        routes::settings_mqtt::update_mqtt_settings,
        routes::settings_mqtt::delete_mqtt_settings,
        routes::hosts::list_hosts,
        routes::hosts::get_host,
        routes::hosts::update_host,
        routes::hosts::deactivate_host,
        routes::provider_configs::create_provider_config,
        routes::provider_configs::list_provider_configs,
        routes::provider_configs::get_provider_config,
        routes::provider_configs::update_provider_config,
        routes::provider_configs::delete_provider_config,
        routes::software_items::create_software_item,
        routes::software_items::list_software_items,
        routes::software_items::get_software_item,
        routes::software_items::update_software_item,
        routes::software_items::delete_software_item,
        routes::software_items::assign_hosts,
        routes::software_items::unassign_host,
        routes::software_items::trigger_update,
        routes::software_items::check_versions,
        routes::software_items::check_versions_host,
        routes::settings_ca::rotate_ca,
        routes::scheduler::list_scheduled_tasks,
        routes::scheduler::get_scheduled_task,
        routes::scheduler::update_scheduled_task,
        routes::scheduler::trigger_scheduled_task,
        routes::update_history::list_update_history,
        routes::update_history::get_update_history,
    ),
    components(
        schemas(
            uptrakit_web_api_types::error::ErrorResponse,
            routes::auth::RegisterRequest,
            routes::auth::LoginRequest,
            routes::auth::LogoutRequest,
            routes::auth::RefreshRequest,
            routes::auth::AuthResponse,
            routes::auth::RefreshResponse,
            routes::auth::UserResponse,
            routes::settings::RegistrationSettingsResponse,
            routes::settings::UpdateRegistrationSettingsRequest,
            routes::settings_auth::AuthenticationSettingsResponse,
            routes::settings_auth::UpdateAuthenticationSettingsRequest,
            uptrakit_web_api_types::settings_combined::CombinedSettingsResponse,
            uptrakit_web_api_types::settings_combined::EnrollmentTokenStatusesResponse,
            auth::registration::RegistrationMode,
            routes::services::ServiceType,
            routes::services::ServiceStatus,
            routes::services::ServiceResponse,
            routes::services::UpdateServiceRequest,
            routes::services::EnrollmentTokenResponse,
            routes::services::MessageResponse,
            routes::services::MergeAgentRequest,
            routes::services::EnrollmentTokenStatusResponse,
            routes::settings_agent_certs::AgentCertificateSettingsResponse,
            routes::settings_agent_certs::UpdateAgentCertificateSettingsRequest,
            routes::system_alerts::SystemAlert,
            routes::system_alerts::SystemAlertsResponse,
            routes::api_tokens::CreateApiTokenRequest,
            routes::api_tokens::CreateApiTokenResponse,
            routes::api_tokens::ApiTokenResponse,
            routes::api_tokens::ApiTokenListResponse,
            routes::device_auth::DeviceAuthStartRequest,
            routes::device_auth::DeviceAuthStartResponse,
            routes::device_auth::DeviceAuthPollRequest,
            routes::device_auth::DeviceAuthPollResponse,
            routes::device_auth::DeviceAuthApproveRequest,
            routes::device_auth::DeviceAuthApproveResponse,
            routes::settings_network::NetworkSettingsResponse,
            routes::settings_network::UpdateNetworkSettingsRequest,
            routes::settings_mqtt::MqttClientResponse,
            routes::settings_mqtt::MqttLimitResponse,
            routes::settings_mqtt::CreateMqttClientRequest,
            routes::settings_mqtt::UpdateMqttClientRequest,
            routes::settings_mqtt::UpdateMqttLimitRequest,
            uptrakit_web_api_types::mqtt_transport::MqttTransport,
            routes::hosts::HostResponse,
            routes::hosts::HostAgentSummary,
            routes::hosts::UpdateHostRequest,
            routes::provider_configs::CreateProviderConfigRequest,
            routes::provider_configs::UpdateProviderConfigRequest,
            routes::provider_configs::ProviderConfigResponse,
            routes::software_items::CreateSoftwareItemRequest,
            routes::software_items::UpdateSoftwareItemRequest,
            routes::software_items::AssignHostsRequest,
            routes::software_items::SoftwareItemResponse,
            routes::software_items::SoftwareItemDetailResponse,
            routes::software_items::SoftwareItemHostSummary,
            routes::software_items::TriggerUpdateRequest,
            routes::software_items::TriggerUpdateResponse,
            routes::software_items::TriggerUpdateStatus,
            routes::software_items::TriggerVersionCheckResponse,
            routes::scheduler::ScheduledTaskResponse,
            routes::scheduler::UpdateScheduledTaskRequest,
            routes::scheduler::TriggerScheduledTaskResponse,
            routes::update_history::UpdateHistoryResponse,
            routes::update_history::UpdateStatus,
            uptrakit_web_api_types::pagination::PaginatedResponse<routes::services::ServiceResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<routes::hosts::HostResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<routes::software_items::SoftwareItemResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<routes::update_history::UpdateHistoryResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<routes::provider_configs::ProviderConfigResponse>,
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

/// OIDC-specific OpenAPI paths and schemas, merged conditionally.
#[cfg(feature = "oidc")]
#[derive(OpenApi)]
#[openapi(
    paths(
        routes::oidc_auth::auth_methods,
        routes::oidc_auth::oidc_authorize,
        routes::oidc_auth::oidc_callback,
        routes::oidc_auth::oidc_link,
        routes::oidc_auth::oidc_exchange,
        routes::oidc_auth::oidc_complete_registration,
        routes::oidc_providers::create_provider,
        routes::oidc_providers::list_providers,
        routes::oidc_providers::get_provider,
        routes::oidc_providers::update_provider,
        routes::oidc_providers::delete_provider,
        routes::oidc_providers::activate_provider,
        routes::oidc_providers::deactivate_provider,
    ),
    components(schemas(
        routes::oidc_auth::AuthMethodsResponse,
        routes::oidc_auth::OidcProviderInfo,
        routes::oidc_auth::OidcAuthorizeResponse,
        routes::oidc_auth::OidcLinkRequest,
        routes::oidc_auth::OidcExchangeRequest,
        routes::oidc_auth::OidcCompleteRegistrationRequest,
        routes::oidc_providers::CreateOidcProviderRequest,
        routes::oidc_providers::UpdateOidcProviderRequest,
        routes::oidc_providers::OidcProviderResponse,
    ))
)]
struct OidcApiDoc;

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
        error_response::error_response_with_code(StatusCode::NOT_FOUND, "Not found", "not_found")
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
        .routes(routes!(routes::auth::logout))
        .routes(routes!(routes::auth::me))
        .routes(routes!(
            routes::api_tokens::create_api_token,
            routes::api_tokens::list_api_tokens
        ))
        .routes(routes!(routes::api_tokens::revoke_api_token))
        .routes(routes!(
            routes::settings::get_registration_settings,
            routes::settings::update_registration_settings
        ))
        .routes(routes!(routes::settings_combined::get_combined_settings))
        .routes(routes!(
            routes::settings_auth::get_authentication_settings,
            routes::settings_auth::update_authentication_settings
        ))
        .routes(routes!(
            routes::settings_agent_certs::get_agent_certificate_settings,
            routes::settings_agent_certs::update_agent_certificate_settings
        ))
        .routes(routes!(routes::services::list_services))
        .routes(routes!(routes::services::enrollment_token_status))
        .routes(routes!(
            routes::services::create_enrollment_token,
            routes::services::revoke_enrollment_token
        ))
        .routes(routes!(routes::services::approve_service))
        .routes(routes!(routes::services::reject_service))
        .routes(routes!(
            routes::services::get_service,
            routes::services::update_service,
            routes::services::deactivate_service
        ))
        .routes(routes!(routes::services::merge_service))
        .routes(routes!(routes::system_alerts::get_system_alerts))
        .routes(routes!(routes::server_cert::renew_server_certificate))
        .routes(routes!(
            routes::settings_network::get_network_settings,
            routes::settings_network::update_network_settings
        ))
        .routes(routes!(
            routes::settings_mqtt::list_mqtt_settings,
            routes::settings_mqtt::create_mqtt_settings
        ))
        .routes(routes!(
            routes::settings_mqtt::get_mqtt_limit,
            routes::settings_mqtt::update_mqtt_limit
        ))
        .routes(routes!(
            routes::settings_mqtt::get_mqtt_settings,
            routes::settings_mqtt::update_mqtt_settings,
            routes::settings_mqtt::delete_mqtt_settings
        ))
        .routes(routes!(routes::device_auth::device_auth_approve))
        .routes(routes!(routes::settings_ca::rotate_ca))
        .routes(routes!(routes::hosts::list_hosts))
        .routes(routes!(routes::hosts::get_host))
        .routes(routes!(routes::hosts::update_host))
        .routes(routes!(routes::hosts::deactivate_host))
        .routes(routes!(
            routes::provider_configs::create_provider_config,
            routes::provider_configs::list_provider_configs
        ))
        .routes(routes!(routes::provider_configs::get_provider_config))
        .routes(routes!(routes::provider_configs::update_provider_config))
        .routes(routes!(routes::provider_configs::delete_provider_config))
        .routes(routes!(
            routes::software_items::create_software_item,
            routes::software_items::list_software_items
        ))
        .routes(routes!(routes::software_items::get_software_item))
        .routes(routes!(routes::software_items::update_software_item))
        .routes(routes!(routes::software_items::delete_software_item))
        .routes(routes!(routes::software_items::assign_hosts))
        .routes(routes!(routes::software_items::unassign_host))
        .routes(routes!(routes::software_items::trigger_update))
        .routes(routes!(routes::software_items::check_versions))
        .routes(routes!(routes::software_items::check_versions_host))
        .routes(routes!(routes::scheduler::list_scheduled_tasks))
        .routes(routes!(routes::scheduler::get_scheduled_task))
        .routes(routes!(routes::scheduler::update_scheduled_task))
        .routes(routes!(routes::scheduler::trigger_scheduled_task))
        .routes(routes!(routes::update_history::list_update_history))
        .routes(routes!(routes::update_history::get_update_history))
        .route_layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            middleware::require_auth::require_auth,
        ));

    // All OpenAPI routes merged into a single router so the spec is complete
    let openapi = ApiDoc::openapi();

    #[cfg(feature = "oidc")]
    let openapi = {
        let mut openapi = openapi;
        openapi.merge(OidcApiDoc::openapi());
        openapi
    };

    let base_router = OpenApiRouter::with_openapi(openapi)
        .routes(routes!(routes::auth::register))
        .routes(routes!(routes::auth::login))
        .routes(routes!(routes::auth::refresh))
        .routes(routes!(routes::device_auth::device_auth_start))
        .routes(routes!(routes::device_auth::device_auth_poll))
        .merge(auth_routes);

    #[cfg(feature = "oidc")]
    let base_router = base_router
        .routes(routes!(routes::oidc_auth::auth_methods))
        .routes(routes!(routes::oidc_auth::oidc_authorize))
        .routes(routes!(routes::oidc_auth::oidc_callback))
        .routes(routes!(routes::oidc_auth::oidc_link))
        .routes(routes!(routes::oidc_auth::oidc_exchange))
        .routes(routes!(routes::oidc_auth::oidc_complete_registration))
        .routes(routes!(routes::oidc_providers::create_provider))
        .routes(routes!(routes::oidc_providers::list_providers))
        .routes(routes!(routes::oidc_providers::get_provider))
        .routes(routes!(routes::oidc_providers::update_provider))
        .routes(routes!(routes::oidc_providers::delete_provider))
        .routes(routes!(routes::oidc_providers::activate_provider))
        .routes(routes!(routes::oidc_providers::deactivate_provider));

    let (api_router, api) = base_router.split_for_parts();

    let mut router = api_router
        .route("/api/v1/ws/service", get(routes::service_ws::service_ws))
        .route("/healthz", get(routes::health::healthz))
        .route("/api/v1/pki/ca.crt", get(routes::ca::ca_cert))
        .route("/api/v1/pki/ca.crl", get(routes::ca::ca_crl))
        .route(
            "/api/v1/pki/ocsp",
            axum::routing::post(routes::ocsp::ocsp_post),
        )
        .route("/api/v1/pki/ocsp/{encoded}", get(routes::ocsp::ocsp_get));

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
            middleware::resolve_proxy_headers::resolve_proxy_headers,
        ))
        .layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            middleware::rate_limit::rate_limit_auth,
        ))
        .layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            middleware::resolve_ip::resolve_ip,
        ))
        .layer(axum_mw::from_fn(middleware::request_log::request_log))
        .with_state(state)
}

/// Build a minimal router serving only PKI endpoints over plain HTTP.
///
/// Used by `--pki-http listener` to expose OCSP, CRL, and CA cert endpoints
/// without TLS (required by Nginx `ssl_ocsp_responder` which only supports http://).
///
/// Applies the same IP-resolution and request-logging middleware as the main
/// router so that client/proxy IPs are properly detected and every request is
/// logged. The `resolve_proxy_headers` layer is intentionally omitted because
/// PKI endpoints do not need agent certificate identity or external base URL
/// resolution.
pub fn build_pki_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(routes::health::healthz))
        .route("/api/v1/pki/ca.crt", get(routes::ca::ca_cert))
        .route("/api/v1/pki/ca.crl", get(routes::ca::ca_crl))
        .route(
            "/api/v1/pki/ocsp",
            axum::routing::post(routes::ocsp::ocsp_post),
        )
        .route("/api/v1/pki/ocsp/{encoded}", get(routes::ocsp::ocsp_get))
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
    use http::Request;
    use http_body_util::BodyExt;
    use ipnet::IpNet;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use tower::ServiceExt;

    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
    use crate::settings::Settings;
    use crate::{AppState, build_pki_router, build_router};

    struct NoopCertSigner;
    #[async_trait::async_trait]
    impl AgentCertSigner for NoopCertSigner {
        async fn sign_agent_csr(
            &self,
            _: &str,
            _: &uuid::Uuid,
            _: time::Duration,
        ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>> {
            Err(rootcause::Report::new(CertSignerError::Signing(
                "noop signer".to_string(),
            )))
        }

        fn active_ca_fingerprint(&self) -> String {
            "0000000000000000000000000000000000000000000000000000000000000000".to_string()
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
        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![crate::ca_snapshot::TrustedCaPublic {
                cert_pem: ca_pem.to_string(),
                fingerprint: "0".repeat(64),
                not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            }],
            trusted_ca_cns: Vec::new(),
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
        let ca_key_store: crate::CaKeyStoreRef =
            Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
                active_key_pem: zeroize::Zeroizing::new(String::new()),
                previous_key_pem: None,
                trusted_ca_keys: vec![],
            }));

        // Create a dummy RustlsConfig — tests don't actually do TLS handshakes.
        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .unwrap()
                .self_signed(&key_pair)
                .unwrap();
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap(),
                )
                .unwrap();
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let db = test_db().await;

        let settings = Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            7,
        );
        if !trusted_proxies.is_empty() {
            settings.set_trusted_proxies(trusted_proxies).await;
        }

        let service_connections = crate::service_connections::ServiceConnectionRegistry::new();
        let controller_id = uuid::Uuid::nil();
        let notification_service = crate::notification_service::NotificationService::new(
            db.clone(),
            service_connections.clone(),
            controller_id,
        );

        Arc::new(AppState {
            ca_snapshot: ca_rx,
            ca_key_store,
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections,
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            #[cfg(feature = "oidc")]
            oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
            #[cfg(feature = "oidc")]
            account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
            jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                b"test-secret-lib",
            )),
            #[cfg(feature = "oidc")]
            oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                db.clone(),
            ),
            #[cfg(feature = "oidc")]
            oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                db.clone(),
            ),
            device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
            rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
            ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            default_tenant_id: uuid::Uuid::nil(),
            controller_id,
            notification_service,
            token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            db,
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
    async fn ca_cert_returns_pem() {
        let app = build_router(test_state().await);
        let req = Request::builder()
            .uri("/api/v1/pki/ca.crt")
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

    /// Verify the PKI router resolves client IPs the same way the main router
    /// does — through `resolve_ip` middleware and `ConnectInfo<SocketAddr>`.
    #[tokio::test]
    async fn pki_router_resolves_client_ip() {
        let router = build_pki_router(test_state().await);
        let mut make_svc = router.into_make_service_with_connect_info::<SocketAddr>();

        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 54321);
        let svc = <_ as tower::Service<SocketAddr>>::call(&mut make_svc, addr)
            .await
            .unwrap();

        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let client_ip = resp.extensions().get::<crate::extract::ClientIp>();
        assert!(
            client_ip.is_some(),
            "ClientIp should be present in PKI router response extensions"
        );
        assert_eq!(
            client_ip.unwrap().0,
            IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))
        );
    }

    /// Verify the PKI router honours trusted proxies when resolving client IPs.
    #[tokio::test]
    async fn pki_router_resolves_proxy_ip() {
        let proxy_net: IpNet = "10.0.0.0/8".parse().unwrap();
        let state = test_state_with_proxies(vec![proxy_net]).await;
        let router = build_pki_router(state);
        let mut make_svc = router.into_make_service_with_connect_info::<SocketAddr>();

        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 54321);
        let svc = <_ as tower::Service<SocketAddr>>::call(&mut make_svc, addr)
            .await
            .unwrap();

        let req = Request::builder()
            .uri("/api/v1/pki/ca.crt")
            .header("x-forwarded-for", "203.0.113.45, 10.0.0.1")
            .body(Body::empty())
            .unwrap();
        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let proxy_ip = resp.extensions().get::<crate::extract::ProxyIp>();
        assert!(
            proxy_ip.is_some(),
            "ProxyIp should be present when request comes from a trusted proxy"
        );
        assert_eq!(proxy_ip.unwrap().0, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn ca_key_store_debug_redacts_keys() {
        let store = crate::ca_snapshot::CaKeyStore {
            active_key_pem: zeroize::Zeroizing::new(
                "-----BEGIN PRIVATE KEY-----\ntest\n-----END PRIVATE KEY-----\n".to_string(),
            ),
            previous_key_pem: Some(zeroize::Zeroizing::new(
                "-----BEGIN PRIVATE KEY-----\nold\n-----END PRIVATE KEY-----\n".to_string(),
            )),
            trusted_ca_keys: vec![],
        };
        let debug_output = format!("{store:?}");
        assert!(
            !debug_output.contains("BEGIN"),
            "Debug output must not contain PEM markers"
        );
        assert!(
            debug_output.contains("REDACTED"),
            "Debug output must contain REDACTED"
        );
    }

    #[test]
    fn ca_public_snapshot_has_no_key_fields() {
        let snapshot = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: String::new(),
            active_fingerprint: String::new(),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![],
            trusted_ca_cns: vec![],
            bundle_pem: String::new(),
            bundle_hash: String::new(),
            managed: false,
            active_not_after: time::OffsetDateTime::now_utc(),
            pki_addr: None,
        };
        let debug_output = format!("{snapshot:?}");
        assert!(
            !debug_output.contains("key_pem"),
            "CaPublicSnapshot must not expose key material"
        );
    }
}
