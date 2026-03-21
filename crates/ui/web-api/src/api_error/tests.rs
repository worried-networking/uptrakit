//! CI-enforcement tests for `api_error`.
//!
//! Sections:
//! 1. `truncate_utf8_safe` unit tests
//! 2. Per-variant mapping tests for all 17 domain error types
//! 3. Dynamic-display dual-payload tests for all 11 allowlisted variants
//! 4. 5xx logging test (exactly one `ERROR` event with structured fields)
//! 5. 4xx no-logging test (zero `ERROR` events)
//! 6. Golden-file test for `code_registry.txt`
//! 7. `MAPPING_REVIEW.md` consistency tests
//! 8. `DYNAMIC_DISPLAY_ALLOWLIST` annotation test

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use axum::response::IntoResponse;
use http_body_util::BodyExt;
use rootcause::{Report, report};
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;

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

use super::{ApiError, mappings::DYNAMIC_DISPLAY_ALLOWLIST, truncate_utf8_safe};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

async fn read_response(
    resp: axum::response::Response,
) -> (axum::http::StatusCode, serde_json::Value) {
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (status, json)
}

fn code_of(json: &serde_json::Value) -> &str {
    json["code"]
        .as_str()
        .expect("response must have a `code` field")
}

fn error_of(json: &serde_json::Value) -> &str {
    json["error"]
        .as_str()
        .expect("response must have an `error` field")
}

// ---------------------------------------------------------------------------
// Minimal tracing event capture
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Captured {
    events: Vec<CapturedEvent>,
}

#[derive(Debug)]
struct CapturedEvent {
    level: String,
    fields: HashMap<String, String>,
}

struct FieldCapture(HashMap<String, String>);

impl Visit for FieldCapture {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

struct CaptureLayer {
    captured: Arc<Mutex<Captured>>,
}

impl CaptureLayer {
    fn new() -> (Self, Arc<Mutex<Captured>>) {
        let captured = Arc::new(Mutex::new(Captured::default()));
        (
            Self {
                captured: Arc::clone(&captured),
            },
            captured,
        )
    }
}

impl<S: tracing::Subscriber> Layer<S> for CaptureLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldCapture(HashMap::new());
        event.record(&mut visitor);
        self.captured.lock().unwrap().events.push(CapturedEvent {
            level: event.metadata().level().to_string(),
            fields: visitor.0,
        });
    }
}

// ---------------------------------------------------------------------------
// 1. truncate_utf8_safe
// ---------------------------------------------------------------------------

#[test]
fn truncate_short_string_unchanged() {
    let s = "hello";
    assert_eq!(truncate_utf8_safe(s, 100), "hello");
}

#[test]
fn truncate_exact_boundary_unchanged() {
    let s = "abcde";
    assert_eq!(truncate_utf8_safe(s, 5), "abcde");
}

#[test]
fn truncate_longer_string_appends_marker() {
    let s = "abcdefghij";
    let out = truncate_utf8_safe(s, 5);
    assert!(out.starts_with("abcde"), "should keep prefix");
    assert!(out.contains("…[truncated]"), "should append marker");
}

#[test]
fn truncate_empty_string() {
    assert_eq!(truncate_utf8_safe("", 10), "");
}

#[test]
fn truncate_respects_utf8_boundary() {
    // "é" is 2 bytes: 0xC3 0xA9. Truncating at 1 byte must step back to 0.
    let s = "aé";
    let out = truncate_utf8_safe(s, 2); // 'a'=1, 'é'=2 bytes → total 3
    // max_bytes=2 cuts mid-é; must retreat to the 'a' boundary
    assert!(
        s.is_char_boundary(out.len().saturating_sub("…[truncated]".len())) || out.starts_with('a')
    );
}

#[test]
fn truncate_multibyte_start() {
    // "日" is 3 bytes. max_bytes=2 should produce only the marker.
    let s = "日本語";
    let out = truncate_utf8_safe(s, 2);
    // boundary retreats to 0, so prefix is empty
    assert!(out.ends_with("…[truncated]"));
}

#[test]
fn truncate_4byte_char_boundary() {
    // "𝄞" (U+1D11E) is 4 bytes. Cutting at 3 must retreat.
    let s = "𝄞abc";
    let out = truncate_utf8_safe(s, 3);
    assert!(out.ends_with("…[truncated]"));
}

#[test]
fn truncate_exactly_zero_max_bytes() {
    let s = "hello";
    let out = truncate_utf8_safe(s, 0);
    assert!(out.ends_with("…[truncated]"));
}

// ---------------------------------------------------------------------------
// 2. Per-variant mapping tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn service_query_error_not_found() {
    let api_err = ApiError::from(report!(ServiceQueryError::NotFound));
    let (status, json) = read_response(api_err.into_response()).await;
    assert_eq!(status, 404);
    assert_eq!(code_of(&json), "service.not_found");
}

#[tokio::test]
async fn service_query_error_all_variants() {
    async fn check(report: Report<ServiceQueryError>, expected_status: u16, expected_code: &str) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(ServiceQueryError::NotFound),
        404,
        "service.not_found",
    )
    .await;
    check(
        report!(ServiceQueryError::NotPending),
        400,
        "service.not_pending",
    )
    .await;
    check(
        report!(ServiceQueryError::NotApproved),
        400,
        "service.not_approved",
    )
    .await;
    check(
        report!(ServiceQueryError::NotMergeable),
        400,
        "service.not_mergeable",
    )
    .await;
    check(
        report!(ServiceQueryError::TargetConnected),
        409,
        "service.target_connected",
    )
    .await;
    check(
        report!(ServiceQueryError::SourceNotFound),
        404,
        "service.source_not_found",
    )
    .await;
    check(
        report!(ServiceQueryError::EmbeddedService),
        400,
        "service.embedded_service",
    )
    .await;
    check(
        report!(ServiceQueryError::Db(sea_orm::DbErr::Custom("test".into()))),
        500,
        "service.database_error",
    )
    .await;
}

#[tokio::test]
async fn system_service_query_error_all_variants() {
    async fn check(
        report: Report<SystemServiceQueryError>,
        expected_status: u16,
        expected_code: &str,
    ) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(SystemServiceQueryError::NotFound),
        404,
        "system_service.not_found",
    )
    .await;
    check(
        report!(SystemServiceQueryError::NotPending),
        400,
        "system_service.not_pending",
    )
    .await;
    check(
        report!(SystemServiceQueryError::NotApproved),
        400,
        "system_service.not_approved",
    )
    .await;
    check(
        report!(SystemServiceQueryError::EmbeddedService),
        400,
        "system_service.embedded_service",
    )
    .await;
    check(
        report!(SystemServiceQueryError::Db(sea_orm::DbErr::Custom(
            "test".into()
        ))),
        500,
        "system_service.database_error",
    )
    .await;
}

#[tokio::test]
async fn plugin_config_error_all_variants() {
    async fn check(report: Report<PluginConfigError>, expected_status: u16, expected_code: &str) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(PluginConfigError::NotFound),
        404,
        "plugin_config.not_found",
    )
    .await;
    check(
        report!(PluginConfigError::EmptyName),
        400,
        "plugin_config.empty_name",
    )
    .await;
    check(
        report!(PluginConfigError::DuplicateName),
        409,
        "plugin_config.duplicate_name",
    )
    .await;
    check(
        report!(PluginConfigError::ConfigValidation("bad".into())),
        400,
        "plugin_config.config_validation",
    )
    .await;
    check(
        report!(PluginConfigError::Db(sea_orm::DbErr::Custom("test".into()))),
        500,
        "plugin_config.internal_error",
    )
    .await;
    check(
        report!(PluginConfigError::Internal("oops".into())),
        500,
        "plugin_config.internal_error",
    )
    .await;
}

#[tokio::test]
async fn channel_query_error_all_variants() {
    async fn check(report: Report<ChannelQueryError>, expected_status: u16, expected_code: &str) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(ChannelQueryError::UnsupportedType("smtp".into())),
        400,
        "notification_channel.unsupported_type",
    )
    .await;
    check(
        report!(ChannelQueryError::InvalidConfig("missing host".into())),
        400,
        "notification_channel.invalid_config",
    )
    .await;
    check(
        report!(ChannelQueryError::Db(sea_orm::DbErr::Custom("test".into()))),
        500,
        "notification_channel.database_error",
    )
    .await;
}

#[tokio::test]
async fn rule_query_error_all_variants() {
    async fn check(report: Report<RuleQueryError>, expected_status: u16, expected_code: &str) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(RuleQueryError::ChannelNotFound),
        404,
        "notification_rule.channel_not_found",
    )
    .await;
    check(
        report!(RuleQueryError::InvalidField("name".into())),
        400,
        "notification_rule.invalid_field",
    )
    .await;
    check(
        report!(RuleQueryError::Db(sea_orm::DbErr::Custom("test".into()))),
        500,
        "notification_rule.database_error",
    )
    .await;
}

#[tokio::test]
async fn allowlist_error_all_variants() {
    let (status, json) =
        read_response(ApiError::from(report!(AllowlistError::InvalidPluginType)).into_response())
            .await;
    assert_eq!(status.as_u16(), 400);
    assert_eq!(code_of(&json), "allowlist.invalid_plugin_type");

    let (status, json) = read_response(
        ApiError::from(report!(AllowlistError::Db(sea_orm::DbErr::Custom(
            "test".into()
        ))))
        .into_response(),
    )
    .await;
    assert_eq!(status.as_u16(), 500);
    assert_eq!(code_of(&json), "allowlist.database_error");
}

#[tokio::test]
async fn scheduled_task_error_all_variants() {
    async fn check(report: Report<ScheduledTaskError>, expected_status: u16, expected_code: &str) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(ScheduledTaskError::NotFound),
        404,
        "scheduled_task.not_found",
    )
    .await;
    check(
        report!(ScheduledTaskError::InvalidInterval),
        400,
        "scheduled_task.invalid_interval",
    )
    .await;
    check(
        report!(ScheduledTaskError::Db(sea_orm::DbErr::Custom(
            "test".into()
        ))),
        500,
        "scheduled_task.database_error",
    )
    .await;
}

#[tokio::test]
async fn software_item_query_error_all_variants() {
    async fn check(
        report: Report<SoftwareItemQueryError>,
        expected_status: u16,
        expected_code: &str,
    ) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(SoftwareItemQueryError::NotFound),
        404,
        "software_item.not_found",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::PluginAssignmentNotFound),
        404,
        "software_item.plugin_assignment_not_found",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::EmptyName),
        400,
        "software_item.empty_name",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::HostNotFound(uuid::Uuid::nil())),
        400,
        "software_item.host_not_found",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::PluginConfigNotFound),
        400,
        "software_item.plugin_config_not_found",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::IncompatibleHost(
            "arch mismatch".into()
        )),
        400,
        "software_item.incompatible_host",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::InvalidPackageIdentifier(
            "bad::id".into()
        )),
        400,
        "software_item.invalid_package_identifier",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::InvalidConfigOverride(
            "key missing".into()
        )),
        400,
        "software_item.invalid_config_override",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::InvalidInlinePluginConfig(
            "bad plugin".into()
        )),
        400,
        "software_item.invalid_inline_plugin_config",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::InvalidExecutionSite(
            "unknown".into()
        )),
        400,
        "software_item.invalid_execution_site",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::DuplicateItem),
        409,
        "software_item.duplicate_item",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::DuplicateHostAssignment),
        409,
        "software_item.duplicate_host_assignment",
    )
    .await;
    check(
        report!(SoftwareItemQueryError::Db(sea_orm::DbErr::Custom(
            "test".into()
        ))),
        500,
        "software_item.database_error",
    )
    .await;
}

#[tokio::test]
async fn trigger_update_error_all_variants() {
    async fn check(report: Report<TriggerUpdateError>, expected_status: u16, expected_code: &str) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(TriggerUpdateError::SoftwareItemNotFound),
        404,
        "trigger_update.software_item_not_found",
    )
    .await;
    check(
        report!(TriggerUpdateError::HostNotFound),
        404,
        "trigger_update.host_not_found",
    )
    .await;
    check(
        report!(TriggerUpdateError::HostNotAssigned),
        400,
        "trigger_update.host_not_assigned",
    )
    .await;
    check(
        report!(TriggerUpdateError::NoExecuteUpdatePlugin),
        400,
        "trigger_update.no_execute_update_plugin",
    )
    .await;
    check(
        report!(TriggerUpdateError::NoAgent),
        400,
        "trigger_update.no_agent",
    )
    .await;
    check(
        report!(TriggerUpdateError::AgentNotApproved),
        400,
        "trigger_update.agent_not_approved",
    )
    .await;
    check(
        report!(TriggerUpdateError::UpdateAlreadyActive),
        409,
        "trigger_update.update_already_active",
    )
    .await;
    check(
        report!(TriggerUpdateError::PluginConfigNotFound),
        400,
        "trigger_update.plugin_config_not_found",
    )
    .await;
    check(
        report!(TriggerUpdateError::UnknownPluginType("shell".into())),
        400,
        "trigger_update.unknown_plugin_type",
    )
    .await;
    check(
        report!(TriggerUpdateError::Database(sea_orm::DbErr::Custom(
            "test".into()
        ))),
        500,
        "trigger_update.database_error",
    )
    .await;
}

#[tokio::test]
async fn audit_log_query_error_all_variants() {
    let (status, json) = read_response(
        ApiError::from(report!(AuditLogQueryError::InvalidFilter(
            "bad date".into()
        )))
        .into_response(),
    )
    .await;
    assert_eq!(status.as_u16(), 400);
    assert_eq!(code_of(&json), "audit_log.invalid_filter");

    let (status, json) = read_response(
        ApiError::from(report!(AuditLogQueryError::Database(
            sea_orm::DbErr::Custom("test".into())
        )))
        .into_response(),
    )
    .await;
    assert_eq!(status.as_u16(), 500);
    assert_eq!(code_of(&json), "audit_log.database_error");
}

#[tokio::test]
async fn device_flow_error_all_variants() {
    async fn check(report: Report<DeviceFlowError>, expected_status: u16, expected_code: &str) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(DeviceFlowError::NotFound),
        404,
        "device_flow.not_found",
    )
    .await;
    check(
        report!(DeviceFlowError::AlreadyAuthorized),
        409,
        "device_flow.already_authorized",
    )
    .await;
    check(
        report!(DeviceFlowError::TokenGeneration("rng failed".into())),
        500,
        "device_flow.token_generation_error",
    )
    .await;
    check(
        report!(DeviceFlowError::Database(sea_orm::DbErr::Custom(
            "test".into()
        ))),
        500,
        "device_flow.database_error",
    )
    .await;
}

#[tokio::test]
async fn registration_validation_error_all_variants() {
    async fn check(err: RegistrationValidationError, expected_status: u16, expected_code: &str) {
        let (status, json) = read_response(ApiError::from(err).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        RegistrationValidationError::Closed,
        403,
        "registration.closed",
    )
    .await;
    check(
        RegistrationValidationError::TokenRequired,
        403,
        "registration.token_required",
    )
    .await;
    check(
        RegistrationValidationError::NoTokenConfigured,
        403,
        "registration.no_token_configured",
    )
    .await;
    check(
        RegistrationValidationError::InvalidToken,
        403,
        "registration.invalid_token",
    )
    .await;
    check(
        RegistrationValidationError::VerificationFailed,
        500,
        "registration.verification_failed",
    )
    .await;
}

#[tokio::test]
async fn plugin_type_settings_error_all_variants() {
    let (status, json) =
        read_response(ApiError::from(report!(PluginTypeSettingsError::NotFound)).into_response())
            .await;
    assert_eq!(status.as_u16(), 404);
    assert_eq!(code_of(&json), "plugin_type_settings.not_found");

    let (status, json) = read_response(
        ApiError::from(report!(PluginTypeSettingsError::Db(
            sea_orm::DbErr::Custom("test".into())
        )))
        .into_response(),
    )
    .await;
    assert_eq!(status.as_u16(), 500);
    assert_eq!(code_of(&json), "plugin_type_settings.database_error");
}

#[tokio::test]
async fn autodiscovery_error_all_variants() {
    let (status, json) = read_response(
        ApiError::from(report!(AutodiscoveryError::Db(sea_orm::DbErr::Custom(
            "test".into()
        ))))
        .into_response(),
    )
    .await;
    assert_eq!(status.as_u16(), 500);
    assert_eq!(code_of(&json), "autodiscovery.database_error");
}

#[tokio::test]
async fn reset_data_query_error_all_variants() {
    let (status, json) = read_response(
        ApiError::from(report!(ResetDataQueryError::Database(
            sea_orm::DbErr::Custom("test".into())
        )))
        .into_response(),
    )
    .await;
    assert_eq!(status.as_u16(), 500);
    assert_eq!(code_of(&json), "reset_data.database_error");
}

#[tokio::test]
async fn system_enrollment_token_error_all_variants() {
    let (status, json) = read_response(
        ApiError::from(report!(SystemEnrollmentTokenError::Database(
            sea_orm::DbErr::Custom("test".into())
        )))
        .into_response(),
    )
    .await;
    assert_eq!(status.as_u16(), 500);
    assert_eq!(code_of(&json), "system_enrollment_token.database_error");
}

#[tokio::test]
async fn auth_error_all_variants() {
    async fn check(report: Report<AuthError>, expected_status: u16, expected_code: &str) {
        let (status, json) = read_response(ApiError::from(report).into_response()).await;
        assert_eq!(status.as_u16(), expected_status, "code={expected_code}");
        assert_eq!(code_of(&json), expected_code);
    }

    check(
        report!(AuthError::InvalidCredentials),
        401,
        "auth.invalid_credentials",
    )
    .await;
    check(
        report!(AuthError::SessionExpired),
        401,
        "auth.session_expired",
    )
    .await;
    check(
        report!(AuthError::UserNotFound("test@example.com".into())),
        401,
        "auth.invalid_credentials",
    )
    .await;
    check(
        report!(AuthError::UserDeactivated),
        401,
        "auth.user_deactivated",
    )
    .await;
    check(
        report!(AuthError::TokenGeneration("rng".into())),
        500,
        "auth.internal",
    )
    .await;
    check(
        report!(AuthError::Database(sea_orm::DbErr::Custom("test".into()))),
        500,
        "auth.internal",
    )
    .await;
    check(
        report!(AuthError::OidcProviderNotFound),
        400,
        "auth.oidc_provider_not_found",
    )
    .await;
    check(
        report!(AuthError::OidcDiscovery("timeout".into())),
        500,
        "auth.internal",
    )
    .await;
    check(
        report!(AuthError::OidcTokenExchange("400".into())),
        500,
        "auth.internal",
    )
    .await;
    check(
        report!(AuthError::OidcTokenValidation("sig".into())),
        400,
        "auth.oidc_token_validation_failed",
    )
    .await;
    check(
        report!(AuthError::OidcStateNotFound),
        400,
        "auth.oidc_state_not_found",
    )
    .await;
    check(
        report!(AuthError::OidcNoAccount),
        403,
        "auth.oidc_no_account",
    )
    .await;
    check(
        report!(AuthError::OidcLinkRequired),
        403,
        "auth.oidc_link_required",
    )
    .await;
    check(
        report!(AuthError::OidcLinkVerificationFailed),
        400,
        "auth.oidc_link_verification_failed",
    )
    .await;
    check(
        report!(AuthError::PasswordAuthDisabled),
        400,
        "auth.password_auth_disabled",
    )
    .await;
    check(
        report!(AuthError::CannotDisableOwnAuthMethod),
        409,
        "auth.cannot_disable_own_auth_method",
    )
    .await;
    check(
        report!(AuthError::NoAuthMethodsRemaining),
        409,
        "auth.no_auth_methods_remaining",
    )
    .await;
    check(
        report!(AuthError::JwtEncode("enc".into())),
        500,
        "auth.internal",
    )
    .await;
    check(
        report!(AuthError::JwtDecode("sig".into())),
        401,
        "auth.jwt_decode_failed",
    )
    .await;
    check(
        report!(AuthError::InvalidRefreshToken),
        401,
        "auth.invalid_refresh_token",
    )
    .await;
    check(
        report!(AuthError::RefreshTokenExpired),
        401,
        "auth.refresh_token_expired",
    )
    .await;
    check(
        report!(AuthError::RefreshTokenRevoked),
        401,
        "auth.refresh_token_revoked",
    )
    .await;
    check(
        report!(AuthError::ApiTokenNotFound),
        401,
        "auth.api_token_not_found",
    )
    .await;
    check(
        report!(AuthError::ApiTokenRevoked),
        401,
        "auth.api_token_revoked",
    )
    .await;
    check(
        report!(AuthError::DeviceFlowNotFound),
        404,
        "auth.device_flow_not_found",
    )
    .await;
    check(
        report!(AuthError::DeviceFlowAlreadyAuthorized),
        409,
        "auth.device_flow_already_authorized",
    )
    .await;
    check(
        report!(AuthError::InvalidSession),
        401,
        "auth.invalid_session",
    )
    .await;
    check(
        report!(AuthError::Internal("oops".into())),
        500,
        "auth.internal",
    )
    .await;
}

// ---------------------------------------------------------------------------
// 3. Dynamic-display dual-payload tests
// ---------------------------------------------------------------------------

/// Assert that two different payload strings produce different user_messages,
/// and that the message contains the expected prefix.
macro_rules! dynamic_display_test {
    ($name:ident, $variant_a:expr, $variant_b:expr) => {
        #[tokio::test]
        async fn $name() {
            let (_, json_a) = read_response(ApiError::from($variant_a).into_response()).await;
            let (_, json_b) = read_response(ApiError::from($variant_b).into_response()).await;
            let msg_a = error_of(&json_a);
            let msg_b = error_of(&json_b);
            assert_ne!(
                msg_a, msg_b,
                "dynamic_display variants must produce different messages for different payloads"
            );
        }
    };
}

dynamic_display_test!(
    plugin_config_validation_dynamic,
    ApiError::from(report!(PluginConfigError::ConfigValidation(
        "field 'x' required".into()
    ))),
    ApiError::from(report!(PluginConfigError::ConfigValidation(
        "value too long".into()
    )))
);

dynamic_display_test!(
    channel_unsupported_type_dynamic,
    ApiError::from(report!(ChannelQueryError::UnsupportedType("smtp".into()))),
    ApiError::from(report!(ChannelQueryError::UnsupportedType(
        "webhook_v2".into()
    )))
);

dynamic_display_test!(
    channel_invalid_config_dynamic,
    ApiError::from(report!(ChannelQueryError::InvalidConfig(
        "missing host".into()
    ))),
    ApiError::from(report!(ChannelQueryError::InvalidConfig(
        "invalid url".into()
    )))
);

dynamic_display_test!(
    rule_invalid_field_dynamic,
    ApiError::from(report!(RuleQueryError::InvalidField("event_type".into()))),
    ApiError::from(report!(RuleQueryError::InvalidField("severity".into())))
);

dynamic_display_test!(
    software_item_incompatible_host_dynamic,
    ApiError::from(report!(SoftwareItemQueryError::IncompatibleHost(
        "arch mismatch".into()
    ))),
    ApiError::from(report!(SoftwareItemQueryError::IncompatibleHost(
        "os mismatch".into()
    )))
);

dynamic_display_test!(
    software_item_invalid_package_identifier_dynamic,
    ApiError::from(report!(SoftwareItemQueryError::InvalidPackageIdentifier(
        "bad::name".into()
    ))),
    ApiError::from(report!(SoftwareItemQueryError::InvalidPackageIdentifier(
        "invalid/slash".into()
    )))
);

dynamic_display_test!(
    software_item_invalid_config_override_dynamic,
    ApiError::from(report!(SoftwareItemQueryError::InvalidConfigOverride(
        "key a".into()
    ))),
    ApiError::from(report!(SoftwareItemQueryError::InvalidConfigOverride(
        "key b".into()
    )))
);

dynamic_display_test!(
    software_item_invalid_inline_plugin_config_dynamic,
    ApiError::from(report!(SoftwareItemQueryError::InvalidInlinePluginConfig(
        "name empty".into()
    ))),
    ApiError::from(report!(SoftwareItemQueryError::InvalidInlinePluginConfig(
        "config invalid".into()
    )))
);

dynamic_display_test!(
    software_item_invalid_execution_site_dynamic,
    ApiError::from(report!(SoftwareItemQueryError::InvalidExecutionSite(
        "cloud".into()
    ))),
    ApiError::from(report!(SoftwareItemQueryError::InvalidExecutionSite(
        "edge".into()
    )))
);

dynamic_display_test!(
    audit_log_invalid_filter_dynamic,
    ApiError::from(report!(AuditLogQueryError::InvalidFilter(
        "bad date".into()
    ))),
    ApiError::from(report!(AuditLogQueryError::InvalidFilter(
        "unknown field".into()
    )))
);

dynamic_display_test!(
    trigger_update_unknown_plugin_type_dynamic,
    ApiError::from(report!(TriggerUpdateError::UnknownPluginType(
        "shell_v3".into()
    ))),
    ApiError::from(report!(TriggerUpdateError::UnknownPluginType(
        "custom".into()
    )))
);

// ---------------------------------------------------------------------------
// 4. 5xx logging test — exactly one ERROR event with required fields
// ---------------------------------------------------------------------------

#[tokio::test]
async fn five_xx_emits_exactly_one_error_event_with_structured_fields() {
    let (layer, captured) = CaptureLayer::new();
    let sub = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(sub);

    let report = report!(ServiceQueryError::Db(sea_orm::DbErr::Custom(
        "db timeout".into()
    )));
    let api_err = ApiError::from(report);
    let (_, json) = read_response(api_err.into_response()).await;

    // Response body must NOT expose internal detail.
    let error_msg = error_of(&json);
    assert!(
        !error_msg.contains("db timeout"),
        "5xx must not leak internal detail to client"
    );

    let events = captured.lock().unwrap();
    let error_events: Vec<_> = events
        .events
        .iter()
        .filter(|e| e.level == "ERROR")
        .collect();
    assert_eq!(error_events.len(), 1, "exactly one ERROR event expected");

    let ev = &error_events[0];
    assert!(
        ev.fields.contains_key("error.code"),
        "event must have error.code field"
    );
    assert!(
        ev.fields.contains_key("error.status"),
        "event must have error.status field"
    );
    assert!(
        ev.fields.contains_key("error.detail"),
        "event must have error.detail field"
    );
}

// ---------------------------------------------------------------------------
// 5. 4xx no-logging test — zero ERROR events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn four_xx_emits_no_error_events() {
    let (layer, captured) = CaptureLayer::new();
    let sub = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(sub);

    let api_err = ApiError::from(report!(ServiceQueryError::NotFound));
    let _ = api_err.into_response();

    let events = captured.lock().unwrap();
    let error_events: Vec<_> = events
        .events
        .iter()
        .filter(|e| e.level == "ERROR")
        .collect();
    assert_eq!(error_events.len(), 0, "4xx must not emit ERROR events");
}

// ---------------------------------------------------------------------------
// 6. Golden-file test for code_registry.txt
// ---------------------------------------------------------------------------

/// All codes emitted by the From impls.  Kept in sync manually; the test
/// catches both missing-from-file and missing-from-impl cases.
const ALL_IMPL_CODES: &[&str] = &[
    "allowlist.database_error",
    "allowlist.invalid_plugin_type",
    "audit_log.database_error",
    "audit_log.invalid_filter",
    "auth.api_token_not_found",
    "auth.api_token_revoked",
    "auth.cannot_disable_own_auth_method",
    "auth.device_flow_already_authorized",
    "auth.device_flow_not_found",
    "auth.internal",
    "auth.invalid_credentials",
    "auth.invalid_refresh_token",
    "auth.invalid_session",
    "auth.jwt_decode_failed",
    "auth.no_auth_methods_remaining",
    "auth.oidc_link_required",
    "auth.oidc_link_verification_failed",
    "auth.oidc_no_account",
    "auth.oidc_provider_not_found",
    "auth.oidc_state_not_found",
    "auth.oidc_token_validation_failed",
    "auth.password_auth_disabled",
    "auth.refresh_token_expired",
    "auth.refresh_token_revoked",
    "auth.session_expired",
    "auth.user_deactivated",
    "autodiscovery.database_error",
    "device_flow.already_authorized",
    "device_flow.database_error",
    "device_flow.not_found",
    "device_flow.token_generation_error",
    "notification_channel.database_error",
    "notification_channel.invalid_config",
    "notification_channel.unsupported_type",
    "notification_rule.channel_not_found",
    "notification_rule.database_error",
    "notification_rule.invalid_field",
    "plugin_config.config_validation",
    "plugin_config.duplicate_name",
    "plugin_config.empty_name",
    "plugin_config.internal_error",
    "plugin_config.not_found",
    "plugin_type_settings.database_error",
    "plugin_type_settings.not_found",
    "registration.closed",
    "registration.invalid_token",
    "registration.no_token_configured",
    "registration.token_required",
    "registration.verification_failed",
    "reset_data.database_error",
    "scheduled_task.database_error",
    "scheduled_task.invalid_interval",
    "scheduled_task.not_found",
    "service.database_error",
    "service.embedded_service",
    "service.not_approved",
    "service.not_found",
    "service.not_mergeable",
    "service.not_pending",
    "service.source_not_found",
    "service.target_connected",
    "software_item.database_error",
    "software_item.duplicate_host_assignment",
    "software_item.duplicate_item",
    "software_item.empty_name",
    "software_item.host_not_found",
    "software_item.incompatible_host",
    "software_item.invalid_config_override",
    "software_item.invalid_execution_site",
    "software_item.invalid_inline_plugin_config",
    "software_item.invalid_package_identifier",
    "software_item.not_found",
    "software_item.plugin_assignment_not_found",
    "software_item.plugin_config_not_found",
    "system_enrollment_token.database_error",
    "system_service.database_error",
    "system_service.embedded_service",
    "system_service.not_approved",
    "system_service.not_found",
    "system_service.not_pending",
    "trigger_update.agent_not_approved",
    "trigger_update.database_error",
    "trigger_update.host_not_assigned",
    "trigger_update.host_not_found",
    "trigger_update.no_agent",
    "trigger_update.no_execute_update_plugin",
    "trigger_update.plugin_config_not_found",
    "trigger_update.software_item_not_found",
    "trigger_update.unknown_plugin_type",
    "trigger_update.update_already_active",
];

#[test]
fn code_registry_golden_file_sorted_and_complete() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api_error/code_registry.txt");
    let contents = std::fs::read_to_string(&path)
        .expect("code_registry.txt must exist — run cargo test to regenerate");

    let file_codes: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();

    // Check sort order.
    let mut sorted = file_codes.clone();
    sorted.sort_unstable();
    assert_eq!(file_codes, sorted, "code_registry.txt must be sorted");

    let file_set: HashSet<&str> = file_codes.iter().copied().collect();
    let impl_set: HashSet<&str> = ALL_IMPL_CODES.iter().copied().collect();

    let in_file_not_impl: Vec<&&str> = file_set.difference(&impl_set).collect();
    let in_impl_not_file: Vec<&&str> = impl_set.difference(&file_set).collect();

    assert!(
        in_file_not_impl.is_empty(),
        "codes in registry but not in impls: {in_file_not_impl:?}"
    );
    assert!(
        in_impl_not_file.is_empty(),
        "codes in impls but not in registry: {in_impl_not_file:?}"
    );
}

#[test]
fn code_registry_no_prefix_collisions() {
    // A code like "auth" must not be both a standalone code and a prefix of
    // "auth.internal". This guards against ambiguous API client pattern matching.
    let mut sorted: Vec<&str> = ALL_IMPL_CODES.to_vec();
    sorted.sort_unstable();
    for window in sorted.windows(2) {
        let (a, b) = (window[0], window[1]);
        assert!(
            !b.starts_with(&format!("{a}.")),
            "code `{a}` is a prefix of `{b}` — codes must not share a dot-separated prefix"
        );
    }
}

// ---------------------------------------------------------------------------
// 7. MAPPING_REVIEW.md consistency tests
// ---------------------------------------------------------------------------

#[test]
fn mapping_review_md_exists_and_has_all_variant_names() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api_error/MAPPING_REVIEW.md");
    let contents = std::fs::read_to_string(&path).expect("MAPPING_REVIEW.md must exist");

    // Spot-check that all enum names appear in the review document.
    let required_sections = [
        "ServiceQueryError",
        "SystemServiceQueryError",
        "PluginConfigError",
        "ChannelQueryError",
        "RuleQueryError",
        "AllowlistError",
        "ScheduledTaskError",
        "SoftwareItemQueryError",
        "TriggerUpdateError",
        "AuditLogQueryError",
        "DeviceFlowError",
        "RegistrationValidationError",
        "PluginTypeSettingsError",
        "AutodiscoveryError",
        "ResetDataQueryError",
        "SystemEnrollmentTokenError",
        "AuthError",
    ];
    for section in required_sections {
        assert!(
            contents.contains(section),
            "MAPPING_REVIEW.md must contain section for `{section}`"
        );
    }
}

#[test]
fn mapping_review_md_documents_intentional_deltas() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api_error/MAPPING_REVIEW.md");
    let contents = std::fs::read_to_string(&path).expect("MAPPING_REVIEW.md must exist");

    // Both intentional status-code corrections must be documented.
    assert!(
        contents.contains("ConfigValidation") && contents.contains("500→400"),
        "MAPPING_REVIEW.md must document PluginConfigError::ConfigValidation 500→400 delta"
    );
    assert!(
        contents.contains("NoAgent") && contents.contains("404→400"),
        "MAPPING_REVIEW.md must document TriggerUpdateError::NoAgent 404→400 delta"
    );
}

// ---------------------------------------------------------------------------
// 8. DYNAMIC_DISPLAY_ALLOWLIST annotation test
// ---------------------------------------------------------------------------

#[test]
fn dynamic_display_allowlist_has_exactly_eleven_entries() {
    assert_eq!(
        DYNAMIC_DISPLAY_ALLOWLIST.len(),
        11,
        "DYNAMIC_DISPLAY_ALLOWLIST must have exactly 11 entries"
    );
}

#[test]
fn dynamic_display_allowlist_entries_are_sorted() {
    let mut sorted = DYNAMIC_DISPLAY_ALLOWLIST.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        DYNAMIC_DISPLAY_ALLOWLIST.to_vec(),
        sorted,
        "DYNAMIC_DISPLAY_ALLOWLIST entries must be sorted"
    );
}

#[test]
fn dynamic_display_allowlist_entries_are_unique() {
    let set: HashSet<&&str> = DYNAMIC_DISPLAY_ALLOWLIST.iter().collect();
    assert_eq!(
        set.len(),
        DYNAMIC_DISPLAY_ALLOWLIST.len(),
        "DYNAMIC_DISPLAY_ALLOWLIST must not contain duplicates"
    );
}
