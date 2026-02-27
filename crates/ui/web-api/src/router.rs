use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware as axum_mw;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;

/// OpenAPI documentation (core — always available)
#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "Authentication", description = "User authentication endpoints"),
        (name = "Settings", description = "Application settings management"),
        (name = "Services", description = "Unified service (agent and MQTT) enrollment and management"),
        (name = "Enrollment Tokens", description = "Enrollment token management"),
        (name = "OIDC Providers", description = "OIDC provider configuration"),
        (name = "API Tokens", description = "Personal access token management"),
        (name = "Hosts", description = "Host machine management"),
        (name = "Plugin Configs", description = "Plugin configuration management"),
        (name = "Software Items", description = "Software item tracking and host assignment"),
        (name = "Update History", description = "Software update history tracking"),
        (name = "Autodiscovery", description = "Automatic software discovery management")
    ),
    paths(
        crate::routes::auth::register,
        crate::routes::auth::login,
        crate::routes::auth::logout,
        crate::routes::auth::me,
        crate::routes::auth::refresh,
        crate::routes::settings::get_registration_settings,
        crate::routes::settings::update_registration_settings,
        crate::routes::settings_combined::get_combined_settings,
        crate::routes::settings_auth::get_authentication_settings,
        crate::routes::settings_auth::update_authentication_settings,
        crate::routes::services::list_services,
        crate::routes::services::get_service,
        crate::routes::services::update_service,
        crate::routes::services::approve_service,
        crate::routes::services::reject_service,
        crate::routes::services::deactivate_service,
        crate::routes::services::merge_service,
        crate::routes::enrollment_tokens::create_enrollment_token,
        crate::routes::enrollment_tokens::list_enrollment_tokens,
        crate::routes::enrollment_tokens::get_enrollment_token,
        crate::routes::enrollment_tokens::revoke_enrollment_token,
        crate::routes::settings_agent_certs::get_agent_certificate_settings,
        crate::routes::settings_agent_certs::update_agent_certificate_settings,
        crate::routes::system_alerts::get_system_alerts,
        crate::routes::server_cert::renew_server_certificate,
        crate::routes::api_tokens::create_api_token,
        crate::routes::api_tokens::list_api_tokens,
        crate::routes::api_tokens::revoke_api_token,
        crate::routes::device_auth::device_auth_start,
        crate::routes::device_auth::device_auth_poll,
        crate::routes::device_auth::device_auth_approve,
        crate::routes::settings_network::get_network_settings,
        crate::routes::settings_network::update_network_settings,
        crate::routes::settings_mqtt::list_mqtt_settings,
        crate::routes::settings_mqtt::create_mqtt_settings,
        crate::routes::settings_mqtt::get_mqtt_limit,
        crate::routes::settings_mqtt::update_mqtt_limit,
        crate::routes::settings_mqtt::get_mqtt_settings,
        crate::routes::settings_mqtt::update_mqtt_settings,
        crate::routes::settings_mqtt::delete_mqtt_settings,
        crate::routes::hosts::list_hosts,
        crate::routes::hosts::get_host,
        crate::routes::hosts::update_host,
        crate::routes::hosts::deactivate_host,
        crate::routes::plugin_configs::create_plugin_config,
        crate::routes::plugin_configs::list_plugin_configs,
        crate::routes::plugin_configs::get_plugin_config,
        crate::routes::plugin_configs::update_plugin_config,
        crate::routes::plugin_configs::delete_plugin_config,
        crate::routes::software_items::create_software_item,
        crate::routes::software_items::list_software_items,
        crate::routes::software_items::get_software_item,
        crate::routes::software_items::update_software_item,
        crate::routes::software_items::delete_software_item,
        crate::routes::software_items::assign_hosts,
        crate::routes::software_items::unassign_host,
        crate::routes::software_items::update_host_assignment,
        crate::routes::software_items::trigger_update,
        crate::routes::software_items::check_versions,
        crate::routes::software_items::check_versions_host,
        crate::routes::software_items::approve_software_item,
        crate::routes::hosts::discover_host,
        crate::routes::hosts::discard_host_discovered,
        crate::routes::plugin_configs::discover_plugin_config,
        crate::routes::plugin_configs::discard_plugin_config_discovered,
        crate::routes::autodiscovery::list_autodiscovery_ignores,
        crate::routes::autodiscovery::create_autodiscovery_ignore,
        crate::routes::autodiscovery::delete_autodiscovery_ignore,
        crate::routes::settings_ca::rotate_ca,
        crate::routes::scheduler::list_scheduled_tasks,
        crate::routes::scheduler::get_scheduled_task,
        crate::routes::scheduler::update_scheduled_task,
        crate::routes::scheduler::trigger_scheduled_task,
        crate::routes::update_history::list_update_history,
        crate::routes::update_history::get_update_history,
        crate::routes::update_history::stream_update_output,
    ),
    components(
        schemas(
            uptrakit_web_api_types::error::ErrorResponse,
            crate::routes::auth::RegisterRequest,
            crate::routes::auth::LoginRequest,
            crate::routes::auth::LogoutRequest,
            crate::routes::auth::RefreshRequest,
            crate::routes::auth::AuthResponse,
            crate::routes::auth::RefreshResponse,
            crate::routes::auth::UserResponse,
            crate::routes::settings::RegistrationSettingsResponse,
            crate::routes::settings::UpdateRegistrationSettingsRequest,
            crate::routes::settings_auth::AuthenticationSettingsResponse,
            crate::routes::settings_auth::UpdateAuthenticationSettingsRequest,
            uptrakit_web_api_types::settings_combined::CombinedSettingsResponse,
            crate::auth::registration::RegistrationMode,
            crate::routes::services::ServiceStatus,
            crate::routes::services::ServiceResponse,
            crate::routes::services::UpdateServiceRequest,
            crate::routes::services::MessageResponse,
            crate::routes::services::MergeAgentRequest,
            crate::routes::enrollment_tokens::CreateEnrollmentTokenRequest,
            crate::routes::enrollment_tokens::EnrollmentTokenCreatedResponse,
            crate::routes::enrollment_tokens::EnrollmentTokenResponse,
            crate::routes::enrollment_tokens::EnrollmentTokensSummary,
            crate::routes::settings_agent_certs::AgentCertificateSettingsResponse,
            crate::routes::settings_agent_certs::UpdateAgentCertificateSettingsRequest,
            crate::routes::system_alerts::SystemAlert,
            crate::routes::system_alerts::SystemAlertsResponse,
            crate::routes::api_tokens::CreateApiTokenRequest,
            crate::routes::api_tokens::CreateApiTokenResponse,
            crate::routes::api_tokens::ApiTokenResponse,
            crate::routes::api_tokens::ApiTokenListResponse,
            crate::routes::device_auth::DeviceAuthStartRequest,
            crate::routes::device_auth::DeviceAuthStartResponse,
            crate::routes::device_auth::DeviceAuthPollRequest,
            crate::routes::device_auth::DeviceAuthPollResponse,
            crate::routes::device_auth::DeviceAuthApproveRequest,
            crate::routes::device_auth::DeviceAuthApproveResponse,
            crate::routes::settings_network::NetworkSettingsResponse,
            crate::routes::settings_network::UpdateNetworkSettingsRequest,
            crate::routes::settings_mqtt::MqttClientResponse,
            crate::routes::settings_mqtt::MqttLimitResponse,
            crate::routes::settings_mqtt::CreateMqttClientRequest,
            crate::routes::settings_mqtt::UpdateMqttClientRequest,
            crate::routes::settings_mqtt::UpdateMqttLimitRequest,
            uptrakit_web_api_types::mqtt_transport::MqttTransport,
            crate::routes::hosts::HostResponse,
            crate::routes::hosts::HostAgentSummary,
            crate::routes::hosts::UpdateHostRequest,
            crate::routes::plugin_configs::CreatePluginConfigRequest,
            crate::routes::plugin_configs::UpdatePluginConfigRequest,
            crate::routes::plugin_configs::PluginConfigResponse,
            crate::routes::software_items::CreateSoftwareItemRequest,
            crate::routes::software_items::UpdateSoftwareItemRequest,
            crate::routes::software_items::AssignHostsRequest,
            crate::routes::software_items::UpdateHostAssignmentRequest,
            uptrakit_web_api_types::software_items::HostSoftwareAssignment,
            crate::routes::software_items::SoftwareItemResponse,
            crate::routes::software_items::SoftwareItemDetailResponse,
            crate::routes::software_items::SoftwareItemHostSummary,
            crate::routes::software_items::TriggerUpdateRequest,
            crate::routes::software_items::TriggerUpdateResponse,
            crate::routes::software_items::TriggerUpdateStatus,
            crate::routes::software_items::TriggerVersionCheckResponse,
            crate::routes::scheduler::ScheduledTaskResponse,
            crate::routes::scheduler::UpdateScheduledTaskRequest,
            crate::routes::scheduler::TriggerScheduledTaskResponse,
            crate::routes::update_history::UpdateHistoryResponse,
            crate::routes::update_history::UpdateStatus,
            crate::routes::hosts::TriggerDiscoveryResponse,
            crate::routes::hosts::DiscardDiscoveredResponse,
            crate::routes::autodiscovery::AutodiscoveryIgnoreResponse,
            crate::routes::autodiscovery::CreateAutodiscoveryIgnoreRequest,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::enrollment_tokens::EnrollmentTokenResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::services::ServiceResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::hosts::HostResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::software_items::SoftwareItemResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::update_history::UpdateHistoryResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::plugin_configs::PluginConfigResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::autodiscovery::AutodiscoveryIgnoreResponse>,
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
        crate::routes::oidc_auth::auth_methods,
        crate::routes::oidc_auth::oidc_authorize,
        crate::routes::oidc_auth::oidc_callback,
        crate::routes::oidc_auth::oidc_link,
        crate::routes::oidc_auth::oidc_exchange,
        crate::routes::oidc_auth::oidc_complete_registration,
        crate::routes::oidc_providers::create_provider,
        crate::routes::oidc_providers::list_providers,
        crate::routes::oidc_providers::get_provider,
        crate::routes::oidc_providers::update_provider,
        crate::routes::oidc_providers::delete_provider,
        crate::routes::oidc_providers::activate_provider,
        crate::routes::oidc_providers::deactivate_provider,
    ),
    components(schemas(
        crate::routes::oidc_auth::AuthMethodsResponse,
        crate::routes::oidc_auth::OidcProviderInfo,
        crate::routes::oidc_auth::OidcAuthorizeResponse,
        crate::routes::oidc_auth::OidcLinkRequest,
        crate::routes::oidc_auth::OidcExchangeRequest,
        crate::routes::oidc_auth::OidcCompleteRegistrationRequest,
        crate::routes::oidc_providers::CreateOidcProviderRequest,
        crate::routes::oidc_providers::UpdateOidcProviderRequest,
        crate::routes::oidc_providers::OidcProviderResponse,
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
        crate::error_response::error_response_with_code(
            StatusCode::NOT_FOUND,
            "Not found",
            "not_found",
        )
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
        .routes(routes!(crate::routes::auth::logout))
        .routes(routes!(crate::routes::auth::me))
        .routes(routes!(
            crate::routes::api_tokens::create_api_token,
            crate::routes::api_tokens::list_api_tokens
        ))
        .routes(routes!(crate::routes::api_tokens::revoke_api_token))
        .routes(routes!(
            crate::routes::settings::get_registration_settings,
            crate::routes::settings::update_registration_settings
        ))
        .routes(routes!(
            crate::routes::settings_combined::get_combined_settings
        ))
        .routes(routes!(
            crate::routes::settings_auth::get_authentication_settings,
            crate::routes::settings_auth::update_authentication_settings
        ))
        .routes(routes!(
            crate::routes::settings_agent_certs::get_agent_certificate_settings,
            crate::routes::settings_agent_certs::update_agent_certificate_settings
        ))
        .routes(routes!(crate::routes::services::list_services))
        .routes(routes!(
            crate::routes::enrollment_tokens::create_enrollment_token,
            crate::routes::enrollment_tokens::list_enrollment_tokens
        ))
        .routes(routes!(
            crate::routes::enrollment_tokens::get_enrollment_token,
            crate::routes::enrollment_tokens::revoke_enrollment_token
        ))
        .routes(routes!(crate::routes::services::approve_service))
        .routes(routes!(crate::routes::services::reject_service))
        .routes(routes!(
            crate::routes::services::get_service,
            crate::routes::services::update_service,
            crate::routes::services::deactivate_service
        ))
        .routes(routes!(crate::routes::services::merge_service))
        .routes(routes!(crate::routes::system_alerts::get_system_alerts))
        .routes(routes!(
            crate::routes::server_cert::renew_server_certificate
        ))
        .routes(routes!(
            crate::routes::settings_network::get_network_settings,
            crate::routes::settings_network::update_network_settings
        ))
        .routes(routes!(
            crate::routes::settings_mqtt::list_mqtt_settings,
            crate::routes::settings_mqtt::create_mqtt_settings
        ))
        .routes(routes!(
            crate::routes::settings_mqtt::get_mqtt_limit,
            crate::routes::settings_mqtt::update_mqtt_limit
        ))
        .routes(routes!(
            crate::routes::settings_mqtt::get_mqtt_settings,
            crate::routes::settings_mqtt::update_mqtt_settings,
            crate::routes::settings_mqtt::delete_mqtt_settings
        ))
        .routes(routes!(crate::routes::device_auth::device_auth_approve))
        .routes(routes!(crate::routes::settings_ca::rotate_ca))
        .routes(routes!(crate::routes::hosts::list_hosts))
        .routes(routes!(crate::routes::hosts::get_host))
        .routes(routes!(crate::routes::hosts::update_host))
        .routes(routes!(crate::routes::hosts::deactivate_host))
        .routes(routes!(
            crate::routes::plugin_configs::create_plugin_config,
            crate::routes::plugin_configs::list_plugin_configs
        ))
        .routes(routes!(crate::routes::plugin_configs::get_plugin_config))
        .routes(routes!(crate::routes::plugin_configs::update_plugin_config))
        .routes(routes!(crate::routes::plugin_configs::delete_plugin_config))
        .routes(routes!(
            crate::routes::software_items::create_software_item,
            crate::routes::software_items::list_software_items
        ))
        .routes(routes!(crate::routes::software_items::get_software_item))
        .routes(routes!(crate::routes::software_items::update_software_item))
        .routes(routes!(crate::routes::software_items::delete_software_item))
        .routes(routes!(crate::routes::software_items::assign_hosts))
        .routes(routes!(
            crate::routes::software_items::unassign_host,
            crate::routes::software_items::update_host_assignment
        ))
        .routes(routes!(crate::routes::software_items::trigger_update))
        .routes(routes!(crate::routes::software_items::check_versions))
        .routes(routes!(crate::routes::software_items::check_versions_host))
        .routes(routes!(crate::routes::scheduler::list_scheduled_tasks))
        .routes(routes!(crate::routes::scheduler::get_scheduled_task))
        .routes(routes!(crate::routes::scheduler::update_scheduled_task))
        .routes(routes!(crate::routes::scheduler::trigger_scheduled_task))
        .routes(routes!(crate::routes::update_history::list_update_history))
        .routes(routes!(crate::routes::update_history::get_update_history))
        .routes(routes!(crate::routes::update_history::stream_update_output))
        // Autodiscovery
        .routes(routes!(
            crate::routes::software_items::approve_software_item
        ))
        .routes(routes!(crate::routes::hosts::discover_host))
        .routes(routes!(crate::routes::hosts::discard_host_discovered))
        .routes(routes!(
            crate::routes::plugin_configs::discover_plugin_config
        ))
        .routes(routes!(
            crate::routes::plugin_configs::discard_plugin_config_discovered
        ))
        .routes(routes!(
            crate::routes::autodiscovery::list_autodiscovery_ignores,
            crate::routes::autodiscovery::create_autodiscovery_ignore
        ))
        .routes(routes!(
            crate::routes::autodiscovery::delete_autodiscovery_ignore
        ));

    // OIDC provider management routes require authentication and belong inside auth_routes.
    // The OIDC auth-flow routes (oidc_auth::*) are added to base_router below because
    // they must remain publicly reachable (browser redirects, token exchange, etc.).
    #[cfg(feature = "oidc")]
    let auth_routes = auth_routes
        .routes(routes!(crate::routes::oidc_providers::create_provider))
        .routes(routes!(crate::routes::oidc_providers::list_providers))
        .routes(routes!(crate::routes::oidc_providers::get_provider))
        .routes(routes!(crate::routes::oidc_providers::update_provider))
        .routes(routes!(crate::routes::oidc_providers::delete_provider))
        .routes(routes!(crate::routes::oidc_providers::activate_provider))
        .routes(routes!(crate::routes::oidc_providers::deactivate_provider));

    let auth_routes = auth_routes.route_layer(axum_mw::from_fn_with_state(
        Arc::clone(&state),
        crate::middleware::require_auth::require_auth,
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
        .routes(routes!(crate::routes::auth::register))
        .routes(routes!(crate::routes::auth::login))
        .routes(routes!(crate::routes::auth::refresh))
        .routes(routes!(crate::routes::device_auth::device_auth_start))
        .routes(routes!(crate::routes::device_auth::device_auth_poll))
        .merge(auth_routes);

    #[cfg(feature = "oidc")]
    let base_router = base_router
        .routes(routes!(crate::routes::oidc_auth::auth_methods))
        .routes(routes!(crate::routes::oidc_auth::oidc_authorize))
        .routes(routes!(crate::routes::oidc_auth::oidc_callback))
        .routes(routes!(crate::routes::oidc_auth::oidc_link))
        .routes(routes!(crate::routes::oidc_auth::oidc_exchange))
        .routes(routes!(
            crate::routes::oidc_auth::oidc_complete_registration
        ));

    let (api_router, api) = base_router.split_for_parts();

    let mut router = api_router
        .route(
            "/api/v1/ws/service",
            get(crate::routes::service_ws::service_ws),
        )
        .route("/healthz", get(crate::routes::health::healthz))
        .route("/api/v1/pki/ca.crt", get(crate::routes::ca::ca_cert))
        .route("/api/v1/pki/ca.crl", get(crate::routes::ca::ca_crl))
        .route(
            "/api/v1/pki/ocsp",
            axum::routing::post(crate::routes::ocsp::ocsp_post),
        )
        .route(
            "/api/v1/pki/ocsp/{encoded}",
            get(crate::routes::ocsp::ocsp_get),
        );

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
            crate::middleware::resolve_proxy_headers::resolve_proxy_headers,
        ))
        .layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            crate::middleware::rate_limit::rate_limit_auth,
        ))
        .layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            crate::middleware::resolve_ip::resolve_ip,
        ))
        .layer(axum_mw::from_fn(
            crate::middleware::request_log::request_log,
        ))
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
        .route("/healthz", get(crate::routes::health::healthz))
        .route("/api/v1/pki/ca.crt", get(crate::routes::ca::ca_cert))
        .route("/api/v1/pki/ca.crl", get(crate::routes::ca::ca_crl))
        .route(
            "/api/v1/pki/ocsp",
            axum::routing::post(crate::routes::ocsp::ocsp_post),
        )
        .route(
            "/api/v1/pki/ocsp/{encoded}",
            get(crate::routes::ocsp::ocsp_get),
        )
        .layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            crate::middleware::resolve_ip::resolve_ip,
        ))
        .layer(axum_mw::from_fn(
            crate::middleware::request_log::request_log,
        ))
        .with_state(state)
}
