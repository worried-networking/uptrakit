//! `From<Report<DomainError>> for ApiError` impls for every domain error type
//! surfaced by route handlers.
//!
//! ## Conventions
//!
//! - All `match` arms are **exhaustive** — no `_` wildcard arms.
//! - **5xx arms** always use a literal, generic `user_message` (never dynamic
//!   domain error display) to avoid leaking internal state to clients.
//! - **Dynamic-display arms** (listed in [`DYNAMIC_DISPLAY_ALLOWLIST`]) use
//!   `ctx.to_string()` as `user_message`.  Every entry in the allowlist carries
//!   an inline safety rationale comment.
//! - `internal_detail` is `Some(format_report_summary(&report))` for 5xx arms
//!   and `None` for 4xx/other client-error arms.

use axum::http::StatusCode;
use rootcause::Report;

use uptrakit_web_api_queries::queries::{
    audit_logs::AuditLogQueryError,
    autodiscovery::AutodiscoveryError,
    discovery_allowlist::AllowlistError,
    notifications::{ChannelQueryError, RuleQueryError},
    plugin_configs::PluginConfigError,
    plugin_type_settings::PluginTypeSettingsError,
    reset_data::ResetDataQueryError,
    scheduled_tasks::ScheduledTaskError,
    services::ServiceQueryError,
    software_items::SoftwareItemQueryError,
    system_enrollment_tokens::SystemEnrollmentTokenError,
    system_services::SystemServiceQueryError,
    update_dispatch::TriggerUpdateError,
};

use crate::auth::{
    device_flow::DeviceFlowError, error::AuthError, registration::RegistrationValidationError,
};

use super::{ApiError, format_report_summary};

/// Variants whose domain-error `Display` output is safe to forward verbatim in
/// the HTTP response body.
///
/// A variant belongs here when its `Display` text:
/// 1. Contains only caller-controlled input that was already submitted in the
///    request (echoing it back is not a new disclosure), OR
/// 2. Is produced by validated plugin/schema logic and never includes secrets,
///    stack traces, or internal system identifiers.
#[cfg(test)]
pub(crate) const DYNAMIC_DISPLAY_ALLOWLIST: &[&str] = &[
    "AuditLogQueryError::InvalidFilter",
    "ChannelQueryError::InvalidConfig",
    "ChannelQueryError::UnsupportedType",
    "PluginConfigError::ConfigValidation",
    "RuleQueryError::InvalidField",
    "SoftwareItemQueryError::IncompatibleHost",
    "SoftwareItemQueryError::InvalidConfigOverride",
    "SoftwareItemQueryError::InvalidExecutionSite",
    "SoftwareItemQueryError::InvalidInlinePluginConfig",
    "SoftwareItemQueryError::InvalidPackageIdentifier",
    "TriggerUpdateError::UnknownPluginType",
];

// ---------------------------------------------------------------------------
// ServiceQueryError
// ---------------------------------------------------------------------------

impl From<Report<ServiceQueryError>> for ApiError {
    fn from(report: Report<ServiceQueryError>) -> Self {
        use ServiceQueryError::*;
        let ctx = report.current_context();
        match ctx {
            NotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Service not found.",
                "service.not_found",
                None,
            ),
            NotPending => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Service is not in a pending state.",
                "service.not_pending",
                None,
            ),
            NotApproved => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Service is not approved.",
                "service.not_approved",
                None,
            ),
            NotMergeable => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Services cannot be merged in their current state.",
                "service.not_mergeable",
                None,
            ),
            TargetConnected => ApiError::new(
                StatusCode::CONFLICT,
                "Target service is currently connected.",
                "service.target_connected",
                None,
            ),
            SourceNotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Source service not found.",
                "service.source_not_found",
                None,
            ),
            EmbeddedService => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Operation is not permitted on embedded services.",
                "service.embedded_service",
                None,
            ),
            Db(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "service.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// SystemServiceQueryError
// ---------------------------------------------------------------------------

impl From<Report<SystemServiceQueryError>> for ApiError {
    fn from(report: Report<SystemServiceQueryError>) -> Self {
        use SystemServiceQueryError::*;
        let ctx = report.current_context();
        match ctx {
            NotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "System service not found.",
                "system_service.not_found",
                None,
            ),
            NotPending => ApiError::new(
                StatusCode::BAD_REQUEST,
                "System service is not in a pending state.",
                "system_service.not_pending",
                None,
            ),
            NotApproved => ApiError::new(
                StatusCode::BAD_REQUEST,
                "System service is not approved.",
                "system_service.not_approved",
                None,
            ),
            EmbeddedService => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Operation is not permitted on embedded system services.",
                "system_service.embedded_service",
                None,
            ),
            Db(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "system_service.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// PluginConfigError
// ---------------------------------------------------------------------------

impl From<Report<PluginConfigError>> for ApiError {
    fn from(report: Report<PluginConfigError>) -> Self {
        use PluginConfigError::*;
        let ctx = report.current_context();
        match ctx {
            NotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Plugin configuration not found.",
                "plugin_config.not_found",
                None,
            ),
            EmptyName => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Plugin configuration name must not be empty.",
                "plugin_config.empty_name",
                None,
            ),
            DuplicateName => ApiError::new(
                StatusCode::CONFLICT,
                "A plugin configuration with this name already exists.",
                "plugin_config.duplicate_name",
                None,
            ),
            ConfigValidation(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: ConfigValidation wraps a human-readable validation
                // message generated by plugin schema validation — never contains
                // secrets or internal state.
                ctx.to_string(),
                "plugin_config.config_validation",
                None,
            ),
            Db(_) | Internal(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "plugin_config.internal_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// ChannelQueryError
// ---------------------------------------------------------------------------

impl From<Report<ChannelQueryError>> for ApiError {
    fn from(report: Report<ChannelQueryError>) -> Self {
        use ChannelQueryError::*;
        let ctx = report.current_context();
        match ctx {
            UnsupportedType(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: UnsupportedType contains the type string from the
                // request payload — caller-controlled, safe to echo back.
                ctx.to_string(),
                "notification_channel.unsupported_type",
                None,
            ),
            InvalidConfig(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: InvalidConfig contains a validation message produced
                // by plugin config schema checks — no secrets or internal state.
                ctx.to_string(),
                "notification_channel.invalid_config",
                None,
            ),
            Db(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "notification_channel.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// RuleQueryError
// ---------------------------------------------------------------------------

impl From<Report<RuleQueryError>> for ApiError {
    fn from(report: Report<RuleQueryError>) -> Self {
        use RuleQueryError::*;
        let ctx = report.current_context();
        match ctx {
            ChannelNotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Notification channel not found.",
                "notification_rule.channel_not_found",
                None,
            ),
            InvalidField(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: InvalidField names the request field that failed
                // validation — caller-controlled, safe to echo.
                ctx.to_string(),
                "notification_rule.invalid_field",
                None,
            ),
            Db(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "notification_rule.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// AllowlistError
// ---------------------------------------------------------------------------

impl From<Report<AllowlistError>> for ApiError {
    fn from(report: Report<AllowlistError>) -> Self {
        use AllowlistError::*;
        let ctx = report.current_context();
        match ctx {
            InvalidPluginType => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Invalid plugin type for allowlist entry.",
                "allowlist.invalid_plugin_type",
                None,
            ),
            Db(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "allowlist.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// ScheduledTaskError
// ---------------------------------------------------------------------------

impl From<Report<ScheduledTaskError>> for ApiError {
    fn from(report: Report<ScheduledTaskError>) -> Self {
        use ScheduledTaskError::*;
        let ctx = report.current_context();
        match ctx {
            NotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Scheduled task not found.",
                "scheduled_task.not_found",
                None,
            ),
            InvalidInterval => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Invalid schedule interval.",
                "scheduled_task.invalid_interval",
                None,
            ),
            Db(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "scheduled_task.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// SoftwareItemQueryError
// ---------------------------------------------------------------------------

impl From<Report<SoftwareItemQueryError>> for ApiError {
    fn from(report: Report<SoftwareItemQueryError>) -> Self {
        use SoftwareItemQueryError::*;
        let ctx = report.current_context();
        match ctx {
            NotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Software item not found.",
                "software_item.not_found",
                None,
            ),
            PluginAssignmentNotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Plugin assignment not found.",
                "software_item.plugin_assignment_not_found",
                None,
            ),
            EmptyName => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Software item name must not be empty.",
                "software_item.empty_name",
                None,
            ),
            HostNotFound(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Host not found or unavailable.",
                "software_item.host_not_found",
                None,
            ),
            PluginConfigNotFound => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Plugin configuration not found.",
                "software_item.plugin_config_not_found",
                None,
            ),
            IncompatibleHost(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: IncompatibleHost contains a human-readable reason
                // from plugin compatibility checks — no secrets or internal paths.
                ctx.to_string(),
                "software_item.incompatible_host",
                None,
            ),
            InvalidPackageIdentifier(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: InvalidPackageIdentifier contains the package ID
                // string from the request — caller-controlled, safe to echo.
                ctx.to_string(),
                "software_item.invalid_package_identifier",
                None,
            ),
            InvalidConfigOverride(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: InvalidConfigOverride contains a validation message
                // from config schema checks — no secrets.
                ctx.to_string(),
                "software_item.invalid_config_override",
                None,
            ),
            InvalidInlinePluginConfig(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: InvalidInlinePluginConfig contains a validation
                // message from plugin config schema checks — no secrets.
                ctx.to_string(),
                "software_item.invalid_inline_plugin_config",
                None,
            ),
            InvalidExecutionSite(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: InvalidExecutionSite contains the execution site
                // string from the request — caller-controlled, safe to echo.
                ctx.to_string(),
                "software_item.invalid_execution_site",
                None,
            ),
            InvalidMergeRequest(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Invalid software item merge request.",
                "software_item.invalid_merge_request",
                None,
            ),
            DuplicateItem => ApiError::new(
                StatusCode::CONFLICT,
                "A software item with this configuration already exists.",
                "software_item.duplicate_item",
                None,
            ),
            DuplicateHostAssignment => ApiError::new(
                StatusCode::CONFLICT,
                "This host is already assigned to the software item.",
                "software_item.duplicate_host_assignment",
                None,
            ),
            Db(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "software_item.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// TriggerUpdateError
// ---------------------------------------------------------------------------

impl From<Report<TriggerUpdateError>> for ApiError {
    fn from(report: Report<TriggerUpdateError>) -> Self {
        use TriggerUpdateError::*;
        let ctx = report.current_context();
        match ctx {
            SoftwareItemNotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Software item not found.",
                "trigger_update.software_item_not_found",
                None,
            ),
            HostNotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Host not found.",
                "trigger_update.host_not_found",
                None,
            ),
            HostNotAssigned => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Host is not assigned to this software item.",
                "trigger_update.host_not_assigned",
                None,
            ),
            NoExecuteUpdatePlugin => ApiError::new(
                StatusCode::BAD_REQUEST,
                "No execute-update plugin is configured for this software item.",
                "trigger_update.no_execute_update_plugin",
                None,
            ),
            NoAgent => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Host does not have an active agent connection.",
                "trigger_update.no_agent",
                None,
            ),
            AgentNotApproved => ApiError::new(
                StatusCode::BAD_REQUEST,
                "The agent for this host has not been approved.",
                "trigger_update.agent_not_approved",
                None,
            ),
            UpdateAlreadyActive => ApiError::new(
                StatusCode::CONFLICT,
                "An update is already active for this host.",
                "trigger_update.update_already_active",
                None,
            ),
            PluginConfigNotFound => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Plugin configuration not found.",
                "trigger_update.plugin_config_not_found",
                None,
            ),
            UnknownPluginType(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: UnknownPluginType contains the plugin type string
                // from the request — caller-controlled, safe to echo.
                ctx.to_string(),
                "trigger_update.unknown_plugin_type",
                None,
            ),
            Database(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "trigger_update.database_error",
                Some(format_report_summary(&report)),
            ),
            PreUpdateProtection(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Update blocked by controller pre-update protection.",
                "trigger_update.pre_update_protection_failed",
                None,
            ),
            PostUpdateFinalization(_) | PostUpdateFinalizationTimeout => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "trigger_update.post_update_finalization_failed",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// AuditLogQueryError
// ---------------------------------------------------------------------------

impl From<Report<AuditLogQueryError>> for ApiError {
    fn from(report: Report<AuditLogQueryError>) -> Self {
        use AuditLogQueryError::*;
        let ctx = report.current_context();
        match ctx {
            InvalidFilter(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                // SAFETY: InvalidFilter contains a validation message about
                // the filter parameter from the request — safe to echo.
                ctx.to_string(),
                "audit_log.invalid_filter",
                None,
            ),
            Database(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "audit_log.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// DeviceFlowError
// ---------------------------------------------------------------------------

impl From<Report<DeviceFlowError>> for ApiError {
    fn from(report: Report<DeviceFlowError>) -> Self {
        use DeviceFlowError::*;
        let ctx = report.current_context();
        match ctx {
            NotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Device flow not found or expired.",
                "device_flow.not_found",
                None,
            ),
            AlreadyAuthorized => ApiError::new(
                StatusCode::CONFLICT,
                "Device flow has already been authorized.",
                "device_flow.already_authorized",
                None,
            ),
            TokenGeneration(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "device_flow.token_generation_error",
                Some(format_report_summary(&report)),
            ),
            Database(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "device_flow.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// RegistrationValidationError  (bare From, not Report)
// ---------------------------------------------------------------------------

impl From<RegistrationValidationError> for ApiError {
    fn from(err: RegistrationValidationError) -> Self {
        use RegistrationValidationError::*;
        match err {
            Closed => ApiError::new(
                StatusCode::FORBIDDEN,
                "Registration is currently closed.",
                "registration.closed",
                None,
            ),
            TokenRequired => ApiError::new(
                StatusCode::FORBIDDEN,
                "A registration token is required.",
                "registration.token_required",
                None,
            ),
            NoTokenConfigured => ApiError::new(
                StatusCode::FORBIDDEN,
                "No registration token is configured.",
                "registration.no_token_configured",
                None,
            ),
            InvalidToken => ApiError::new(
                StatusCode::FORBIDDEN,
                "Invalid registration token.",
                "registration.invalid_token",
                None,
            ),
            VerificationFailed => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "registration.verification_failed",
                // VerificationFailed is not a Report, so no detail chain.
                None,
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Proactive impls — no handler migration required
// ---------------------------------------------------------------------------

// PluginTypeSettingsError

impl From<Report<PluginTypeSettingsError>> for ApiError {
    fn from(report: Report<PluginTypeSettingsError>) -> Self {
        use PluginTypeSettingsError::*;
        let ctx = report.current_context();
        match ctx {
            NotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Plugin type settings not found.",
                "plugin_type_settings.not_found",
                None,
            ),
            Db(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "plugin_type_settings.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// AutodiscoveryError

impl From<Report<AutodiscoveryError>> for ApiError {
    fn from(report: Report<AutodiscoveryError>) -> Self {
        use AutodiscoveryError::*;
        let ctx = report.current_context();
        match ctx {
            Db(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "autodiscovery.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// ResetDataQueryError

impl From<Report<ResetDataQueryError>> for ApiError {
    fn from(report: Report<ResetDataQueryError>) -> Self {
        use ResetDataQueryError::*;
        let ctx = report.current_context();
        match ctx {
            Database(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "reset_data.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// SystemEnrollmentTokenError

impl From<Report<SystemEnrollmentTokenError>> for ApiError {
    fn from(report: Report<SystemEnrollmentTokenError>) -> Self {
        use SystemEnrollmentTokenError::*;
        let ctx = report.current_context();
        match ctx {
            Database(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "system_enrollment_token.database_error",
                Some(format_report_summary(&report)),
            ),
        }
    }
}

// AuthError

impl From<Report<AuthError>> for ApiError {
    fn from(report: Report<AuthError>) -> Self {
        use AuthError::*;
        let ctx = report.current_context();
        match ctx {
            InvalidCredentials => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Invalid credentials.",
                "auth.invalid_credentials",
                None,
            ),
            SessionExpired => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Session not found or expired.",
                "auth.session_expired",
                None,
            ),
            // Mapped to generic "invalid credentials" to prevent user enumeration.
            UserNotFound(_) => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Invalid credentials.",
                "auth.invalid_credentials",
                None,
            ),
            UserDeactivated => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Account is deactivated.",
                "auth.user_deactivated",
                None,
            ),
            PasswordHash(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
            TokenGeneration(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
            Database(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
            UuidParse(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
            TimeError(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
            OidcProviderNotFound => ApiError::new(
                StatusCode::BAD_REQUEST,
                "OIDC provider not found or inactive.",
                "auth.oidc_provider_not_found",
                None,
            ),
            OidcDiscovery(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
            OidcTokenExchange(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
            OidcTokenValidation(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                "OIDC token validation failed.",
                "auth.oidc_token_validation_failed",
                None,
            ),
            OidcStateNotFound => ApiError::new(
                StatusCode::BAD_REQUEST,
                "OIDC state not found or expired.",
                "auth.oidc_state_not_found",
                None,
            ),
            OidcNoAccount => ApiError::new(
                StatusCode::FORBIDDEN,
                "OIDC account not found and auto-creation is disabled.",
                "auth.oidc_no_account",
                None,
            ),
            OidcLinkRequired => ApiError::new(
                StatusCode::FORBIDDEN,
                "OIDC account linking is required.",
                "auth.oidc_link_required",
                None,
            ),
            OidcLinkVerificationFailed => ApiError::new(
                StatusCode::BAD_REQUEST,
                "OIDC link verification failed.",
                "auth.oidc_link_verification_failed",
                None,
            ),
            PasswordAuthDisabled => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Password authentication is disabled.",
                "auth.password_auth_disabled",
                None,
            ),
            CannotDisableOwnAuthMethod => ApiError::new(
                StatusCode::CONFLICT,
                "Cannot disable the auth method used by the current session.",
                "auth.cannot_disable_own_auth_method",
                None,
            ),
            NoAuthMethodsRemaining => ApiError::new(
                StatusCode::CONFLICT,
                "At least one auth method must remain enabled.",
                "auth.no_auth_methods_remaining",
                None,
            ),
            JwtEncode(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
            JwtDecode(_) => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "JWT validation failed.",
                "auth.jwt_decode_failed",
                None,
            ),
            InvalidRefreshToken => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Invalid refresh token.",
                "auth.invalid_refresh_token",
                None,
            ),
            RefreshTokenExpired => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Refresh token has expired.",
                "auth.refresh_token_expired",
                None,
            ),
            RefreshTokenRevoked => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Refresh token has been revoked.",
                "auth.refresh_token_revoked",
                None,
            ),
            ApiTokenNotFound => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "API token not found.",
                "auth.api_token_not_found",
                None,
            ),
            ApiTokenRevoked => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "API token has been revoked.",
                "auth.api_token_revoked",
                None,
            ),
            DeviceFlowNotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "Device flow not found or expired.",
                "auth.device_flow_not_found",
                None,
            ),
            DeviceFlowAlreadyAuthorized => ApiError::new(
                StatusCode::CONFLICT,
                "Device flow has already been authorized.",
                "auth.device_flow_already_authorized",
                None,
            ),
            Io(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
            InvalidSession => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Invalid session.",
                "auth.invalid_session",
                None,
            ),
            Internal(_) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "auth.internal",
                Some(format_report_summary(&report)),
            ),
        }
    }
}
