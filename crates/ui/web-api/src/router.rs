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
        (name = "System Services", description = "Tenant-agnostic infrastructure service management"),
        (name = "Enrollment Tokens", description = "Enrollment token management"),
        (name = "OIDC Providers", description = "OIDC provider configuration"),
        (name = "API Tokens", description = "Personal access token management"),
        (name = "Hosts", description = "Host machine management"),
        (name = "Plugin Configs", description = "Plugin configuration management"),
        (name = "Software Items", description = "Software item tracking and host assignment"),
        (name = "Update History", description = "Software update history tracking"),
        (name = "Autodiscovery", description = "Automatic software discovery management"),
        (name = "Update Batches", description = "Batch update operations"),
        (name = "Host Tags", description = "Host tag management"),
        (name = "Notifications", description = "Notification channel, rule, and log management"),
        (name = "Global Settings", description = "Infrastructure-scoped settings requiring global administrator access"),
        (name = "Audit Logs", description = "Tenant and system-level audit log access"),
        (name = "Users", description = "User management, roles, and access presets")
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
        crate::routes::services::set_update_freeze,
        crate::routes::services::merge_service,
        crate::routes::services::batch_services,
        crate::routes::system_services::list_system_services,
        crate::routes::system_services::get_system_service,
        crate::routes::system_services::update_system_service,
        crate::routes::system_services::approve_system_service,
        crate::routes::system_services::reject_system_service,
        crate::routes::system_services::deactivate_system_service,
        crate::routes::system_services::batch_system_services,
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
        crate::routes::settings_global_combined::get_global_combined_settings,
        crate::routes::settings_provider_github::get_github_provider_settings,
        crate::routes::settings_provider_github::update_github_provider_settings,
        crate::routes::settings_network::get_network_settings,
        crate::routes::settings_network::update_network_settings,
        crate::routes::hosts::list_hosts,
        crate::routes::hosts::get_host,
        crate::routes::hosts::update_host,
        crate::routes::hosts::deactivate_host,
        crate::routes::hosts::batch_hosts,
        crate::routes::plugin_configs::list_plugin_types,
        crate::routes::plugin_configs::create_plugin_config,
        crate::routes::plugin_configs::list_plugin_configs,
        crate::routes::plugin_configs::get_plugin_config,
        crate::routes::plugin_configs::update_plugin_config,
        crate::routes::plugin_configs::delete_plugin_config,
        crate::routes::plugin_configs::batch_plugin_configs,
        crate::routes::plugin_configs::test_plugin_config,
        crate::routes::software_items::create_software_item,
        crate::routes::software_items::list_software_items,
        crate::routes::software_items::preview_software_item_merge,
        crate::routes::software_items::execute_software_item_merge,
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
        crate::routes::software_items::batch_software_items,
        crate::routes::hosts::discover_host,
        // Host tags
        crate::routes::host_tags::list_host_tags,
        crate::routes::host_tags::create_host_tag,
        crate::routes::host_tags::get_host_tag,
        crate::routes::host_tags::update_host_tag,
        crate::routes::host_tags::delete_host_tag,
        crate::routes::host_tags::batch_host_tags,
        crate::routes::host_tags::set_host_tags,
        crate::routes::plugin_configs::discover_plugin_config,
        crate::routes::autodiscovery::list_autodiscovery_ignores,
        crate::routes::autodiscovery::create_autodiscovery_ignore,
        crate::routes::autodiscovery::delete_autodiscovery_ignore,
        crate::routes::autodiscovery::batch_autodiscovery_ignores,
        crate::routes::discovery_allowlist::list_tenant_discovery_allowlist,
        crate::routes::discovery_allowlist::add_tenant_discovery_allowlist_entry,
        crate::routes::discovery_allowlist::remove_tenant_discovery_allowlist_entry,
        crate::routes::discovery_allowlist::list_host_discovery_allowlist,
        crate::routes::discovery_allowlist::add_host_discovery_allowlist_entry,
        crate::routes::discovery_allowlist::remove_host_discovery_allowlist_entry,
        crate::routes::settings_ca::rotate_ca,
        crate::routes::scheduler::list_scheduled_tasks,
        crate::routes::scheduler::get_scheduled_task,
        crate::routes::scheduler::update_scheduled_task,
        crate::routes::scheduler::trigger_scheduled_task,
        crate::routes::update_history::list_update_history,
        crate::routes::update_history::get_update_history,
        crate::routes::update_history::stream_update_output,
        crate::routes::update_batches::trigger_host_batch_update,
        crate::routes::update_batches::trigger_item_batch_update,
        crate::routes::update_batches::list_batches,
        crate::routes::update_batches::get_batch,
        crate::routes::update_batches::stream_batch_progress,
        crate::routes::notifications::create_channel,
        crate::routes::notifications::list_channels,
        crate::routes::notifications::get_channel,
        crate::routes::notifications::update_channel,
        crate::routes::notifications::delete_channel,
        crate::routes::notifications::test_channel,
        crate::routes::notifications::create_rule,
        crate::routes::notifications::list_rules,
        crate::routes::notifications::get_rule,
        crate::routes::notifications::update_rule,
        crate::routes::notifications::delete_rule,
        crate::routes::notifications::list_log,
        // System enrollment tokens
        crate::routes::system_enrollment_tokens::create_system_enrollment_token,
        crate::routes::system_enrollment_tokens::list_system_enrollment_tokens,
        crate::routes::system_enrollment_tokens::get_system_enrollment_token,
        crate::routes::system_enrollment_tokens::revoke_system_enrollment_token,
        // Audit logs
        crate::routes::audit_logs::list_audit_logs,
        crate::routes::audit_logs::list_system_audit_logs,
        // User management
        crate::routes::users::list_users,
        crate::routes::users::get_user,
        crate::routes::users::update_user_roles,
        crate::routes::users::update_user_active,
        crate::routes::users::list_permissions,
        // Roles (read-only)
        crate::routes::roles::list_roles,
        crate::routes::roles::get_role,
        // Access presets
        crate::routes::access_presets::list_access_presets,
        crate::routes::access_presets::apply_preset,
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
            crate::routes::services::SetUpdateFreezeRequest,
            crate::routes::services::BatchActionRequest,
            crate::routes::services::BatchActionResponse,
            crate::routes::services::BatchActionSuccess,
            crate::routes::services::BatchActionFailure,
            crate::routes::system_services::SystemServiceResponse,
            crate::routes::system_services::UpdateSystemServiceRequest,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::system_services::SystemServiceResponse>,
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
            crate::routes::hosts::HostResponse,
            crate::routes::hosts::HostAgentSummary,
            crate::routes::hosts::UpdateHostRequest,
            crate::routes::plugin_configs::PluginTypeInfo,
            uptrakit_shared_types::PluginCapability,
            crate::routes::plugin_configs::CreatePluginConfigRequest,
            crate::routes::plugin_configs::UpdatePluginConfigRequest,
            crate::routes::plugin_configs::PluginConfigResponse,
            uptrakit_web_api_types::plugin_config_test::TestPluginConfigRequest,
            uptrakit_web_api_types::plugin_config_test::TestPluginConfigResponse,
            uptrakit_web_api_types::plugin_type_settings::PluginTypeSettingsResponse,
            uptrakit_web_api_types::plugin_type_settings::UpsertPluginTypeSettingsRequest,
            crate::routes::software_items::CreateSoftwareItemRequest,
            crate::routes::software_items::UpdateSoftwareItemRequest,
            crate::routes::software_items::AssignHostsRequest,
            crate::routes::software_items::UpdateHostAssignmentRequest,
            crate::routes::software_items::MergeSoftwareItemsPreviewRequest,
            crate::routes::software_items::MergeSoftwareItemsPreviewResponse,
            crate::routes::software_items::MergeSoftwareItemsExecuteRequest,
            crate::routes::software_items::MergeSoftwareItemsExecuteResponse,
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
            crate::routes::autodiscovery::SoftwareIgnoreResponse,
            crate::routes::autodiscovery::CreateSoftwareIgnoreRequest,
            crate::routes::discovery_allowlist::TenantDiscoveryAllowlistEntry,
            crate::routes::discovery_allowlist::HostDiscoveryAllowlistEntry,
            crate::routes::discovery_allowlist::CreateDiscoveryAllowlistEntryRequest,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::enrollment_tokens::EnrollmentTokenResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::services::ServiceResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::hosts::HostResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::software_items::SoftwareItemResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::update_history::UpdateHistoryResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::plugin_configs::PluginConfigResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::autodiscovery::SoftwareIgnoreResponse>,
            // Host tags
            crate::routes::host_tags::HostTagResponse,
            crate::routes::host_tags::HostTagSummary,
            crate::routes::host_tags::CreateHostTagRequest,
            crate::routes::host_tags::UpdateHostTagRequest,
            crate::routes::host_tags::SetHostTagsRequest,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::host_tags::HostTagResponse>,
            crate::routes::notifications::CreateNotificationChannelRequest,
            crate::routes::notifications::UpdateNotificationChannelRequest,
            crate::routes::notifications::NotificationChannelResponse,
            crate::routes::notifications::CreateNotificationRuleRequest,
            crate::routes::notifications::UpdateNotificationRuleRequest,
            crate::routes::notifications::NotificationRuleResponse,
            crate::routes::notifications::NotificationEventType,
            crate::routes::notifications::NotificationLogResponse,
            crate::routes::notifications::NotificationDeliveryStatus,
            crate::routes::notifications::TestNotificationResponse,
            // System enrollment tokens
            crate::routes::system_enrollment_tokens::CreateSystemEnrollmentTokenRequest,
            crate::routes::system_enrollment_tokens::SystemEnrollmentTokenCreatedResponse,
            crate::routes::system_enrollment_tokens::SystemEnrollmentTokenResponse,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::system_enrollment_tokens::SystemEnrollmentTokenResponse>,
            uptrakit_web_api_types::settings_combined::GlobalSettingsCombinedResponse,
            // Audit logs
            crate::routes::audit_logs::AuditLogResponse,
            crate::routes::audit_logs::SystemAuditLogResponse,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::audit_logs::AuditLogResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::audit_logs::SystemAuditLogResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::notifications::NotificationChannelResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::notifications::NotificationRuleResponse>,
            uptrakit_web_api_types::pagination::PaginatedResponse<crate::routes::notifications::NotificationLogResponse>,
            // User management
            crate::routes::users::UserWithRolesResponse,
            crate::routes::users::UserRoleSummary,
            crate::routes::users::UpdateUserRolesRequest,
            crate::routes::users::UpdateUserActiveRequest,
            crate::routes::users::ApplyPresetRequest,
            crate::routes::users::PermissionInfo,
            crate::routes::roles::RoleResponse,
            crate::routes::access_presets::AccessPresetResponse,
            // Update batches
            crate::routes::update_batches::BatchUpdateResponse,
            crate::routes::update_batches::BatchUpdateItem,
            crate::routes::update_batches::BatchSkippedItem,
            uptrakit_web_api_types::update_batches::HostBatchUpdateRequest,
            uptrakit_web_api_types::update_batches::ItemBatchUpdateRequest,
            uptrakit_web_api_types::update_batches::UpdateBatchSummaryResponse,
            uptrakit_web_api_types::update_batches::UpdateBatchDetailResponse,
            uptrakit_web_api_types::update_batches::UpdateBatchItemSummary,
            uptrakit_web_api_types::pagination::PaginatedResponse<uptrakit_web_api_types::update_batches::UpdateBatchSummaryResponse>,
            crate::routes::settings_provider_github::GitHubProviderSettingsResponse,
            crate::routes::settings_provider_github::UpdateGitHubProviderSettingsRequest,
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

/// Zeroconf OpenAPI paths and schemas.
#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::settings_zeroconf::get_zeroconf_settings,
        crate::routes::settings_zeroconf::update_zeroconf_settings,
    ),
    components(schemas(
        crate::routes::settings_zeroconf::ZeroconfSettingsResponse,
        crate::routes::settings_zeroconf::UpdateZeroconfSettingsRequest,
    ))
)]
struct ZeroconfApiDoc;

/// NATS-specific OpenAPI paths and schemas, merged conditionally.
#[cfg(feature = "nats")]
#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::settings_nats::get_nats_settings,
        crate::routes::settings_nats::update_nats_settings,
    ),
    components(schemas(
        crate::routes::settings_nats::NatsSettingsResponse,
        crate::routes::settings_nats::UpdateNatsSettingsRequest,
    ))
)]
struct NatsApiDoc;

/// Reset-data OpenAPI paths and schemas, merged conditionally.
#[cfg(feature = "reset-data")]
#[derive(OpenApi)]
#[openapi(
    paths(crate::routes::settings_reset::reset_data),
    components(schemas(
        uptrakit_web_api_types::settings_reset::ResetDataRequest,
        uptrakit_web_api_types::settings_reset::ResetDataResponse,
        uptrakit_web_api_types::settings_reset::ResetDeletedCounts,
    ))
)]
struct ResetDataApiDoc;

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
        .routes(routes!(crate::routes::services::batch_services))
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
        .routes(routes!(crate::routes::services::set_update_freeze))
        .routes(routes!(crate::routes::services::merge_service))
        // Batch actions
        .routes(routes!(
            crate::routes::system_services::batch_system_services
        ))
        .routes(routes!(
            crate::routes::system_services::list_system_services
        ))
        .routes(routes!(
            crate::routes::system_services::approve_system_service
        ))
        .routes(routes!(
            crate::routes::system_services::reject_system_service
        ))
        .routes(routes!(
            crate::routes::system_services::get_system_service,
            crate::routes::system_services::update_system_service,
            crate::routes::system_services::deactivate_system_service
        ))
        .routes(routes!(crate::routes::system_alerts::get_system_alerts))
        .routes(routes!(
            crate::routes::server_cert::renew_server_certificate
        ))
        .routes(routes!(
            crate::routes::settings_global_combined::get_global_combined_settings
        ))
        .routes(routes!(
            crate::routes::settings_provider_github::get_github_provider_settings,
            crate::routes::settings_provider_github::update_github_provider_settings
        ))
        .routes(routes!(
            crate::routes::settings_network::get_network_settings,
            crate::routes::settings_network::update_network_settings
        ))
        .routes(routes!(crate::routes::device_auth::device_auth_approve))
        .routes(routes!(crate::routes::settings_ca::rotate_ca))
        .routes(routes!(crate::routes::hosts::list_hosts))
        .routes(routes!(crate::routes::hosts::get_host))
        .routes(routes!(crate::routes::hosts::update_host))
        .routes(routes!(crate::routes::hosts::deactivate_host))
        .routes(routes!(crate::routes::hosts::batch_hosts))
        .routes(routes!(crate::routes::plugin_configs::list_plugin_types))
        .routes(routes!(
            crate::routes::plugin_configs::create_plugin_config,
            crate::routes::plugin_configs::list_plugin_configs
        ))
        .routes(routes!(crate::routes::plugin_configs::get_plugin_config))
        .routes(routes!(crate::routes::plugin_configs::update_plugin_config))
        .routes(routes!(crate::routes::plugin_configs::delete_plugin_config))
        .routes(routes!(crate::routes::plugin_configs::batch_plugin_configs))
        .routes(routes!(crate::routes::plugin_configs::test_plugin_config))
        // Plugin type settings
        .routes(routes!(
            crate::routes::plugin_type_settings::list_plugin_type_settings
        ))
        .routes(routes!(
            crate::routes::plugin_type_settings::get_plugin_type_settings
        ))
        .routes(routes!(
            crate::routes::plugin_type_settings::upsert_plugin_type_settings
        ))
        .routes(routes!(
            crate::routes::plugin_type_settings::delete_plugin_type_settings
        ))
        .routes(routes!(
            crate::routes::software_items::create_software_item,
            crate::routes::software_items::list_software_items
        ))
        .routes(routes!(
            crate::routes::software_items::preview_software_item_merge
        ))
        .routes(routes!(
            crate::routes::software_items::execute_software_item_merge
        ))
        .routes(routes!(crate::routes::software_items::get_software_item))
        .routes(routes!(crate::routes::software_items::update_software_item))
        .routes(routes!(crate::routes::software_items::delete_software_item))
        .routes(routes!(crate::routes::software_items::assign_hosts))
        .routes(routes!(
            crate::routes::software_items::unassign_host,
            crate::routes::software_items::update_host_assignment
        ))
        .routes(routes!(
            crate::routes::software_items::delete_plugin_assignment
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
        // Update batches
        .routes(routes!(
            crate::routes::update_batches::trigger_host_batch_update
        ))
        .routes(routes!(
            crate::routes::update_batches::trigger_item_batch_update
        ))
        .routes(routes!(crate::routes::update_batches::list_batches))
        .routes(routes!(crate::routes::update_batches::get_batch))
        .routes(routes!(
            crate::routes::update_batches::stream_batch_progress
        ))
        // Autodiscovery
        .routes(routes!(
            crate::routes::software_items::approve_software_item
        ))
        .routes(routes!(crate::routes::software_items::batch_software_items))
        .routes(routes!(crate::routes::hosts::discover_host))
        // Host tags
        .routes(routes!(
            crate::routes::host_tags::list_host_tags,
            crate::routes::host_tags::create_host_tag
        ))
        .routes(routes!(crate::routes::host_tags::batch_host_tags))
        .routes(routes!(
            crate::routes::host_tags::get_host_tag,
            crate::routes::host_tags::update_host_tag,
            crate::routes::host_tags::delete_host_tag
        ))
        .routes(routes!(crate::routes::host_tags::set_host_tags))
        .routes(routes!(
            crate::routes::plugin_configs::discover_plugin_config
        ))
        .routes(routes!(
            crate::routes::autodiscovery::list_autodiscovery_ignores,
            crate::routes::autodiscovery::create_autodiscovery_ignore
        ))
        .routes(routes!(
            crate::routes::autodiscovery::delete_autodiscovery_ignore
        ))
        .routes(routes!(
            crate::routes::autodiscovery::batch_autodiscovery_ignores
        ))
        // Notifications
        .routes(routes!(
            crate::routes::notifications::create_channel,
            crate::routes::notifications::list_channels
        ))
        .routes(routes!(
            crate::routes::notifications::get_channel,
            crate::routes::notifications::update_channel,
            crate::routes::notifications::delete_channel
        ))
        .routes(routes!(crate::routes::notifications::test_channel))
        .routes(routes!(
            crate::routes::notifications::create_rule,
            crate::routes::notifications::list_rules
        ))
        .routes(routes!(
            crate::routes::notifications::get_rule,
            crate::routes::notifications::update_rule,
            crate::routes::notifications::delete_rule
        ))
        .routes(routes!(crate::routes::notifications::list_log))
        // System enrollment tokens
        .routes(routes!(
            crate::routes::system_enrollment_tokens::create_system_enrollment_token,
            crate::routes::system_enrollment_tokens::list_system_enrollment_tokens
        ))
        .routes(routes!(
            crate::routes::system_enrollment_tokens::get_system_enrollment_token,
            crate::routes::system_enrollment_tokens::revoke_system_enrollment_token
        ))
        // Audit logs
        .routes(routes!(crate::routes::audit_logs::list_audit_logs))
        .routes(routes!(crate::routes::audit_logs::list_system_audit_logs))
        // User management
        .routes(routes!(crate::routes::users::list_users))
        .routes(routes!(crate::routes::users::list_permissions))
        .routes(routes!(crate::routes::users::get_user))
        .routes(routes!(crate::routes::users::update_user_roles))
        .routes(routes!(crate::routes::users::update_user_active))
        // Roles (read-only)
        .routes(routes!(crate::routes::roles::list_roles))
        .routes(routes!(crate::routes::roles::get_role))
        // Access presets
        .routes(routes!(crate::routes::access_presets::list_access_presets))
        .routes(routes!(crate::routes::access_presets::apply_preset))
        // Admin events SSE stream
        .route(
            "/api/v1/events/stream",
            get(crate::routes::events::stream_events),
        );

    // Surface endpoints — plain axum routes (no OpenAPI annotations).
    let auth_routes = auth_routes
        .route(
            "/api/v1/surfaces",
            axum::routing::get(crate::routes::surfaces::list_surfaces),
        )
        .route(
            "/api/v1/surfaces/runtime-status",
            axum::routing::get(crate::routes::surfaces::get_surface_runtime_status),
        )
        .route(
            "/api/v1/surfaces/{surface_id}/providers",
            axum::routing::get(crate::routes::surfaces::list_surface_providers),
        )
        .route(
            "/api/v1/surfaces/{surface_id}/read",
            axum::routing::get(crate::routes::surfaces::get_surface_read),
        )
        .route(
            "/api/v1/surfaces/{surface_id}/interactions/{interaction_id}",
            axum::routing::post(crate::routes::surfaces::invoke_surface_interaction),
        );

    // Zeroconf settings
    let auth_routes = auth_routes.routes(routes!(
        crate::routes::settings_zeroconf::get_zeroconf_settings,
        crate::routes::settings_zeroconf::update_zeroconf_settings
    ));

    // Reset data
    #[cfg(feature = "reset-data")]
    let auth_routes = auth_routes.routes(routes!(crate::routes::settings_reset::reset_data));

    // NATS settings
    #[cfg(feature = "nats")]
    let auth_routes = auth_routes.routes(routes!(
        crate::routes::settings_nats::get_nats_settings,
        crate::routes::settings_nats::update_nats_settings
    ));

    let auth_routes = auth_routes
        // Discovery allowlist
        .routes(routes!(
            crate::routes::discovery_allowlist::list_tenant_discovery_allowlist,
            crate::routes::discovery_allowlist::add_tenant_discovery_allowlist_entry
        ))
        .routes(routes!(
            crate::routes::discovery_allowlist::remove_tenant_discovery_allowlist_entry
        ))
        .routes(routes!(
            crate::routes::discovery_allowlist::list_host_discovery_allowlist,
            crate::routes::discovery_allowlist::add_host_discovery_allowlist_entry
        ))
        .routes(routes!(
            crate::routes::discovery_allowlist::remove_host_discovery_allowlist_entry
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

    // audit_log declared FIRST = inner layer = runs AFTER require_auth
    let auth_routes = auth_routes
        .route_layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            crate::middleware::audit_log::audit_log,
        ))
        .route_layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            crate::middleware::require_auth::require_auth,
        ));

    // All OpenAPI routes merged into a single router so the spec is complete
    let openapi = {
        let mut openapi = ApiDoc::openapi();
        openapi.merge(ZeroconfApiDoc::openapi());
        openapi
    };

    #[cfg(feature = "nats")]
    let openapi = {
        let mut openapi = openapi;
        openapi.merge(NatsApiDoc::openapi());
        openapi
    };

    #[cfg(feature = "reset-data")]
    let openapi = {
        let mut openapi = openapi;
        openapi.merge(ResetDataApiDoc::openapi());
        openapi
    };

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
        .merge(auth_routes)
        .route(
            "/api/v1/auth/device/stream",
            get(crate::routes::device_auth::device_auth_stream),
        );

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
            "/api/v1/notifications/callback/{channel_type}/{channel_id}",
            axum::routing::post(crate::routes::notifications::notification_callback),
        )
        .route(
            "/api/v1/ws/service",
            get(crate::routes::service_ws::service_ws),
        )
        .route("/healthz", get(crate::routes::health::healthz))
        .route("/readyz", get(crate::routes::health::readyz))
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

    // Interactive update WebSocket endpoint (manual auth via query/header token).
    #[cfg(feature = "interactive")]
    {
        router = router.route(
            "/api/v1/update-history/{id}/interactive",
            get(crate::routes::interactive_ws::interactive_ws),
        );
    }

    // Always serve the raw OpenAPI JSON at a stable URL.
    let api_for_json = api.clone();
    router = router.route(
        "/api/openapi.json",
        get(move || async move { axum::Json(api_for_json) }),
    );

    // When swagger-ui is compiled in, additionally serve the Swagger UI.
    // It uses a separate spec path (/api/docs/openapi.json) to avoid
    // conflicting with the always-present /api/openapi.json route.
    #[cfg(feature = "swagger-ui")]
    {
        use utoipa_swagger_ui::SwaggerUi;
        router = router.merge(SwaggerUi::new("/api/docs").url("/api/docs/openapi.json", api));
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
        .route("/readyz", get(crate::routes::health::readyz))
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
        .layer(axum_mw::from_fn(crate::middleware::request_id::request_id))
        .layer(axum_mw::from_fn(
            crate::middleware::security_headers::security_headers,
        ))
        .with_state(state)
}
