//! Unified capability-gated WebSocket handler for all service types.
//!
//! This module replaces the three separate handlers (`agent_ws`, `mqtt_ws`,
//! `ssh_agent_ws`) with a single pair of handler functions that dispatch
//! messages based on the service's persisted capability set.
//!
//! ## Background message processing
//!
//! Heavy message processing (DB queries, notifications, etc.) is offloaded
//! to a [`MessageProcessor`] task spawned per connection. The main loop
//! reads WebSocket frames, handles lightweight inline operations (Ping/Pong,
//! Disconnecting, Unknown, Close, rate limiting), and forwards everything
//! else to the processor via a bounded MPSC channel.
//!
//! The processor handles messages sequentially (preserving ordering) and
//! sends [`ProcessorResponse`](shared_types::ProcessorResponse) values back
//! to the main loop, which serializes and writes replies to the WebSocket
//! sink with `out_seq` staying in the main loop.
//!
//! # Public API
//!
//! - [`handle_authenticated_loop`] -- post-certificate operational loop.
//! - [`handle_enrolled_loop`] -- pre-certificate enrollment loop.
//! - [`trigger_discovery_for_agent_host`] -- send `DiscoverSoftware` to an
//!   agent for a specific host (also used by `hosts.rs`).

mod cert;
mod credentials;
mod discovery;
pub(super) mod messages;
mod reconnect;
mod renewal;
mod service_config;
mod shared_types;
mod update_tracking;
mod updates;
mod workload;

use cert::{
    ApprovalPollResult, CertificateResult, handle_request_certificate, poll_approval_status,
};
use credentials::deliver_service_credentials;
pub(crate) use discovery::trigger_discovery_for_agent_host;
use shared_types::{ProcessorAction, ProcessorResponse, load_linked_host_ids};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use thiserror::Error;

use rootcause::prelude::*;
use sea_orm::EntityTrait;

use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_wire::limits::{MAX_LONG_STRING_LEN, MAX_SHORT_STRING_LEN, WireValidate};
use uptrakit_wire::report_tracker::ReportTracker;
use uptrakit_wire::{
    AuditEventPayload, Capability, CloseReason, ControllerMessage, ErrorCode, ErrorPayload,
    HostConnectivityUpdate, IncomingSeq, OutgoingSeq, PingPayload, RegisterPayload,
    ReportPagination, ServiceMessage, surfaces,
};

use super::protocol::{
    AuthenticatedContext, CertIdentity, MessageRateLimiter, WS_MESSAGE_RATE_LIMIT,
    WS_MESSAGE_RATE_WINDOW, close_with_reason, deserialize_service_msg, record_service_activity,
    record_system_service_activity, send_pong, serialize_controller_msg,
};
use crate::AppState;
use uptrakit_wire::service_profile::parse_capabilities;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum size of the `update_history.output` column (50 MB).
///
/// Docker image pulls generate very verbose progress output (tens of megabytes
/// for large images). This cap covers virtually all real-world update outputs
/// while preventing unbounded DB growth.
///
/// When the cap is first exceeded, a visible system output line is emitted
/// into the stream and the `output_truncated` flag is set on the history
/// record so the UI can display a persistent warning banner.
const MAX_UPDATE_OUTPUT_BYTES: usize = 52_428_800;

/// Interval between approval-status DB polls in enrolled loops.
const APPROVAL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Maximum time to wait for a WebSocket write (`sink.send()`) to complete.
///
/// If a service stops reading from the WebSocket, the OS TCP send buffer fills
/// and `sink.send()` blocks indefinitely. This timeout bounds the hang so that
/// the handler loop can break and clean up the connection. Kept deliberately
/// shorter than the agent-side `SEND_TIMEOUT` (30 s) so the controller detects
/// the stuck connection first.
const WS_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Maximum consecutive unknown messages before closing the connection.
///
/// Prevents a misbehaving or fuzzing client from keeping a connection alive
/// indefinitely by sending only garbage message types. Resets on any known
/// message.
const MAX_CONSECUTIVE_UNKNOWN_MESSAGES: u32 = 10;
const MQTT_SERVICE_APP_NAME: &str = "uptrakit-mqtt";

fn system_service_tenant_binding(
    service_app_name: Option<&str>,
    default_tenant_id: uuid::Uuid,
) -> Option<uuid::Uuid> {
    (service_app_name == Some(MQTT_SERVICE_APP_NAME)).then_some(default_tenant_id)
}

fn is_valid_service_config_scope(
    service_tenant_id: Option<uuid::Uuid>,
    payload_tenant_id: Option<uuid::Uuid>,
) -> bool {
    match service_tenant_id {
        Some(bound_tenant_id) => payload_tenant_id == Some(bound_tenant_id),
        None => true,
    }
}

fn surface_action_target_display(
    surface_id: &surfaces::SurfaceId,
    interaction_id: &surfaces::InteractionId,
) -> String {
    format!("{surface_id}/{interaction_id}")
}

fn surface_provider_kind_name(provider_kind: surfaces::ProviderKind) -> &'static str {
    match provider_kind {
        surfaces::ProviderKind::Service => "service",
        surfaces::ProviderKind::BuiltIn => "built_in",
        surfaces::ProviderKind::Plugin => "plugin",
        _ => {
            tracing::warn!(
                ?provider_kind,
                "unknown ProviderKind variant; defaulting to 'plugin'"
            );
            "plugin"
        }
    }
}

fn truncate_surface_registration_audit_value(value: &str) -> String {
    const MAX_BYTES: usize = 128;

    if value.len() <= MAX_BYTES {
        return value.to_string();
    }

    let mut boundary = MAX_BYTES;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }

    value[..boundary].to_string()
}

fn classify_surface_registration_validation_error(
    error: &uptrakit_wire::limits::WireValidationError,
) -> &'static str {
    match error.field {
        "effective_tenant_binding.tenant_id" => "invalid_tenant_binding",
        "provider.provider_id" => "invalid_provider_id",
        _ => "invalid_request",
    }
}

fn surface_registration_rejection_reason_code(
    code: &crate::surface_registry::SurfaceProviderRejectionCode,
) -> &'static str {
    match code {
        crate::surface_registry::SurfaceProviderRejectionCode::UnsupportedGeneration => {
            "unsupported_generation"
        }
        crate::surface_registry::SurfaceProviderRejectionCode::MissingCapability => {
            "missing_capability"
        }
        crate::surface_registry::SurfaceProviderRejectionCode::InvalidSlot => "invalid_slot",
        crate::surface_registry::SurfaceProviderRejectionCode::InvalidTransport => {
            "invalid_transport"
        }
        crate::surface_registry::SurfaceProviderRejectionCode::SchemaOrLimitFailure => {
            "schema_or_limit_failure"
        }
    }
}

fn classify_surface_registration_error_for_audit(
    error: &crate::surface_registry::SurfaceRegistryError,
) -> &'static str {
    match error {
        crate::surface_registry::SurfaceRegistryError::ProviderRejected(rejection) => rejection
            .reasons
            .first()
            .map(|reason| surface_registration_rejection_reason_code(&reason.code))
            .unwrap_or("provider_rejected"),
        crate::surface_registry::SurfaceRegistryError::ProviderConflict(_) => "provider_conflict",
    }
}

struct ServiceAuditCtx<'a> {
    state: &'a AppState,
    service_id: uuid::Uuid,
    service_app_name: Option<&'a str>,
}

fn emit_surface_registration_audit_event(
    ctx: &ServiceAuditCtx<'_>,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    payload: &uptrakit_wire::surfaces::SurfaceRegistration,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
) {
    let provider_id = truncate_surface_registration_audit_value(&payload.provider.provider_id);
    let mut details = serde_json::Map::from_iter([
        ("provider_id".to_string(), serde_json::json!(provider_id)),
        (
            "provider_kind".to_string(),
            serde_json::json!(surface_provider_kind_name(payload.provider.provider_kind)),
        ),
        (
            "framework_generation".to_string(),
            serde_json::json!(format!(
                "{}.{}",
                payload.framework_generation.major, payload.framework_generation.minor
            )),
        ),
        (
            "capability_count".to_string(),
            serde_json::json!(payload.capabilities.0.len()),
        ),
        (
            "surface_count".to_string(),
            serde_json::json!(payload.surfaces.len()),
        ),
    ]);
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    let builder = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
    );
    let builder = if is_system {
        builder.system_scope()
    } else if let Some(tenant_id) = service_tenant_id {
        builder.tenant_scope(tenant_id)
    } else {
        builder.system_scope()
    };

    let entry = builder
        .actor_service(ctx.service_id)
        .actor_display_opt(ctx.service_app_name.map(str::to_string))
        .target_opt(
            Some("surface_provider".to_string()),
            Some(provider_id.clone()),
            Some(provider_id),
        )
        .outcome(outcome)
        .details(serde_json::Value::Object(details))
        .build();

    match entry {
        Ok(entry) => ctx.state.audit_emitter.emit_best_effort(entry),
        Err(error) => tracing::warn!(
            service_id = %ctx.service_id,
            provider_id = %payload.provider.provider_id,
            outcome = %outcome,
            error = %error,
            "failed to build surface registration audit entry"
        ),
    }
}

async fn emit_surface_action_scope_denied_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
    service_app_name: Option<&str>,
    service_tenant_id: uuid::Uuid,
    payload: &uptrakit_wire::surfaces::SurfaceActionRequest,
) {
    let entry = match uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .tenant_scope(service_tenant_id)
    .actor_service(service_id)
    .actor_display_opt(service_app_name.map(str::to_string))
    .target_opt(
        Some("surface_action".to_string()),
        None,
        Some(surface_action_target_display(
            &payload.surface_id,
            &payload.interaction_id,
        )),
    )
    .outcome(uptrakit_audit_log::AuditOutcome::Denied)
    .request_id_opt(Some(payload.request_id.to_string()))
    .details(serde_json::json!({
        "service_app_name": service_app_name,
        "surface_id": payload.surface_id,
        "interaction_id": payload.interaction_id,
        "target_provider_id": payload.target_provider_id,
        "service_tenant_id": service_tenant_id,
        "requested_tenant_id": payload.tenant_id,
        "reason_code": "outside_tenant_binding",
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(error) => {
            tracing::warn!(
                %service_id,
                request_id = %payload.request_id,
                error = %error,
                "failed to build surface action scope denial audit entry"
            );
            return;
        }
    };

    state.audit_emitter.emit_best_effort(entry);
}

fn emit_surface_action_invoke_audit_event(
    ctx: &ServiceAuditCtx<'_>,
    tenant_id: uuid::Uuid,
    payload: &uptrakit_wire::surfaces::SurfaceActionRequest,
    resolved: Option<&crate::surface_registry::ResolvedSurfaceAction>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
) {
    let mut details = serde_json::Map::from_iter([
        (
            "surface_id".to_string(),
            serde_json::json!(payload.surface_id.as_str()),
        ),
        (
            "interaction_id".to_string(),
            serde_json::json!(payload.interaction_id.as_str()),
        ),
        (
            "target_provider_id".to_string(),
            serde_json::json!(
                resolved
                    .map(|value| value.provider_id.as_str())
                    .or(payload.target_provider_id.as_deref())
            ),
        ),
    ]);
    if let Some(resolved) = resolved {
        details.insert(
            "provider_kind".to_string(),
            serde_json::json!(surface_provider_kind_name(resolved.provider_kind)),
        );
        if let Some(provider_service_app_name) = resolved.service_app_name.as_deref() {
            details.insert(
                "provider_service_app_name".to_string(),
                serde_json::json!(provider_service_app_name),
            );
        }
    }
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    let entry = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .tenant_scope(tenant_id)
    .actor_service(ctx.service_id)
    .actor_display_opt(ctx.service_app_name.map(str::to_string))
    .target_opt(
        Some("surface_action".to_string()),
        None,
        Some(surface_action_target_display(
            &payload.surface_id,
            &payload.interaction_id,
        )),
    )
    .outcome(outcome)
    .request_id_opt(Some(payload.request_id.to_string()))
    .details(serde_json::Value::Object(details))
    .build();

    match entry {
        Ok(entry) => ctx.state.audit_emitter.emit_best_effort(entry),
        Err(error) => tracing::warn!(
            service_id = %ctx.service_id,
            request_id = %payload.request_id,
            surface_id = %payload.surface_id,
            interaction_id = %payload.interaction_id,
            outcome = %outcome,
            error = %error,
            "failed to build surface action invoke audit entry"
        ),
    }
}

fn classify_surface_action_response_for_audit(
    response: &uptrakit_wire::surfaces::SurfaceActionResponse,
) -> (uptrakit_audit_log::AuditOutcome, Option<&'static str>) {
    if response.success {
        return (uptrakit_audit_log::AuditOutcome::Success, None);
    }

    let Some(error) = response.error.as_ref() else {
        return (
            uptrakit_audit_log::AuditOutcome::Failed,
            Some("action_failed"),
        );
    };

    let outcome = match error.code {
        uptrakit_wire::surfaces::SurfaceActionErrorCode::PermissionDenied
        | uptrakit_wire::surfaces::SurfaceActionErrorCode::DuplicateRequest => {
            uptrakit_audit_log::AuditOutcome::Denied
        }
        uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest
        | uptrakit_wire::surfaces::SurfaceActionErrorCode::SchemaValidationFailed => {
            uptrakit_audit_log::AuditOutcome::ValidationFailed
        }
        uptrakit_wire::surfaces::SurfaceActionErrorCode::UnsupportedCapability
        | uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable
        | uptrakit_wire::surfaces::SurfaceActionErrorCode::Timeout
        | uptrakit_wire::surfaces::SurfaceActionErrorCode::InternalError => {
            uptrakit_audit_log::AuditOutcome::Failed
        }
    };

    (outcome, Some(action_error_code(&error.code)))
}

fn classify_surface_proxy_error_for_audit(
    error: &crate::surface_proxy::SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match error {
        crate::surface_proxy::SurfaceProxyError::NoProvider => {
            (uptrakit_audit_log::AuditOutcome::Failed, "no_provider")
        }
        crate::surface_proxy::SurfaceProxyError::TargetProviderRequired => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "target_provider_required",
        ),
        crate::surface_proxy::SurfaceProxyError::InvalidProvider(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_provider",
        ),
        crate::surface_proxy::SurfaceProxyError::InteractionNotFound => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "interaction_not_found",
        ),
        crate::surface_proxy::SurfaceProxyError::PermissionDenied(_) => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "permission_denied",
        ),
        crate::surface_proxy::SurfaceProxyError::Conflict { code, .. } => {
            (uptrakit_audit_log::AuditOutcome::Denied, code)
        }
        crate::surface_proxy::SurfaceProxyError::SchemaValidationFailed(_)
        | crate::surface_proxy::SurfaceProxyError::SensitiveFieldRejected(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_request",
        ),
        crate::surface_proxy::SurfaceProxyError::DuplicateRequest => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "duplicate_request",
        ),
        crate::surface_proxy::SurfaceProxyError::RateLimited => {
            (uptrakit_audit_log::AuditOutcome::Denied, "rate_limited")
        }
        crate::surface_proxy::SurfaceProxyError::ServiceDisconnected
        | crate::surface_proxy::SurfaceProxyError::SendFailed => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "provider_unavailable",
        ),
        crate::surface_proxy::SurfaceProxyError::Timeout => {
            (uptrakit_audit_log::AuditOutcome::Failed, "timeout")
        }
    }
}

fn classify_surface_lookup_error_for_audit(
    error: &crate::surface_registry::SurfaceRegistryLookupError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match error {
        crate::surface_registry::SurfaceRegistryLookupError::SurfaceNotFound => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "surface_not_found",
        ),
        crate::surface_registry::SurfaceRegistryLookupError::InteractionNotFound => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "interaction_not_found",
        ),
        crate::surface_registry::SurfaceRegistryLookupError::TargetProviderRequired => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "target_provider_required",
        ),
        crate::surface_registry::SurfaceRegistryLookupError::InvalidProvider(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_provider",
        ),
        crate::surface_registry::SurfaceRegistryLookupError::NoTenantCompatibleProvider => {
            (uptrakit_audit_log::AuditOutcome::Failed, "no_provider")
        }
    }
}

fn classify_surface_action_request_validation_error(
    error: &uptrakit_wire::limits::WireValidationError,
) -> &'static str {
    if error.field == "tenant_id" {
        "invalid_tenant_id"
    } else {
        "invalid_request"
    }
}

fn resolve_surface_action_audit_tenant_id(
    service_tenant_id: Option<uuid::Uuid>,
    payload: &uptrakit_wire::surfaces::SurfaceActionRequest,
) -> Option<uuid::Uuid> {
    service_tenant_id.or_else(|| uuid::Uuid::parse_str(&payload.tenant_id).ok())
}

/// Send a serialized WebSocket message with a timeout, returning `true`
/// on success and `false` if the write failed or timed out.
async fn send_ws_with_timeout(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    json: String,
    service_id: uuid::Uuid,
) -> bool {
    match tokio::time::timeout(WS_WRITE_TIMEOUT, sink.send(Message::Text(json.into()))).await {
        Ok(Ok(())) => true,
        Ok(Err(_)) => false,
        Err(_) => {
            tracing::warn!(
                %service_id,
                "WebSocket write timed out after {}s, dropping connection",
                WS_WRITE_TIMEOUT.as_secs(),
            );
            false
        }
    }
}

/// Bounded channel capacity for messages forwarded to the processor.
const PROCESSOR_CHANNEL_CAPACITY: usize = 32;

/// Bounded channel capacity for responses from the processor.
const RESPONSE_CHANNEL_CAPACITY: usize = 32;
const SYSTEM_SERVICE_AUDIT_ACTIONS: &[uptrakit_audit_log::RegisteredAuditAction] =
    &[uptrakit_audit_log::AuditActionType::SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP];
const TENANT_SERVICE_AUDIT_ACTIONS: &[uptrakit_audit_log::RegisteredAuditAction] = &[
    uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
    uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STARTED,
    uptrakit_audit_log::AuditActionType::SOFTWARE_BATCH_UPDATE_STARTED,
    uptrakit_audit_log::AuditActionType::HOST_UPDATE,
];
const SERVICE_BOUND_AUDIT_ACTIONS: &[uptrakit_audit_log::RegisteredAuditAction] = &[
    uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_ISSUE,
    uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
    uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_UPDATE_GATE,
    uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_MACHINE_ID_VALIDATE,
    uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_UPDATE_FREEZE_APPLY,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuditEventScope {
    TenantOnly,
    ServiceBound,
    SystemOnly,
}

fn audit_event_scope(action_type: &uptrakit_audit_log::AuditActionType) -> Option<AuditEventScope> {
    if TENANT_SERVICE_AUDIT_ACTIONS
        .iter()
        .any(|registered| registered.as_str() == action_type.as_str())
    {
        Some(AuditEventScope::TenantOnly)
    } else if SERVICE_BOUND_AUDIT_ACTIONS
        .iter()
        .any(|registered| registered.as_str() == action_type.as_str())
    {
        Some(AuditEventScope::ServiceBound)
    } else if SYSTEM_SERVICE_AUDIT_ACTIONS
        .iter()
        .any(|registered| registered.as_str() == action_type.as_str())
    {
        Some(AuditEventScope::SystemOnly)
    } else {
        None
    }
}

fn validate_audit_event_payload(
    payload: &AuditEventPayload,
) -> Result<
    (
        uptrakit_audit_log::AuditActionType,
        uptrakit_audit_log::AuditOutcome,
        AuditEventScope,
        Option<serde_json::Value>,
    ),
    String,
> {
    if payload.action_type.is_empty() {
        return Err("action_type must not be empty".to_string());
    }
    if payload.action_type.len() > MAX_SHORT_STRING_LEN {
        return Err(format!("action_type exceeds {MAX_SHORT_STRING_LEN} bytes"));
    }
    let action_type = payload
        .action_type
        .parse::<uptrakit_audit_log::AuditActionType>()
        .map_err(|error| error.to_string())?;
    let scope = audit_event_scope(&action_type)
        .ok_or_else(|| format!("unsupported audit action_type: {}", action_type.as_str()))?;
    if payload.outcome.is_empty() {
        return Err("outcome must not be empty".to_string());
    }
    if payload.outcome.len() > MAX_SHORT_STRING_LEN {
        return Err(format!("outcome exceeds {MAX_SHORT_STRING_LEN} bytes"));
    }
    let outcome = uptrakit_audit_log::AuditOutcome::try_from(payload.outcome.as_str())
        .map_err(|_| format!("unsupported audit outcome: {}", payload.outcome))?;
    for (field, value) in [
        ("tenant_id", payload.tenant_id.as_deref()),
        ("target_type", payload.target_type.as_deref()),
        ("target_id", payload.target_id.as_deref()),
        ("target_display", payload.target_display.as_deref()),
        ("request_id", payload.request_id.as_deref()),
    ] {
        if let Some(value) = value
            && value.len() > MAX_SHORT_STRING_LEN
        {
            return Err(format!("{field} exceeds {MAX_SHORT_STRING_LEN} bytes"));
        }
    }
    let details_json = match payload.details_json.as_deref() {
        Some(details_json) => {
            if details_json.len() > MAX_LONG_STRING_LEN {
                return Err(format!("details_json exceeds {MAX_LONG_STRING_LEN} bytes"));
            }
            Some(
                serde_json::from_str::<serde_json::Value>(details_json)
                    .map_err(|error| format!("details_json is not valid JSON: {error}"))?,
            )
        }
        None => None,
    };
    Ok((action_type, outcome, scope, details_json))
}

async fn resolve_service_audit_identity(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
) -> Option<(Option<uuid::Uuid>, String)> {
    if is_system {
        match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(service)) => Some((
                None,
                service
                    .service_app_name
                    .unwrap_or_else(|| "unknown".to_string()),
            )),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(
                    %service_id,
                    error = %error,
                    "failed to resolve system service audit identity"
                );
                None
            }
        }
    } else {
        match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(service)) => Some((
                Some(service.tenant_id),
                service
                    .service_app_name
                    .unwrap_or_else(|| "unknown".to_string()),
            )),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(
                    %service_id,
                    error = %error,
                    "failed to resolve tenant service audit identity"
                );
                None
            }
        }
    }
}

async fn resolve_service_target_display(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
) -> String {
    if is_system {
        if let Ok(Some(service)) = sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            if !service.friendly_name.is_empty() {
                return service.friendly_name;
            }
            if !service.hostname.is_empty() {
                return service.hostname;
            }
            if let Some(service_app_name) =
                service.service_app_name.filter(|value| !value.is_empty())
            {
                return service_app_name;
            }
        }
    } else if let Ok(Some(service)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        if !service.friendly_name.is_empty() {
            return service.friendly_name;
        }
        if !service.hostname.is_empty() {
            return service.hostname;
        }
        if let Some(service_app_name) = service.service_app_name.filter(|value| !value.is_empty()) {
            return service_app_name;
        }
    }

    service_id.to_string()
}

pub(super) async fn ingest_service_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    service_app_name: Option<&str>,
    payload: AuditEventPayload,
) -> bool {
    let (action_type, outcome, scope, details_json) = match validate_audit_event_payload(&payload) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                %service_id,
                action_type = %payload.action_type,
                error = %error,
                "dropping invalid service audit event"
            );
            return false;
        }
    };

    let (resolved_tenant_id, resolved_service_app_name) =
        if service_tenant_id.is_none() || service_app_name.is_none() {
            match resolve_service_audit_identity(state, service_id, is_system).await {
                Some((tenant_id, app_name)) => (
                    service_tenant_id.or(tenant_id),
                    service_app_name.map(str::to_string).unwrap_or(app_name),
                ),
                None => {
                    tracing::warn!(
                        %service_id,
                        action_type = %action_type,
                        "dropping service audit event for unknown service"
                    );
                    return false;
                }
            }
        } else {
            (
                service_tenant_id,
                service_app_name.unwrap_or("unknown").to_string(),
            )
        };

    let payload_tenant_id = match payload.tenant_id.as_deref() {
        Some(tenant_id) => match uuid::Uuid::parse_str(tenant_id) {
            Ok(tenant_id) => Some(tenant_id),
            Err(error) => {
                tracing::warn!(
                    %service_id,
                    action_type = %action_type,
                    error = %error,
                    "dropping service audit event with invalid tenant_id"
                );
                return false;
            }
        },
        None => None,
    };

    let target_tenant_id = match scope {
        AuditEventScope::TenantOnly => {
            if is_system {
                tracing::warn!(
                    %service_id,
                    action_type = %action_type,
                    "dropping tenant-scoped audit event from system service"
                );
                return false;
            }
            let tenant_id = match (payload_tenant_id, resolved_tenant_id) {
                (Some(payload_tenant_id), Some(bound_tenant_id))
                    if payload_tenant_id != bound_tenant_id =>
                {
                    tracing::warn!(
                        %service_id,
                        action_type = %action_type,
                        "dropping tenant-scoped audit event with mismatched tenant_id"
                    );
                    return false;
                }
                (Some(payload_tenant_id), _) => payload_tenant_id,
                (None, Some(bound_tenant_id)) => bound_tenant_id,
                (None, None) => {
                    tracing::warn!(
                        %service_id,
                        action_type = %action_type,
                        "dropping tenant-scoped audit event without tenant_id"
                    );
                    return false;
                }
            };
            Some(tenant_id)
        }
        AuditEventScope::ServiceBound => {
            if is_system {
                if payload_tenant_id.is_some() || resolved_tenant_id.is_some() {
                    tracing::warn!(
                        %service_id,
                        action_type = %action_type,
                        "dropping service-bound system audit event with tenant_id"
                    );
                    return false;
                }
                None
            } else {
                let tenant_id = match (payload_tenant_id, resolved_tenant_id) {
                    (Some(payload_tenant_id), Some(bound_tenant_id))
                        if payload_tenant_id != bound_tenant_id =>
                    {
                        tracing::warn!(
                            %service_id,
                            action_type = %action_type,
                            "dropping service-bound audit event with mismatched tenant_id"
                        );
                        return false;
                    }
                    (Some(payload_tenant_id), _) => payload_tenant_id,
                    (None, Some(bound_tenant_id)) => bound_tenant_id,
                    (None, None) => {
                        tracing::warn!(
                            %service_id,
                            action_type = %action_type,
                            "dropping service-bound audit event without tenant_id"
                        );
                        return false;
                    }
                };
                Some(tenant_id)
            }
        }
        AuditEventScope::SystemOnly => {
            if !is_system {
                tracing::warn!(
                    %service_id,
                    action_type = %action_type,
                    "dropping system-scoped audit event from tenant service"
                );
                return false;
            }
            if payload_tenant_id.is_some() || resolved_tenant_id.is_some() {
                tracing::warn!(
                    %service_id,
                    action_type = %action_type,
                    "dropping system-scoped audit event with tenant_id"
                );
                return false;
            }
            None
        }
    };

    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .actor_service(service_id)
        .actor_display_opt(Some(resolved_service_app_name))
        .target_opt(
            payload.target_type.clone(),
            payload.target_id.clone(),
            payload.target_display.clone(),
        )
        .outcome(outcome)
        .request_id_opt(payload.request_id.clone());
    builder = if let Some(tenant_id) = target_tenant_id {
        builder.tenant_scope(tenant_id)
    } else {
        builder.system_scope()
    };
    if let Some(details_json) = details_json {
        builder = builder.details(details_json);
    }
    let entry = match builder.build() {
        Ok(entry) => entry,
        Err(error) => {
            tracing::warn!(
                %service_id,
                action_type = %payload.action_type,
                error = %error,
                "dropping invalid service audit entry"
            );
            return false;
        }
    };
    state.audit_emitter.emit_best_effort(entry);
    true
}

pub(super) async fn emit_service_enrollment_completed_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
) {
    let payload = AuditEventPayload {
        action_type: uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED.to_string(),
        tenant_id: None,
        target_type: Some("service".to_string()),
        target_id: Some(service_id.to_string()),
        target_display: Some(resolve_service_target_display(state, service_id, false).await),
        outcome: uptrakit_audit_log::AuditOutcome::Success
            .as_str()
            .to_string(),
        details_json: Some(serde_json::json!({ "service_id": service_id }).to_string()),
        request_id: None,
    };
    let _ = ingest_service_audit_event(state, service_id, false, None, None, payload).await;
}

pub(super) async fn emit_service_certificate_issue_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
    not_after: time::OffsetDateTime,
) {
    let is_system = sys_svc_entity::Entity::find_by_id(service_id)
        .one(state.db())
        .await
        .ok()
        .flatten()
        .is_some();
    let payload = AuditEventPayload {
        action_type: uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_ISSUE.to_string(),
        tenant_id: None,
        target_type: Some("service".to_string()),
        target_id: Some(service_id.to_string()),
        target_display: Some(resolve_service_target_display(state, service_id, is_system).await),
        outcome: uptrakit_audit_log::AuditOutcome::Success
            .as_str()
            .to_string(),
        details_json: Some(
            serde_json::json!({
                "not_after": not_after.to_string(),
            })
            .to_string(),
        ),
        request_id: None,
    };
    let _ = ingest_service_audit_event(state, service_id, is_system, None, None, payload).await;
}

pub(super) async fn emit_service_certificate_renew_audit_event(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    not_after: time::OffsetDateTime,
) {
    let payload = AuditEventPayload {
        action_type: uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW.to_string(),
        tenant_id: None,
        target_type: Some("service".to_string()),
        target_id: Some(service_id.to_string()),
        target_display: Some(resolve_service_target_display(state, service_id, is_system).await),
        outcome: uptrakit_audit_log::AuditOutcome::Success
            .as_str()
            .to_string(),
        details_json: Some(
            serde_json::json!({
                "not_after": not_after.to_string(),
            })
            .to_string(),
        ),
        request_id: None,
    };
    let _ = ingest_service_audit_event(state, service_id, is_system, None, None, payload).await;
}

// ---------------------------------------------------------------------------
// LoopAction
// ---------------------------------------------------------------------------

/// Signal returned by message handlers to control the authenticated loop.
pub(super) enum LoopAction {
    /// Continue processing messages.
    Continue,
    /// Break out of the main loop (normal disconnect or error).
    Break,
}

impl LoopAction {
    /// Returns `true` if this action signals the loop should break.
    pub(super) fn is_break(&self) -> bool {
        matches!(self, Self::Break)
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Internal error type for helper functions (deliver_pending_updates, etc.).
#[derive(Debug, Error)]
enum HandlerError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("websocket send failed")]
    WebSocketSend,
}

type HandlerResult<T> = std::result::Result<T, Report<HandlerError>>;

impl_report_conversion!(sea_orm::DbErr => HandlerError::Database);

// ---------------------------------------------------------------------------
// ProcessorMessage
// ---------------------------------------------------------------------------

/// A deserialized service message forwarded to the background processor.
struct ProcessorMessage {
    message: ServiceMessage,
    pagination: Option<ReportPagination>,
}

// ---------------------------------------------------------------------------
// MessageProcessor
// ---------------------------------------------------------------------------

/// Background message processor spawned per WebSocket connection.
///
/// Receives deserialized [`ServiceMessage`] values via an MPSC channel,
/// dispatches them to the appropriate handler, and sends
/// [`ProcessorResponse`] values back to the main loop.
struct MessageProcessor {
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    cert: Option<CertIdentity>,
    is_system: bool,
    has_update_tracking: bool,
    has_software_discovery: bool,
    has_update_hooks: bool,
    has_ui_surfaces: bool,
    has_workload_claims: bool,
    runtime_instance_id: Option<uuid::Uuid>,
    service_app_name: Option<String>,
    service_tenant_id: Option<uuid::Uuid>,
    linked_host_ids: Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    report_tracker: ReportTracker,
}

impl MessageProcessor {
    /// Run the processor loop: read messages, dispatch handlers, send responses.
    async fn run(
        mut self,
        mut msg_rx: tokio::sync::mpsc::Receiver<ProcessorMessage>,
        resp_tx: tokio::sync::mpsc::Sender<ProcessorResponse>,
    ) {
        while let Some(pm) = msg_rx.recv().await {
            let response = self.dispatch(pm.message, pm.pagination).await;
            if resp_tx.send(response).await.is_err() {
                // Main loop dropped -- connection is closing.
                break;
            }
        }
    }

    /// Dispatch a single service message to the appropriate handler.
    ///
    /// Messages are grouped by capability gate. Universal messages (available
    /// to all service types) are handled last.
    async fn dispatch(
        &mut self,
        service_msg: ServiceMessage,
        pagination: Option<ReportPagination>,
    ) -> ProcessorResponse {
        match service_msg {
            // -- SoftwareDiscovery capability --
            ServiceMessage::ReportHosts(payload) if self.has_software_discovery => {
                messages::handle_report_hosts(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                )
                .await
            }
            ServiceMessage::VersionCheckResults(payload)
                if self.has_software_discovery && !self.has_update_tracking =>
            {
                messages::handle_version_check_results(&self.state, self.service_id, &payload).await
            }
            ServiceMessage::DiscoveryResults(payload) if self.has_software_discovery => {
                messages::handle_discovery_results(
                    &self.state,
                    self.service_id,
                    payload,
                    pagination.as_ref(),
                    &mut self.report_tracker,
                )
                .await
            }

            // -- UpdateHooks capability --
            msg @ (ServiceMessage::UpdateStarted(_)
            | ServiceMessage::UpdateOutput(_)
            | ServiceMessage::UpdateResult(_)
            | ServiceMessage::BatchUpdateResult(_)
            | ServiceMessage::StdinAttention(_))
                if self.has_update_hooks =>
            {
                self.dispatch_update_hooks(msg).await
            }

            // -- UpdateTracking capability --
            msg @ (ServiceMessage::ServiceTriggerUpdate(_)
            | ServiceMessage::ServiceTriggerHostBatchUpdate(_))
                if self.has_update_tracking =>
            {
                self.dispatch_update_tracking(msg).await
            }

            // -- Shared surfaces runtime (parallel migration path) --
            msg @ (ServiceMessage::SurfaceRegistration(_)
            | ServiceMessage::SurfaceActionResponse(_)
            | ServiceMessage::SurfaceActionRequest(_))
                if self.has_ui_surfaces =>
            {
                self.dispatch_surfaces(msg).await
            }

            // -- WorkloadClaims capability --
            ServiceMessage::WorkloadClaim(payload) if self.has_workload_claims => {
                workload::handle_workload_claim(&self.state, self.service_id, payload).await
            }
            ServiceMessage::WorkloadRelease(payload) if self.has_workload_claims => {
                workload::handle_workload_release(&self.state, self.service_id, payload).await
            }

            // -- Universal messages (all capabilities) --
            ServiceMessage::AuditEvent(payload) => {
                let _ = ingest_service_audit_event(
                    &self.state,
                    self.service_id,
                    self.is_system,
                    self.service_tenant_id,
                    self.service_app_name.as_deref(),
                    payload,
                )
                .await;
                ProcessorResponse::cont()
            }
            ServiceMessage::RenewCertificate(payload) => {
                if let Some(ref cert) = self.cert {
                    messages::handle_renew_certificate(
                        &self.state,
                        self.service_id,
                        cert,
                        &payload,
                        self.is_system,
                    )
                    .await
                } else {
                    // Embedded services do not use certificates.
                    ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                        code: ErrorCode::BadRequest,
                        message: "certificate renewal not supported for embedded services"
                            .to_string(),
                    }))
                }
            }
            ServiceMessage::TestPluginConfigResult(payload) => {
                let request_id = payload.request_id.clone();
                self.state.config_test_proxy.complete(&request_id, payload);
                ProcessorResponse::cont()
            }
            ServiceMessage::ReportPluginConfig(payload) => {
                messages::handle_report_plugin_config(&self.state, self.service_id, &payload).await
            }
            ServiceMessage::StoreServiceConfig(payload) => {
                if !is_valid_service_config_scope(self.service_tenant_id, payload.tenant_id) {
                    service_config::emit_service_config_scope_denied_audit_event(
                        service_config::ServiceConfigAuditCtx {
                            state: &self.state,
                            action_type: uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_STORE,
                            service_id: self.service_id,
                            service_app_name: self.service_app_name.as_deref().unwrap_or(""),
                        },
                        self.service_tenant_id
                            .expect("service config scope denial requires tenant binding"),
                        payload.tenant_id,
                        &payload.key,
                        &payload.request_id,
                        "outside_tenant_binding",
                    );
                    return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                        uptrakit_wire::ServiceConfigAckPayload::error(
                            payload.request_id,
                            "service cannot write config outside its tenant binding".to_string(),
                        ),
                    ));
                }
                service_config::handle_store_service_config(
                    &self.state,
                    self.service_app_name.as_deref().unwrap_or(""),
                    self.service_id,
                    payload,
                )
                .await
            }
            ServiceMessage::DeleteServiceConfig(payload) => {
                if !is_valid_service_config_scope(self.service_tenant_id, payload.tenant_id) {
                    service_config::emit_service_config_scope_denied_audit_event(
                        service_config::ServiceConfigAuditCtx {
                            state: &self.state,
                            action_type: uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_DELETE,
                            service_id: self.service_id,
                            service_app_name: self.service_app_name.as_deref().unwrap_or(""),
                        },
                        self.service_tenant_id
                            .expect("service config scope denial requires tenant binding"),
                        payload.tenant_id,
                        &payload.key,
                        &payload.request_id,
                        "outside_tenant_binding",
                    );
                    return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                        uptrakit_wire::ServiceConfigAckPayload::error(
                            payload.request_id,
                            "service cannot delete config outside its tenant binding".to_string(),
                        ),
                    ));
                }
                service_config::handle_delete_service_config(
                    &self.state,
                    self.service_app_name.as_deref().unwrap_or(""),
                    self.service_id,
                    payload,
                )
                .await
            }

            // -- Register: embedded services send this on startup to declare capabilities --
            ServiceMessage::Register(payload) => {
                self.runtime_instance_id = payload.runtime_instance_id;
                upgrade_service_capabilities(
                    self.state.db(),
                    self.service_id,
                    self.is_system,
                    payload.capabilities,
                    &mut self.has_ui_surfaces,
                )
                .await;

                if let Err(error) = updates::recover_owned_updates_on_connect_with_dispatch_mode(
                    &self.state,
                    self.service_id,
                    self.runtime_instance_id,
                    updates::ReconnectSuccessorDispatchMode::Immediate,
                )
                .await
                {
                    tracing::warn!(
                        error = %error,
                        %self.service_id,
                        "embedded reconnect recovery failed"
                    );
                }

                ProcessorResponse::cont()
            }

            // -- Disconnecting: embedded services send this during shutdown --
            ServiceMessage::Disconnecting(_) => {
                tracing::debug!(
                    service_id = %self.service_id,
                    "embedded service sent Disconnecting"
                );
                ProcessorResponse {
                    replies: Vec::new(),
                    action: ProcessorAction::Break,
                }
            }

            _ => ProcessorResponse::reply_and_break(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: "message not supported for this service capability".to_string(),
            })),
        }
    }

    /// Dispatch update-hooks messages (UpdateStarted, UpdateOutput, etc.).
    async fn dispatch_update_hooks(&self, msg: ServiceMessage) -> ProcessorResponse {
        match msg {
            ServiceMessage::UpdateStarted(payload) => {
                updates::handle_update_started(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                    self.runtime_instance_id,
                )
                .await
            }
            ServiceMessage::UpdateOutput(payload) => {
                updates::handle_update_output(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                    self.runtime_instance_id,
                )
                .await
            }
            ServiceMessage::UpdateResult(payload) => {
                updates::handle_update_result(
                    &self.state,
                    self.service_id,
                    payload,
                    &self.linked_host_ids,
                    self.runtime_instance_id,
                )
                .await
            }
            ServiceMessage::BatchUpdateResult(payload) => {
                updates::handle_batch_update_result(
                    &self.state,
                    self.service_id,
                    payload,
                    &self.linked_host_ids,
                    self.runtime_instance_id,
                )
                .await
            }
            ServiceMessage::StdinAttention(payload) => {
                updates::handle_stdin_attention(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                    self.runtime_instance_id,
                )
                .await
            }
            _ => unreachable!("dispatch_update_hooks called with non-update message"),
        }
    }

    /// Dispatch update-tracking messages (ServiceTriggerUpdate, etc.).
    async fn dispatch_update_tracking(&self, msg: ServiceMessage) -> ProcessorResponse {
        let service_app_name = self.service_app_name.as_deref().unwrap_or("unknown");
        match msg {
            ServiceMessage::ServiceTriggerUpdate(payload) => {
                update_tracking::handle_service_trigger_update(
                    &self.state,
                    service_app_name,
                    &payload,
                )
                .await
            }
            ServiceMessage::ServiceTriggerHostBatchUpdate(payload) => {
                update_tracking::handle_service_trigger_host_batch_update(
                    &self.state,
                    service_app_name,
                    &payload,
                )
                .await
            }
            _ => unreachable!("dispatch_update_tracking called with non-update-tracking message"),
        }
    }

    /// Dispatch surface runtime messages (SurfaceRegistration, SurfaceActionResponse, etc.).
    async fn dispatch_surfaces(&mut self, msg: ServiceMessage) -> ProcessorResponse {
        match msg {
            ServiceMessage::SurfaceRegistration(payload) => {
                self.handle_surface_registration(payload).await
            }
            ServiceMessage::SurfaceActionResponse(payload) => {
                self.state
                    .surface_proxy
                    .complete(payload.request_id, payload);
                ProcessorResponse::cont()
            }
            ServiceMessage::SurfaceActionRequest(payload) => {
                self.handle_surface_action_request(payload).await
            }
            _ => unreachable!("dispatch_surfaces called with non-surface message"),
        }
    }

    /// Handle a `SurfaceRegistration` message: validate and register provider surfaces.
    async fn handle_surface_registration(
        &self,
        payload: uptrakit_wire::surfaces::SurfaceRegistration,
    ) -> ProcessorResponse {
        if let Err(e) = payload.wire_validate() {
            emit_surface_registration_audit_event(
                &ServiceAuditCtx {
                    state: &self.state,
                    service_id: self.service_id,
                    service_app_name: self.service_app_name.as_deref(),
                },
                self.is_system,
                self.service_tenant_id,
                &payload,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                Some(classify_surface_registration_validation_error(&e)),
            );
            tracing::warn!(
                service_id = %self.service_id,
                error = %e,
                "invalid SurfaceRegistration payload"
            );
            return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: format!("invalid surface registration: {e}"),
            }));
        }

        let app_name = self.service_app_name.as_deref().unwrap_or("unknown");
        if let Err(e) = register_surface_provider(
            self.state.surface_registry.as_ref(),
            self.state.surface_proxy.as_ref(),
            self.service_id,
            app_name,
            self.service_tenant_id,
            payload.clone(),
        ) {
            emit_surface_registration_audit_event(
                &ServiceAuditCtx {
                    state: &self.state,
                    service_id: self.service_id,
                    service_app_name: self.service_app_name.as_deref(),
                },
                self.is_system,
                self.service_tenant_id,
                &payload,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some(classify_surface_registration_error_for_audit(&e)),
            );
            tracing::warn!(
                service_id = %self.service_id,
                app_name,
                error = %e,
                "surface registration rejected"
            );
            return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: surface_registration_error_message(&e),
            }));
        }

        tracing::info!(
            service_id = %self.service_id,
            app_name,
            "registered service surfaces"
        );
        emit_surface_registration_audit_event(
            &ServiceAuditCtx {
                state: &self.state,
                service_id: self.service_id,
                service_app_name: self.service_app_name.as_deref(),
            },
            self.is_system,
            self.service_tenant_id,
            &payload,
            uptrakit_audit_log::AuditOutcome::Success,
            None,
        );
        ProcessorResponse::cont()
    }

    /// Handle a `SurfaceActionRequest` message: service-initiated surface action invocation.
    async fn handle_surface_action_request(
        &self,
        payload: uptrakit_wire::surfaces::SurfaceActionRequest,
    ) -> ProcessorResponse {
        let request_id = payload.request_id;

        if let Err(e) = payload.wire_validate() {
            tracing::warn!(
                service_id = %self.service_id,
                error = %e,
                "invalid SurfaceActionRequest payload"
            );
            if let Some(tenant_id) =
                resolve_surface_action_audit_tenant_id(self.service_tenant_id, &payload)
            {
                emit_surface_action_invoke_audit_event(
                    &ServiceAuditCtx {
                        state: &self.state,
                        service_id: self.service_id,
                        service_app_name: self.service_app_name.as_deref(),
                    },
                    tenant_id,
                    &payload,
                    None,
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    Some(classify_surface_action_request_validation_error(&e)),
                );
            }
            return ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(
                uptrakit_wire::surfaces::SurfaceActionResponse {
                    request_id,
                    success: false,
                    result: None,
                    error: Some(uptrakit_wire::surfaces::SurfaceActionError {
                        code: uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
                        message: format!("invalid surface action request: {e}"),
                        details: None,
                    }),
                },
            ));
        }

        let request_tenant_id = match uuid::Uuid::parse_str(&payload.tenant_id) {
            Ok(tenant_id) => tenant_id,
            Err(error) => {
                if let Some(tenant_id) = self.service_tenant_id {
                    emit_surface_action_invoke_audit_event(
                        &ServiceAuditCtx {
                            state: &self.state,
                            service_id: self.service_id,
                            service_app_name: self.service_app_name.as_deref(),
                        },
                        tenant_id,
                        &payload,
                        None,
                        uptrakit_audit_log::AuditOutcome::ValidationFailed,
                        Some("invalid_tenant_id"),
                    );
                }
                return ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(
                    uptrakit_wire::surfaces::SurfaceActionResponse {
                        request_id,
                        success: false,
                        result: None,
                        error: Some(uptrakit_wire::surfaces::SurfaceActionError {
                            code: uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
                            message: format!("invalid tenant_id: {error}"),
                            details: None,
                        }),
                    },
                ));
            }
        };

        if let Some(service_tenant_id) = self.service_tenant_id
            && service_tenant_id != request_tenant_id
        {
            emit_surface_action_scope_denied_audit_event(
                &self.state,
                self.service_id,
                self.service_app_name.as_deref(),
                service_tenant_id,
                &payload,
            )
            .await;
            return ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(
                uptrakit_wire::surfaces::SurfaceActionResponse {
                    request_id,
                    success: false,
                    result: None,
                    error: Some(uptrakit_wire::surfaces::SurfaceActionError {
                        code: uptrakit_wire::surfaces::SurfaceActionErrorCode::PermissionDenied,
                        message: "service cannot invoke actions outside its tenant".to_string(),
                        details: None,
                    }),
                },
            ));
        }

        let invoke_request = crate::surface_proxy::SurfaceInvokeRequest {
            tenant_id: request_tenant_id,
            surface_id: payload.surface_id.to_string(),
            interaction_id: payload.interaction_id.to_string(),
            idempotency_key: payload.idempotency_key.clone(),
            target_provider_id: payload.target_provider_id.clone(),
            caller_origin: crate::surface_proxy::SurfaceCallerOrigin::Provider {
                service_id: self.service_id,
            },
            params: payload.params.clone(),
            encrypted_sensitive_params: payload.encrypted_sensitive_params.clone(),
        };
        let resolved = match self.state.surface_registry.resolve_surface_action(
            request_tenant_id,
            payload.surface_id.as_str(),
            payload.interaction_id.as_str(),
            payload.target_provider_id.as_deref(),
        ) {
            Ok(resolved) => Some(resolved),
            Err(error) => {
                let (outcome, reason_code) = classify_surface_lookup_error_for_audit(&error);
                emit_surface_action_invoke_audit_event(
                    &ServiceAuditCtx {
                        state: &self.state,
                        service_id: self.service_id,
                        service_app_name: self.service_app_name.as_deref(),
                    },
                    request_tenant_id,
                    &payload,
                    None,
                    outcome,
                    Some(reason_code),
                );
                return ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(
                    uptrakit_wire::surfaces::SurfaceActionResponse {
                        request_id,
                        success: false,
                        result: None,
                        error: Some(surface_registry_lookup_error_to_wire(error)),
                    },
                ));
            }
        };

        let response = match self
            .state
            .surface_proxy
            .invoke(
                &self.state.service_connections,
                &self.state.surface_registry,
                invoke_request,
                None,
            )
            .await
        {
            Ok(mut response) => {
                let (outcome, reason_code) = classify_surface_action_response_for_audit(&response);
                emit_surface_action_invoke_audit_event(
                    &ServiceAuditCtx {
                        state: &self.state,
                        service_id: self.service_id,
                        service_app_name: self.service_app_name.as_deref(),
                    },
                    request_tenant_id,
                    &payload,
                    resolved.as_ref(),
                    outcome,
                    reason_code,
                );
                response.request_id = request_id;
                response
            }
            Err(error) => {
                let (outcome, reason_code) = classify_surface_proxy_error_for_audit(&error);
                emit_surface_action_invoke_audit_event(
                    &ServiceAuditCtx {
                        state: &self.state,
                        service_id: self.service_id,
                        service_app_name: self.service_app_name.as_deref(),
                    },
                    request_tenant_id,
                    &payload,
                    resolved.as_ref(),
                    outcome,
                    Some(reason_code),
                );
                uptrakit_wire::surfaces::SurfaceActionResponse {
                    request_id,
                    success: false,
                    result: None,
                    error: Some(surface_proxy_error_to_wire(error)),
                }
            }
        };

        ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(response))
    }
}

fn register_surface_provider(
    surface_registry: &crate::surface_registry::SurfaceRegistry,
    surface_proxy: &crate::surface_proxy::SurfaceProxy,
    service_id: uuid::Uuid,
    app_name: &str,
    service_tenant_id: Option<uuid::Uuid>,
    payload: uptrakit_wire::surfaces::SurfaceRegistration,
) -> Result<(), crate::surface_registry::SurfaceRegistryError> {
    let previous_provider_id = surface_registry.provider_id_for_service(&service_id);
    let incoming_provider_id = payload.provider.provider_id.clone();

    surface_registry.register_service(service_id, app_name, service_tenant_id, payload)?;

    if let Some(previous_provider_id) = previous_provider_id
        && previous_provider_id != incoming_provider_id
    {
        surface_proxy.fail_in_flight_for_provider(&previous_provider_id);
    }

    Ok(())
}

fn surface_registration_error_message(
    error: &crate::surface_registry::SurfaceRegistryError,
) -> String {
    match error {
        crate::surface_registry::SurfaceRegistryError::ProviderRejected(rejection) => {
            serde_json::to_string(rejection).unwrap_or_else(|_| error.to_string())
        }
        crate::surface_registry::SurfaceRegistryError::ProviderConflict(_) => error.to_string(),
    }
}

fn surface_proxy_error_to_wire(
    error: crate::surface_proxy::SurfaceProxyError,
) -> uptrakit_wire::surfaces::SurfaceActionError {
    let (code, message) = match error {
        crate::surface_proxy::SurfaceProxyError::NoProvider => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable,
            "no provider available for requested surface interaction".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::TargetProviderRequired => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "target_provider_id is required for targeted surface interactions".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::InvalidProvider(message) => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            message,
        ),
        crate::surface_proxy::SurfaceProxyError::PermissionDenied(message) => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::PermissionDenied,
            message,
        ),
        crate::surface_proxy::SurfaceProxyError::Conflict { message, .. } => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            message,
        ),
        crate::surface_proxy::SurfaceProxyError::SchemaValidationFailed(message)
        | crate::surface_proxy::SurfaceProxyError::SensitiveFieldRejected(message) => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            message,
        ),
        crate::surface_proxy::SurfaceProxyError::InteractionNotFound => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "interaction not found".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::DuplicateRequest => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::DuplicateRequest,
            "duplicate idempotency key".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::RateLimited => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable,
            "surface provider is temporarily rate-limited".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::ServiceDisconnected
        | crate::surface_proxy::SurfaceProxyError::SendFailed => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable,
            "surface provider is disconnected".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::Timeout => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::Timeout,
            "surface action timed out".to_string(),
        ),
    };

    uptrakit_wire::surfaces::SurfaceActionError {
        code,
        message,
        details: None,
    }
}

fn surface_registry_lookup_error_to_wire(
    error: crate::surface_registry::SurfaceRegistryLookupError,
) -> uptrakit_wire::surfaces::SurfaceActionError {
    let (code, message) = match error {
        crate::surface_registry::SurfaceRegistryLookupError::SurfaceNotFound => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "surface not found".to_string(),
        ),
        crate::surface_registry::SurfaceRegistryLookupError::InteractionNotFound => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "interaction not found".to_string(),
        ),
        crate::surface_registry::SurfaceRegistryLookupError::TargetProviderRequired => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "target_provider_id is required for targeted surface interactions".to_string(),
        ),
        crate::surface_registry::SurfaceRegistryLookupError::InvalidProvider(message) => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            message,
        ),
        crate::surface_registry::SurfaceRegistryLookupError::NoTenantCompatibleProvider => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable,
            "no provider available for requested surface interaction".to_string(),
        ),
    };

    uptrakit_wire::surfaces::SurfaceActionError {
        code,
        message,
        details: None,
    }
}

fn action_error_code(code: &uptrakit_wire::surfaces::SurfaceActionErrorCode) -> &'static str {
    match code {
        uptrakit_wire::surfaces::SurfaceActionErrorCode::PermissionDenied => "permission_denied",
        uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest => "invalid_request",
        uptrakit_wire::surfaces::SurfaceActionErrorCode::SchemaValidationFailed => {
            "schema_validation_failed"
        }
        uptrakit_wire::surfaces::SurfaceActionErrorCode::UnsupportedCapability => {
            "unsupported_capability"
        }
        uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable => {
            "provider_unavailable"
        }
        uptrakit_wire::surfaces::SurfaceActionErrorCode::Timeout => "timeout",
        uptrakit_wire::surfaces::SurfaceActionErrorCode::DuplicateRequest => "duplicate_request",
        uptrakit_wire::surfaces::SurfaceActionErrorCode::InternalError => "internal_error",
    }
}

// ---------------------------------------------------------------------------
// AuthenticatedSessionState
// ---------------------------------------------------------------------------

/// All state produced during authenticated session setup that the main loop
/// and cleanup phases need.
struct AuthenticatedSessionState {
    service_id: uuid::Uuid,
    connected_at: time::OffsetDateTime,
    is_system: bool,
    has_update_tracking: bool,
    has_software_discovery: bool,
    has_workload_claims: bool,
    service_tenant_id: Option<uuid::Uuid>,
    linked_host_ids: Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    push_rx: tokio::sync::mpsc::Receiver<ControllerMessage>,
    cancel_token: tokio_util::sync::CancellationToken,
    msg_tx: tokio::sync::mpsc::Sender<ProcessorMessage>,
    resp_rx: tokio::sync::mpsc::Receiver<ProcessorResponse>,
    processor_cancel: tokio_util::sync::CancellationToken,
    processor_handle: tokio::task::JoinHandle<()>,
    rate_limiter: MessageRateLimiter,
}

// ---------------------------------------------------------------------------
// setup_authenticated_session — stage helpers
// ---------------------------------------------------------------------------

/// Stage 1: Load service from DB and return capabilities, app name, and tenant
/// ID.
///
/// Falls back to empty capabilities on any DB error or missing row so that
/// the setup can continue with a degraded (no-capability) service rather than
/// crashing.
async fn load_service_capabilities(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
) -> (BTreeSet<Capability>, Option<String>, Option<uuid::Uuid>) {
    if is_system {
        match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => (
                parse_capabilities(&svc.capabilities),
                svc.service_app_name.clone(),
                system_service_tenant_binding(
                    svc.service_app_name.as_deref(),
                    state.default_tenant_id,
                ),
            ),
            _ => (BTreeSet::new(), None, None),
        }
    } else {
        match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => (
                parse_capabilities(&svc.capabilities),
                svc.service_app_name,
                Some(svc.tenant_id),
            ),
            _ => (BTreeSet::new(), None, None),
        }
    }
}

/// Stage 3: Register the connection in `ServiceConnectionRegistry` and notify
/// the embedded service infrastructure about the new external connection.
///
/// Returns `(push_rx, cancel_token, connected_at)`.
fn cancellation_token_from_connection_handle(
    connection: crate::service_connections::ServiceConnectionHandle,
) -> tokio_util::sync::CancellationToken {
    let cancel_token = tokio_util::sync::CancellationToken::new();
    let notify_token = cancel_token.clone();
    tokio::spawn(async move {
        connection.cancelled().await;
        notify_token.cancel();
    });
    cancel_token
}

async fn register_connection(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    capabilities: &BTreeSet<Capability>,
    service_app_name: Option<String>,
) -> (
    tokio::sync::mpsc::Receiver<ControllerMessage>,
    tokio_util::sync::CancellationToken,
    time::OffsetDateTime,
) {
    let (push_rx, connection_handle) = state
        .service_connections
        .register(
            service_id,
            capabilities.clone(),
            None,
            None,
            service_app_name,
        )
        .await;

    // Notify embedded services about the new external connection.
    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_connected(service_id, capabilities, None, false);
    }

    let connected_at = state
        .service_connections
        .connected_at(&service_id)
        .await
        .expect("connected service should have a registered timestamp");

    let cancel_token = cancellation_token_from_connection_handle(connection_handle);

    (push_rx, cancel_token, connected_at)
}

/// Stage 4: Load linked host IDs shared between the main loop and the processor.
async fn load_session_host_ids(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    has_software_discovery: bool,
) -> Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>> {
    if has_software_discovery {
        Arc::new(parking_lot::Mutex::new(
            load_linked_host_ids(state.db(), service_id)
                .await
                .unwrap_or_default(),
        ))
    } else {
        Arc::new(parking_lot::Mutex::new(HashSet::new()))
    }
}

/// Stage 5: Prepare reconnect cleanup and any replayable pending updates.
///
/// Errors are logged but do not abort setup — the connection is still usable.
async fn prepare_reconnect_updates_on_connect(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    runtime_instance_id: Option<uuid::Uuid>,
    has_update_hooks: bool,
    out_seq: &mut OutgoingSeq,
) {
    let replay = reconnect::prepare_reconnect_replay(
        state,
        service_id,
        runtime_instance_id,
        has_update_hooks,
        true,
    )
    .await;

    for msg in replay.messages {
        let Some(json) = serialize_controller_msg(out_seq, msg) else {
            continue;
        };
        if !send_ws_with_timeout(sink, json, service_id).await {
            tracing::error!(%service_id, "failed to send replayed pending update on reconnect");
            break;
        }
    }
}

/// Output of [`spawn_message_processor`]: channels for communicating with the
/// background task.
struct ProcessorChannels {
    msg_tx: tokio::sync::mpsc::Sender<ProcessorMessage>,
    resp_rx: tokio::sync::mpsc::Receiver<ProcessorResponse>,
    processor_cancel: tokio_util::sync::CancellationToken,
    processor_handle: tokio::task::JoinHandle<()>,
}

/// Stage 6: Spawn the background [`MessageProcessor`] and return the channels
/// the main loop needs to exchange messages with it.
fn spawn_message_processor(processor: MessageProcessor) -> ProcessorChannels {
    let (msg_tx, msg_rx) =
        tokio::sync::mpsc::channel::<ProcessorMessage>(PROCESSOR_CHANNEL_CAPACITY);
    let (resp_tx, resp_rx) =
        tokio::sync::mpsc::channel::<ProcessorResponse>(RESPONSE_CHANNEL_CAPACITY);

    let processor_cancel = tokio_util::sync::CancellationToken::new();
    let proc_cancel_clone = processor_cancel.clone();
    let processor_handle = tokio::spawn(async move {
        tokio::select! {
            () = processor.run(msg_rx, resp_tx) => {}
            () = proc_cancel_clone.cancelled() => {}
        }
    });

    ProcessorChannels {
        msg_tx,
        resp_rx,
        processor_cancel,
        processor_handle,
    }
}

// ---------------------------------------------------------------------------
// Embedded service message handler
// ---------------------------------------------------------------------------

/// Run a message handler loop for an embedded service.
///
/// This creates a [`MessageProcessor`] configured for an embedded (in-process)
/// service and reads messages from the provided channel. Replies are pushed
/// back through the [`ServiceConnectionRegistry`].
///
/// Used by `embedded_support::run_embedded_message_handler`.
pub(crate) async fn run_embedded_message_handler(
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    capabilities: &BTreeSet<Capability>,
    app_name: &str,
    service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    run_embedded_message_handler_inner(
        state,
        EmbeddedHandlerSession {
            service_id,
            is_system: false,
            service_tenant_id: Some(tenant_id),
            app_name,
        },
        capabilities,
        service_rx,
        cancel,
    )
    .await;
}

pub(crate) async fn run_embedded_system_message_handler(
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    service_tenant_id: Option<uuid::Uuid>,
    capabilities: &BTreeSet<Capability>,
    app_name: &str,
    service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    run_embedded_message_handler_inner(
        state,
        EmbeddedHandlerSession {
            service_id,
            is_system: true,
            service_tenant_id,
            app_name,
        },
        capabilities,
        service_rx,
        cancel,
    )
    .await;
}

struct EmbeddedHandlerSession<'a> {
    service_id: uuid::Uuid,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    app_name: &'a str,
}

async fn run_embedded_message_handler_inner(
    state: Arc<AppState>,
    session: EmbeddedHandlerSession<'_>,
    capabilities: &BTreeSet<Capability>,
    mut service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let has_software_discovery = capabilities.contains(&Capability::SoftwareDiscovery);
    let has_update_hooks = capabilities.contains(&Capability::UpdateHooks);
    let has_ui_surfaces = capabilities.contains(&Capability::UiSurfaces);
    let has_workload_claims = capabilities.contains(&Capability::WorkloadClaims);
    let has_update_tracking = capabilities.contains(&Capability::UpdateTracking);

    let linked_host_ids =
        load_session_host_ids(&state, session.service_id, has_software_discovery).await;

    let mut processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id: session.service_id,
        cert: None,
        is_system: session.is_system,
        has_update_tracking,
        has_software_discovery,
        has_update_hooks,
        has_ui_surfaces,
        has_workload_claims,
        runtime_instance_id: None,
        service_app_name: Some(session.app_name.to_string()),
        service_tenant_id: session.service_tenant_id,
        linked_host_ids,
        report_tracker: ReportTracker::new(),
    };

    loop {
        let msg = tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            msg = service_rx.recv() => match msg {
                Some(m) => m,
                None => break,
            },
        };

        let response = processor.dispatch(msg, None).await;

        for reply in response.replies {
            state
                .service_connections
                .send(&session.service_id, reply)
                .await;
        }

        match response.action {
            ProcessorAction::Continue => {}
            ProcessorAction::Break | ProcessorAction::CloseWithReason(_) => {
                tracing::info!(
                    service_id = %session.service_id,
                    app_name = session.app_name,
                    "embedded message handler stopping (processor requested break)"
                );
                break;
            }
        }
    }

    cleanup_embedded_service_session(
        &state,
        session.service_id,
        session.app_name,
        has_workload_claims,
    )
    .await;

    tracing::debug!(
        service_id = %session.service_id,
        app_name = session.app_name,
        "embedded message handler exited"
    );
}

async fn cleanup_embedded_service_session(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    _service_app_name: &str,
    has_workload_claims: bool,
) {
    if has_workload_claims {
        workload::release_all_claims_on_disconnect(state, service_id).await;
    }

    if let Some(provider_id) = state.surface_registry.provider_id_for_service(&service_id) {
        state
            .surface_proxy
            .fail_in_flight_for_provider(&provider_id);
    }
    state.surface_registry.unregister_service(&service_id);

    state.service_connections.unregister(&service_id).await;

    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_disconnected(&service_id);
    }
}

// ---------------------------------------------------------------------------
// receive_register_message
// ---------------------------------------------------------------------------

/// Read the first frame from the service and expect it to be a `Register` message.
///
/// Called as Stage 3 of [`setup_authenticated_session`], immediately after
/// credential and config delivery, before the service begins sending
/// operational messages. The service must send `Register` synchronously from
/// `on_connected` so it arrives here before any other message.
///
/// Returns `Some(RegisterPayload)` on success, or `None` if:
/// - The connection closed or produced a read error.
/// - Rate limiting was exceeded.
/// - Deserialization failed (hard error — malformed frame).
/// - The first message was not `ServiceMessage::Register`.
///
/// On failure the connection is closed with [`CloseReason::ProtocolError`].
#[allow(clippy::too_many_arguments)]
async fn receive_register_message(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    rate_limiter: &mut MessageRateLimiter,
) -> Option<RegisterPayload> {
    use futures_util::StreamExt as _;

    let _ = (state, out_seq); // unused but kept for consistency with other stage helpers

    let frame = match stream.next().await {
        Some(Ok(f)) => f,
        Some(Err(e)) => {
            tracing::debug!(%service_id, error = %e, "websocket read error waiting for Register");
            return None;
        }
        None => {
            tracing::debug!(%service_id, "connection closed before Register was received");
            return None;
        }
    };

    let text = match frame {
        Message::Text(t) => t,
        Message::Close(_) => {
            tracing::debug!(%service_id, "received Close frame waiting for Register");
            return None;
        }
        _ => {
            tracing::warn!(%service_id, "expected text frame for Register, got non-text frame");
            let _ = close_with_reason(sink, CloseReason::ProtocolError).await;
            return None;
        }
    };

    if !rate_limiter.allow() {
        tracing::warn!(%service_id, "rate limit exceeded on Register frame");
        let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
        return None;
    }

    let deserialized = match deserialize_service_msg(in_seq, &text) {
        Ok(Some(d)) => d,
        Ok(None) => {
            tracing::warn!(%service_id, "Register frame could not be deserialized (unknown type)");
            let _ = close_with_reason(sink, CloseReason::ProtocolError).await;
            return None;
        }
        Err(e) => {
            tracing::debug!(%service_id, error = %e, "hard deserialize error on Register frame");
            let _ = close_with_reason(sink, CloseReason::ProtocolError).await;
            return None;
        }
    };

    match deserialized.message {
        ServiceMessage::Register(payload) => {
            tracing::debug!(
                %service_id,
                capabilities = ?payload.capabilities,
                "received Register from service"
            );
            Some(payload)
        }
        other => {
            tracing::warn!(
                %service_id,
                message_type = ?std::mem::discriminant(&other),
                "expected Register as first message, got unexpected variant; closing connection"
            );
            let _ = close_with_reason(sink, CloseReason::ProtocolError).await;
            None
        }
    }
}

// ---------------------------------------------------------------------------
// setup_authenticated_session
// ---------------------------------------------------------------------------

/// Perform all pre-loop setup for the authenticated handler.
///
/// Loads the service from the DB, delivers credentials, receives the Register
/// handshake, registers the connection, spawns the background processor, and
/// delivers pending updates.
///
/// Returns `None` if the connection must be closed early (e.g. failed Register
/// handshake or write failure).
// All parameters originate from the caller's `AuthenticatedContext` and cannot
// be meaningfully grouped without introducing a wrapper that duplicates it.
#[allow(clippy::too_many_arguments)]
async fn setup_authenticated_session(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    cert: &CertIdentity,
    is_system: bool,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) -> Option<AuthenticatedSessionState> {
    // Stage 1: Load service record from DB. The DB capabilities are used for
    // credential delivery (DatabaseAccess, NatsAccess, etc.). Session-level
    // capability flags (has_update_tracking, has_software_discovery, etc.) come from the
    // Register handshake in Stage 3 so they are correct on first connect even
    // when the DB row has no stored capabilities yet.
    let (db_capabilities, service_app_name, service_tenant_id) =
        load_service_capabilities(state, service_id, is_system).await;

    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

    // Stage 2: Deliver credentials to services that have credential capabilities.
    deliver_service_credentials(
        sink,
        state,
        &db_capabilities,
        credentials::ServiceCredentialTarget {
            service_id,
            is_system,
            service_tenant_id,
            service_app_name: service_app_name.as_deref(),
        },
        out_seq,
    )
    .await?;

    // Stage 2.5: Deliver stored service config entries to services with a known app name.
    if let Some(ref app_name) = service_app_name {
        service_config::deliver_service_config(
            sink,
            state,
            service_id,
            is_system,
            service_tenant_id,
            app_name,
            out_seq,
        )
        .await?;
    }

    // Stage 3: Receive the Register handshake from the service.
    //
    // The service sends `Register` from `on_connected` immediately after the
    // controller completes credential + config delivery. This gives us the
    // authoritative session-level capability set before we register the
    // connection, so all downstream decisions use live data rather than
    // potentially-stale DB values.
    let register_payload = receive_register_message(
        sink,
        stream,
        state,
        service_id,
        out_seq,
        in_seq,
        &mut rate_limiter,
    )
    .await?;
    let runtime_instance_id = register_payload.runtime_instance_id;

    let session_capabilities = register_payload.capabilities.clone();
    let has_update_tracking = session_capabilities.contains(&Capability::UpdateTracking);
    let has_software_discovery = session_capabilities.contains(&Capability::SoftwareDiscovery);
    let has_update_hooks = session_capabilities.contains(&Capability::UpdateHooks);
    let has_ui_surfaces = session_capabilities.contains(&Capability::UiSurfaces);
    let has_workload_claims = session_capabilities.contains(&Capability::WorkloadClaims);

    // Persist the session capabilities to the DB so that subsequent reconnects
    // (and other controller instances) see the up-to-date capability set.
    upgrade_service_capabilities(
        state.db(),
        service_id,
        is_system,
        register_payload.capabilities,
        &mut { has_ui_surfaces },
    )
    .await;

    // Stage 4: Register the connection and notify embedded services.
    let (push_rx, cancel_token, connected_at) = register_connection(
        state,
        service_id,
        &session_capabilities,
        service_app_name.clone(),
    )
    .await;

    // Stage 5: Load linked host IDs shared between the main loop and the processor.
    let linked_host_ids = load_session_host_ids(state, service_id, has_software_discovery).await;

    // Stage 6: Recover interrupted owned updates and replay any pending updates.
    prepare_reconnect_updates_on_connect(
        sink,
        state,
        service_id,
        runtime_instance_id,
        has_update_hooks,
        out_seq,
    )
    .await;

    // Stage 7: Spawn the background message processor.
    let processor = MessageProcessor {
        state: Arc::clone(state),
        service_id,
        cert: Some(cert.clone()),
        is_system,
        has_update_tracking,
        has_software_discovery,
        has_update_hooks,
        has_ui_surfaces,
        has_workload_claims,
        runtime_instance_id,
        service_app_name,
        service_tenant_id,
        linked_host_ids: Arc::clone(&linked_host_ids),
        report_tracker: ReportTracker::new(),
    };
    let channels = spawn_message_processor(processor);

    Some(AuthenticatedSessionState {
        service_id,
        connected_at,
        is_system,
        has_update_tracking,
        has_software_discovery,
        has_workload_claims,
        service_tenant_id,
        linked_host_ids,
        push_rx,
        cancel_token,
        msg_tx: channels.msg_tx,
        resp_rx: channels.resp_rx,
        processor_cancel: channels.processor_cancel,
        processor_handle: channels.processor_handle,
        rate_limiter,
    })
}

// ---------------------------------------------------------------------------
// cleanup_authenticated_session
// ---------------------------------------------------------------------------

/// Perform all cleanup after the authenticated loop exits normally (not
/// superseded).
async fn cleanup_authenticated_session(state: &Arc<AppState>, session: AuthenticatedSessionState) {
    let AuthenticatedSessionState {
        service_id,
        is_system,
        has_update_tracking,
        has_software_discovery,
        has_workload_claims,
        service_tenant_id,
        linked_host_ids,
        processor_cancel,
        processor_handle,
        ..
    } = session;

    // Cancel the processor task and wait for it to finish.
    processor_cancel.cancel();
    let _ = processor_handle.await;

    // Release all workload claims held by this service.
    if has_workload_claims {
        workload::release_all_claims_on_disconnect(state, service_id).await;
    }

    // Cleanup must not rely on the session-start UiSurfaces snapshot because
    // services can upgrade capabilities in-session via Register.
    if let Some(provider_id) = state.surface_registry.provider_id_for_service(&service_id) {
        state
            .surface_proxy
            .fail_in_flight_for_provider(&provider_id);
    }
    state.surface_registry.unregister_service(&service_id);

    // Notify services that this agent's hosts are now offline.
    if !is_system
        && !has_update_tracking
        && has_software_discovery
        && let Some(tenant_id) = service_tenant_id
    {
        let current_ids = linked_host_ids.lock().clone();
        if !current_ids.is_empty() {
            let now = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default();
            let updates: Vec<HostConnectivityUpdate> = current_ids
                .iter()
                .map(|&host_id| HostConnectivityUpdate::offline(host_id, Some(now.clone())))
                .collect();
            state
                .notification
                .notification_service
                .send_connectivity_update(tenant_id, updates)
                .await;
        }
    }

    state.service_connections.unregister(&service_id).await;

    // Notify embedded services about the disconnection.
    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_disconnected(&service_id);
    }

    tracing::debug!(%service_id, "authenticated service disconnected");
}

async fn handle_cancelled_authenticated_session_after_close(
    state: &Arc<AppState>,
    session: AuthenticatedSessionState,
) {
    finalize_authenticated_session(state, session).await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthenticatedSessionOwnership {
    Current,
    Replaced,
    Removed,
}

async fn authenticated_session_ownership(
    state: &Arc<AppState>,
    session: &AuthenticatedSessionState,
) -> AuthenticatedSessionOwnership {
    match state
        .service_connections
        .connected_at(&session.service_id)
        .await
    {
        Some(connected_at) if connected_at == session.connected_at => {
            AuthenticatedSessionOwnership::Current
        }
        Some(_) => AuthenticatedSessionOwnership::Replaced,
        None => AuthenticatedSessionOwnership::Removed,
    }
}

async fn finalize_authenticated_session(state: &Arc<AppState>, session: AuthenticatedSessionState) {
    match authenticated_session_ownership(state, &session).await {
        AuthenticatedSessionOwnership::Replaced => {
            let AuthenticatedSessionState {
                processor_cancel,
                processor_handle,
                ..
            } = session;
            processor_cancel.cancel();
            let _ = processor_handle.await;
        }
        AuthenticatedSessionOwnership::Current | AuthenticatedSessionOwnership::Removed => {
            cleanup_authenticated_session(state, session).await;
        }
    }
}

// ---------------------------------------------------------------------------
// handle_authenticated_loop
// ---------------------------------------------------------------------------

/// Action returned by [`handle_incoming_text`] to control the main event loop.
enum TextAction {
    /// Continue to the next iteration (message was handled inline).
    Continue,
    /// Break out of the loop.
    Break,
    /// Break out of the loop after closing the connection for rate limiting.
    RateLimitBreak,
    /// The message was forwarded to the processor; continue the loop.
    Forwarded,
}

/// Handle a deserialized text frame: fast-path messages inline, forward
/// everything else to the processor.
// All parameters originate from the main event loop and cannot be meaningfully
// grouped without duplicating the AuthenticatedSessionState struct.
#[allow(clippy::too_many_arguments)]
async fn handle_incoming_text(
    text: &str,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    session: &mut AuthenticatedSessionState,
    consecutive_unknown: &mut u32,
) -> TextAction {
    let deserialized = match deserialize_service_msg(in_seq, text) {
        Ok(Some(m)) => m,
        Ok(None) => return TextAction::Continue,
        Err(e) => {
            tracing::debug!(error = %e, "deserialize error");
            return TextAction::Break;
        }
    };
    let pagination = deserialized.pagination;
    let service_msg = deserialized.message;

    // Fast-path messages handled inline.
    match &service_msg {
        ServiceMessage::Ping(PingPayload { service_ts, .. }) => {
            if messages::handle_ping(sink, out_seq, state, service_id, *service_ts, is_system)
                .await
                .is_break()
            {
                return TextAction::Break;
            }
            *consecutive_unknown = 0;
            return TextAction::Continue;
        }
        ServiceMessage::Disconnecting(payload) => {
            tracing::info!(
                %service_id,
                reason = ?payload.reason,
                "service disconnecting gracefully"
            );
            return TextAction::Break;
        }
        ServiceMessage::Unknown => {
            *consecutive_unknown += 1;
            tracing::warn!(
                %service_id,
                consecutive_unknown = *consecutive_unknown,
                "received unknown service message type; \
                 ignoring for forward compatibility"
            );
            if *consecutive_unknown >= MAX_CONSECUTIVE_UNKNOWN_MESSAGES {
                tracing::warn!(
                    %service_id,
                    "closing connection: {MAX_CONSECUTIVE_UNKNOWN_MESSAGES} \
                     consecutive unknown messages"
                );
                return TextAction::RateLimitBreak;
            }
            return TextAction::Continue;
        }
        _ => {}
    }

    // Known non-fast-path message: reset unknown counter and forward.
    *consecutive_unknown = 0;
    if session
        .msg_tx
        .send(ProcessorMessage {
            message: service_msg,
            pagination,
        })
        .await
        .is_err()
    {
        tracing::debug!("processor channel closed, breaking main loop");
        return TextAction::Break;
    }
    TextAction::Forwarded
}

/// Unified authenticated handler for all service types.
///
/// Called by [`super::service_ws`] after certificate validation, service status
/// check, and sending `ServiceSettings`. Dispatches incoming messages based on
/// the service's capability set.
///
/// Spawns a [`MessageProcessor`] task for heavy message processing. The main
/// loop handles lightweight inline operations and forwards everything else.
#[tracing::instrument(skip_all, fields(service_id = %ctx.service_id))]
pub(crate) async fn handle_authenticated_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    ctx: AuthenticatedContext<'_>,
) {
    let AuthenticatedContext {
        service_id,
        cert,
        is_system,
        out_seq,
        in_seq,
    } = ctx;

    let Some(mut session) = setup_authenticated_session(
        sink, stream, state, service_id, &cert, is_system, out_seq, in_seq,
    )
    .await
    else {
        return;
    };

    let mut consecutive_unknown: u32 = 0;

    // ------------------------------------------------------------------
    // Main operational loop
    // ------------------------------------------------------------------
    loop {
        tokio::select! {
            // 1. Incoming WebSocket messages
            msg = stream.next() => {
                let Some(msg) = msg else { break };
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "websocket receive error");
                        break;
                    }
                };
                if !session.rate_limiter.allow() {
                    let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
                    break;
                }
                match msg {
                    Message::Text(text) => {
                        match handle_incoming_text(
                            &text, sink, out_seq, in_seq, state, service_id,
                            is_system, &mut session, &mut consecutive_unknown,
                        ).await {
                            TextAction::Continue | TextAction::Forwarded => {}
                            TextAction::Break => break,
                            TextAction::RateLimitBreak => {
                                let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
                                break;
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }

            // 2. Push messages from ServiceConnectionRegistry
            push = session.push_rx.recv() => {
                let Some(msg) = push else { break };
                let Some(json) = serialize_controller_msg(out_seq, msg) else { break };
                if !send_ws_with_timeout(sink, json, service_id).await {
                    break;
                }
            }

            // 3. Responses from the background processor
            resp = session.resp_rx.recv() => {
                let Some(resp) = resp else {
                    tracing::debug!("processor response channel closed");
                    break;
                };

                // Send reply messages
                let mut write_failed = false;
                for reply in resp.replies {
                    let Some(json) = serialize_controller_msg(out_seq, reply) else {
                        write_failed = true;
                        break;
                    };
                    if !send_ws_with_timeout(sink, json, service_id).await {
                        write_failed = true;
                        break;
                    }
                }

                if write_failed {
                    break;
                }

                // Execute the action
                match resp.action {
                    ProcessorAction::Continue => {}
                    ProcessorAction::Break => break,
                    ProcessorAction::CloseWithReason(reason) => {
                        let _ = close_with_reason(sink, reason).await;
                        break;
                    }
                }
            }

            // 4. Connection superseded or force-disconnected
            _ = session.cancel_token.cancelled() => {
                tracing::info!(%service_id, "connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                handle_cancelled_authenticated_session_after_close(state, session).await;
                return;
            }
        }
    }

    // ------------------------------------------------------------------
    // Cleanup
    // ------------------------------------------------------------------
    finalize_authenticated_session(state, session).await;
}

// ---------------------------------------------------------------------------
// upgrade_service_capabilities
// ---------------------------------------------------------------------------

/// Persist the service's current capability set to the database and refresh
/// in-session gating flags.
async fn upgrade_service_capabilities(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    is_system: bool,
    capabilities: std::collections::BTreeSet<Capability>,
    has_ui_surfaces: &mut bool,
) {
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_wire::service_profile::serialize_capabilities;

    let new_caps_json = serialize_capabilities(&capabilities);
    let had_ui_surfaces = *has_ui_surfaces;
    *has_ui_surfaces = capabilities.contains(&Capability::UiSurfaces);

    if had_ui_surfaces != *has_ui_surfaces {
        tracing::info!(
            %service_id,
            ui_surfaces = *has_ui_surfaces,
            "service UiSurfaces capability changed in-session",
        );
    }

    let persist_result = if is_system {
        sys_svc_entity::ActiveModel {
            id: Set(service_id),
            capabilities: Set(new_caps_json),
            ..Default::default()
        }
        .update(db)
        .await
        .map(|_| ())
    } else {
        service::ActiveModel {
            id: Set(service_id),
            capabilities: Set(new_caps_json),
            ..Default::default()
        }
        .update(db)
        .await
        .map(|_| ())
    };

    match persist_result {
        Ok(()) => {
            tracing::debug!(%service_id, "persisted updated service capabilities");
        }
        Err(e) => {
            tracing::warn!(
                %service_id,
                error = %e,
                "failed to persist updated service capabilities"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// EnrolledSessionState
// ---------------------------------------------------------------------------

/// All state produced during enrolled session setup that the main loop and
/// cleanup phases need.
struct EnrolledSessionState {
    push_rx: tokio::sync::mpsc::Receiver<ControllerMessage>,
    cancel_token: tokio_util::sync::CancellationToken,
    approved: bool,
    rate_limiter: MessageRateLimiter,
    approval_poll: tokio::time::Interval,
}

// ---------------------------------------------------------------------------
// setup_enrolled_session
// ---------------------------------------------------------------------------

/// Perform all pre-loop setup for the enrolled handler.
///
/// Loads the service from the DB, registers the connection, detects the
/// external scheduler capability, and checks the initial approval status.
async fn setup_enrolled_session(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
) -> EnrolledSessionState {
    // Fetch service to derive capabilities and app name for registration.
    let (capabilities, service_app_name): (BTreeSet<Capability>, Option<String>) = if is_system {
        match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => (parse_capabilities(&svc.capabilities), svc.service_app_name),
            _ => (BTreeSet::new(), None),
        }
    } else {
        match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => (parse_capabilities(&svc.capabilities), svc.service_app_name),
            _ => (BTreeSet::new(), None),
        }
    };

    // Register in service_connections.
    let (push_rx, connection_handle) = state
        .service_connections
        .register(
            service_id,
            capabilities.clone(),
            None,
            None,
            service_app_name,
        )
        .await;
    let cancel_token = cancellation_token_from_connection_handle(connection_handle);

    // Notify embedded services about the new external connection.
    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_connected(service_id, &capabilities, None, is_system);
    }

    // Check current status to set initial approved flag.
    let mut approved = false;
    if is_system {
        if let Ok(Some(svc)) = sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
            && svc.status == sys_svc_entity::SystemServiceStatus::Approved
        {
            approved = true;
        }
    } else if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
        && svc.status == service::ServiceStatus::Approved
    {
        approved = true;
        emit_service_enrollment_completed_audit_event(state, service_id).await;
    }

    let rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

    // Dedicated interval for polling approval status from the DB.
    let mut approval_poll = tokio::time::interval(APPROVAL_POLL_INTERVAL);
    approval_poll.tick().await; // skip immediate first tick

    EnrolledSessionState {
        push_rx,
        cancel_token,
        approved,
        rate_limiter,
        approval_poll,
    }
}

/// Clean up after an enrolled loop exits normally (not superseded).
async fn cleanup_enrolled_session(
    state: &AppState,
    service_id: uuid::Uuid,
    session: &EnrolledSessionState,
) {
    if session.cancel_token.is_cancelled() {
        return;
    }
    state.service_connections.unregister(&service_id).await;

    // Notify embedded services about the disconnection.
    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_disconnected(&service_id);
    }

    tracing::debug!(%service_id, "enrolled service disconnected");
}

/// Unified enrolled handler for all service types.
///
/// Handles Ping, RequestCertificate, and polls for approval changes at a
/// fixed interval (decoupled from client-controlled ping frequency).
#[tracing::instrument(skip_all, fields(%service_id, is_system))]
pub(crate) async fn handle_enrolled_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) {
    let mut session = setup_enrolled_session(state, service_id, is_system).await;

    loop {
        tokio::select! {
            msg = stream.next() => {
                let Some(msg) = msg else { break };
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "websocket receive error");
                        break;
                    }
                };
                if !session.rate_limiter.allow() {
                    let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
                    break;
                }

                match msg {
                    Message::Text(text) => {
                        let service_msg: ServiceMessage =
                            match deserialize_service_msg(in_seq, &text) {
                                Ok(Some(m)) => m.message,
                                Ok(None) => continue,
                                Err(e) => {
                                    tracing::debug!(error = %e, "deserialize error");
                                    break;
                                }
                            };

                        match service_msg {
                            ServiceMessage::Ping(PingPayload { service_ts, .. }) => {
                                let Ok(controller_ts) =
                                    send_pong(sink, out_seq, service_ts).await
                                else {
                                    break;
                                };
                                tracing::trace!(
                                    service_ts,
                                    controller_ts,
                                    "ping/pong (enrolled)"
                                );
                                let activity_result = if is_system {
                                    record_system_service_activity(
                                        state.db(),
                                        service_id,
                                        None,
                                    )
                                    .await
                                } else {
                                    record_service_activity(state.db(), service_id, None).await
                                };
                                if let Err(e) = activity_result {
                                    tracing::warn!(
                                        error = %e,
                                        %service_id,
                                        "failed to record service activity"
                                    );
                                }
                            }
                            ServiceMessage::RequestCertificate(payload) => {
                                match handle_request_certificate(
                                    sink, state, service_id, is_system,
                                    session.approved, out_seq, &payload,
                                ).await {
                                    CertificateResult::Break => break,
                                    CertificateResult::NotApproved => continue,
                                }
                            }
                            ServiceMessage::Enroll(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "already enrolled".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            ServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %service_id,
                                    reason = ?payload.reason,
                                    "service disconnecting gracefully during enrollment"
                                );
                                break;
                            }
                            ServiceMessage::AuditEvent(payload) => {
                                let _ = ingest_service_audit_event(
                                    state,
                                    service_id,
                                    is_system,
                                    None,
                                    None,
                                    payload,
                                )
                                .await;
                            }
                            _ => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "not available during enrollment".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            push = session.push_rx.recv() => {
                let Some(msg) = push else { break };

                // Track state transitions; handle Rejected specially (send + break).
                let is_rejected = matches!(&msg, ControllerMessage::Rejected(_));
                if matches!(&msg, ControllerMessage::Approved(_)) {
                    session.approved = true;
                }

                let Some(json) = serialize_controller_msg(out_seq, msg) else { break };
                match tokio::time::timeout(
                    WS_WRITE_TIMEOUT,
                    sink.send(Message::Text(json.into())),
                ).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => {
                        tracing::warn!(
                            %service_id,
                            "WebSocket write timed out after {}s during enrollment, dropping connection",
                            WS_WRITE_TIMEOUT.as_secs(),
                        );
                        break;
                    }
                }
                if is_rejected {
                    break;
                }
            }
            // Dedicated approval poll at a fixed interval.
            _ = session.approval_poll.tick(), if !session.approved => {
                match poll_approval_status(sink, state, service_id, is_system, out_seq).await {
                    ApprovalPollResult::Approved => session.approved = true,
                    ApprovalPollResult::Rejected => break,
                    ApprovalPollResult::Unchanged => {}
                }
            }
            _ = session.cancel_token.cancelled() => {
                tracing::info!(%service_id, "enrolled connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                // Same as authenticated: genuine supersession skips cleanup, but
                // force-disconnect removes the registry entry so we must notify.
                if !state.service_connections.is_connected(&service_id).await
                    && let Some(ref notifier) = state.embedded_service_notifier
                {
                    notifier.on_external_disconnected(&service_id);
                }
                return;
            }
        }
    }

    cleanup_enrolled_session(state, service_id, &session).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use uptrakit_wire::surfaces;
    use uuid::Uuid;

    fn test_surface_registration(
        provider_id: &str,
        tenant_id: Uuid,
    ) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: provider_id.to_string(),
                provider_kind: surfaces::ProviderKind::Service,
                provider_namespace: "service".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::TargetedTargeting,
                surfaces::Capability::ProviderInitiatedActions,
                surfaces::Capability::MutationAction,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(tenant_id.to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                    .label("SSH Guest Panel")
                    .priority(100)
                    .slot(surfaces::SLOT_SOFTWARE_TABS)
                    .scope(surfaces::Scope::Tenant)
                    .targeting(surfaces::Targeting::Targeted)
                    .required_permission("view_software")
                    .provider_kind(surfaces::ProviderKind::Service)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::TargetedTargeting,
                        surfaces::Capability::MutationAction,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: "Refresh".to_string(),
                    required_permission: Some("update_software".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: Vec::new(),
                    timeout_seconds: Some(30),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ProviderProxied,
                    workflow_steps: Vec::new(),
                    form_ui: None,
                }],
                data_sources: Vec::new(),
            }],
            encryption_metadata: None,
        }
    }

    struct NoopCertSigner;

    #[async_trait::async_trait]
    impl crate::cert_signer::AgentCertSigner for NoopCertSigner {
        async fn sign_agent_csr(
            &self,
            _: &str,
            _: &uuid::Uuid,
            _: time::Duration,
        ) -> std::result::Result<
            crate::cert_signer::SignedCertBundle,
            Report<crate::cert_signer::CertSignerError>,
        > {
            Err(report!(crate::cert_signer::CertSignerError::Signing(
                "noop signer".to_string(),
            )))
        }

        fn active_ca_fingerprint(&self) -> String {
            "0000000000000000000000000000000000000000000000000000000000000000".to_string()
        }
    }

    async fn build_handler_test_state(
        surface_registry: Arc<crate::surface_registry::SurfaceRegistry>,
        surface_proxy: Arc<crate::surface_proxy::SurfaceProxy>,
    ) -> Arc<AppState> {
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

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
                .expect("test key generation should succeed");
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .expect("test cert params should be valid")
                .self_signed(&key_pair)
                .expect("test certificate should self-sign");
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der())
                        .expect("test private key should parse"),
                )
                .expect("test rustls config should build");
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let db = sea_orm::Database::connect(sea_orm::ConnectOptions::new("sqlite::memory:"))
            .await
            .expect("test db should connect");
        let settings = crate::settings::Settings::new(
            crate::auth::registration::RegistrationSettings {
                mode: crate::auth::registration::RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            168,
        );
        let service_connections = crate::service_connections::ServiceConnectionRegistry::new();
        let controller_id = Uuid::nil();
        let notification_service = crate::notification_service::NotificationService::new(
            service_connections.clone(),
            controller_id,
        );
        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            )
            .expect("catalog should build in tests"),
        );
        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

        Arc::new(AppState {
            db: crate::app_state::DbState::new(db.clone()),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState {
                jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-secret-handler",
                )),
                device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
                rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
                token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            },
            notification: crate::app_state::NotificationState {
                notification_service,
                notification_dispatcher,
                event_broadcaster: crate::event_broadcaster::EventBroadcaster::new(),
            },
            broadcast: crate::app_state::BroadcastState {
                device_flow_broadcaster: crate::device_flow_broadcaster::DeviceFlowBroadcaster::new(
                ),
                update_output_broadcaster:
                    crate::update_output_broadcaster::UpdateOutputBroadcaster::new(),
                batch_progress_broadcaster:
                    crate::batch_progress_broadcaster::BatchProgressBroadcaster::new(),
            },
            #[cfg(feature = "oidc")]
            oidc: crate::app_state::OidcState {
                oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
                account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
                oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                    db.clone(),
                ),
                oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                    db.clone(),
                ),
            },
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections,
            plugin_ops,
            global_providers: Arc::new(crate::global_providers::GlobalProviders::new(db.clone())),
            credential_sources: crate::ServiceCredentialSources::default(),
            shutdown_token: Default::default(),
            embedded_service_notifier: None,
            audit_log_filter: uptrakit_audit_log::AuditFilter::default(),
            audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                uptrakit_audit_log::NoopBackend,
            )),
            audit_emitter: uptrakit_audit_log::AuditEmitter::new(
                uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                    uptrakit_audit_log::NoopBackend,
                )),
            ),
            surface_registry,
            surface_proxy,
            config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
            workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            default_tenant_id: Uuid::nil(),
            controller_id,
            reject_dangerous_commands: false,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
        })
    }

    #[cfg(feature = "db-sqlite")]
    fn test_authenticated_session(
        service_id: Uuid,
        connected_at: time::OffsetDateTime,
    ) -> AuthenticatedSessionState {
        let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
        let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
        AuthenticatedSessionState {
            service_id,
            connected_at,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_workload_claims: false,
            service_tenant_id: None,
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            push_rx,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            msg_tx,
            resp_rx,
            processor_cancel: tokio_util::sync::CancellationToken::new(),
            processor_handle: tokio::spawn(async {}),
            rate_limiter: MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT),
        }
    }

    #[cfg(feature = "db-sqlite")]
    fn register_test_runtime_state(state: &Arc<AppState>, service_id: Uuid, tenant_id: Uuid) {
        state
            .surface_registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                test_surface_registration("provider-a", tenant_id),
            )
            .expect("surface registration should succeed");
    }

    #[cfg(feature = "db-sqlite")]
    async fn register_test_connection(
        state: &Arc<AppState>,
        service_id: Uuid,
    ) -> time::OffsetDateTime {
        let capabilities = BTreeSet::from([Capability::UiSurfaces]);
        let _ = state
            .service_connections
            .register(
                service_id,
                capabilities,
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        state
            .service_connections
            .connected_at(&service_id)
            .await
            .expect("connected_at should exist after registration")
    }

    #[test]
    fn surface_registration_error_message_serializes_structured_rejection_reasons() {
        let message = surface_registration_error_message(
            &crate::surface_registry::SurfaceRegistryError::ProviderRejected(
                crate::surface_registry::SurfaceProviderRejection {
                    provider_id: "provider-a".to_string(),
                    reasons: vec![crate::surface_registry::SurfaceProviderRejectionReason {
                        code:
                            crate::surface_registry::SurfaceProviderRejectionCode::InvalidTransport,
                        message: "invalid transport".to_string(),
                        surface_id: Some("ssh.guest.panel".to_string()),
                    }],
                },
            ),
        );

        let parsed: serde_json::Value =
            serde_json::from_str(&message).expect("expected JSON rejection payload");
        assert_eq!(parsed["provider_id"], "provider-a");
        assert_eq!(parsed["reasons"][0]["message"], "invalid transport");
    }

    #[test]
    fn system_service_tenant_binding_only_targets_mqtt() {
        let tenant_id = uuid::Uuid::now_v7();
        assert_eq!(
            system_service_tenant_binding(Some("uptrakit-mqtt"), tenant_id),
            Some(tenant_id)
        );
        assert_eq!(
            system_service_tenant_binding(Some("uptrakit-scheduler"), tenant_id),
            None
        );
        assert_eq!(system_service_tenant_binding(None, tenant_id), None);
    }

    #[test]
    fn service_config_scope_validation_requires_exact_tenant_for_bound_sessions() {
        let tenant_id = uuid::Uuid::now_v7();
        assert!(is_valid_service_config_scope(
            Some(tenant_id),
            Some(tenant_id)
        ));
        assert!(!is_valid_service_config_scope(Some(tenant_id), None));
        assert!(!is_valid_service_config_scope(
            Some(tenant_id),
            Some(uuid::Uuid::now_v7())
        ));
        assert!(is_valid_service_config_scope(None, None));
        assert!(is_valid_service_config_scope(None, Some(tenant_id)));
    }

    #[test]
    fn surface_action_target_display_includes_surface_and_interaction() {
        let surface_id = surfaces::SurfaceId::new("notifications.email").unwrap();
        let interaction_id = surfaces::InteractionId::new("configure_smtp").unwrap();

        assert_eq!(
            surface_action_target_display(&surface_id, &interaction_id),
            "notifications.email/configure_smtp"
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn store_service_config_scope_violation_emits_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-mqtt".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::StoreServiceConfig(uptrakit_wire::StoreServiceConfigPayload::new(
                    "req-store-denied".to_string(),
                    None,
                    "clients.primary".to_string(),
                    serde_json::json!({"enabled": true}),
                    true,
                )),
                None,
            )
            .await;

        let [ControllerMessage::ServiceConfigAck(ack)] = response.replies.as_slice() else {
            panic!("expected exactly one ServiceConfigAck reply");
        };
        assert_eq!(ack.request_id, "req-store-denied");
        assert!(!ack.success);
        assert_eq!(
            ack.error.as_deref(),
            Some("service cannot write config outside its tenant binding")
        );

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_STORE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.request_id.as_deref(), Some("req-store-denied"));
        assert_eq!(row.target_type.as_deref(), Some("service_config"));
        assert_eq!(row.target_display.as_deref(), Some("clients.primary"));
        let details = row
            .details_json
            .as_ref()
            .expect("scope denial audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["requested_scope"], "global");
        assert_eq!(details["service_tenant_id"], tenant_id.to_string());
        assert_eq!(details["requested_tenant_id"], serde_json::Value::Null);
        assert_eq!(details["reason_code"], "outside_tenant_binding");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn delete_service_config_scope_violation_emits_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        let requested_tenant_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-mqtt".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::DeleteServiceConfig(
                    uptrakit_wire::DeleteServiceConfigPayload::new(
                        "req-delete-denied".to_string(),
                        Some(requested_tenant_id),
                        "clients.primary".to_string(),
                    ),
                ),
                None,
            )
            .await;

        let [ControllerMessage::ServiceConfigAck(ack)] = response.replies.as_slice() else {
            panic!("expected exactly one ServiceConfigAck reply");
        };
        assert_eq!(ack.request_id, "req-delete-denied");
        assert!(!ack.success);
        assert_eq!(
            ack.error.as_deref(),
            Some("service cannot delete config outside its tenant binding")
        );

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.request_id.as_deref(), Some("req-delete-denied"));
        assert_eq!(row.target_type.as_deref(), Some("service_config"));
        assert_eq!(row.target_display.as_deref(), Some("clients.primary"));
        let details = row
            .details_json
            .as_ref()
            .expect("scope denial audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["requested_scope"], "tenant");
        assert_eq!(details["service_tenant_id"], tenant_id.to_string());
        assert_eq!(
            details["requested_tenant_id"],
            requested_tenant_id.to_string()
        );
        assert_eq!(details["reason_code"], "outside_tenant_binding");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_scope_violation_emits_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let requested_tenant_id = Uuid::now_v7();
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-mqtt".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: requested_tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("notifications.email").unwrap(),
                interaction_id: surfaces::InteractionId::new("configure_smtp").unwrap(),
                idempotency_key: "scope-violation".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::from_iter([(
                    "host".to_string(),
                    serde_json::Value::String("smtp.example.invalid".to_string()),
                )]),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(
            error.code,
            surfaces::SurfaceActionErrorCode::PermissionDenied
        );
        assert_eq!(
            error.message,
            "service cannot invoke actions outside its tenant"
        );

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(row.target_id, None);
        assert_eq!(
            row.target_display.as_deref(),
            Some("notifications.email/configure_smtp")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("scope denial audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["surface_id"], "notifications.email");
        assert_eq!(details["interaction_id"], "configure_smtp");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["service_tenant_id"], tenant_id.to_string());
        assert_eq!(
            details["requested_tenant_id"],
            requested_tenant_id.to_string()
        );
        assert_eq!(details["reason_code"], "outside_tenant_binding");
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_invalid_payload_emits_validation_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "x".repeat(MAX_SHORT_STRING_LEN + 1),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(error.code, surfaces::SurfaceActionErrorCode::InvalidRequest);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(
            row.target_display.as_deref(),
            Some("ssh.guest.panel/refresh")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("invalid payload audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["reason_code"], "invalid_request");
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_invalid_tenant_emits_validation_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: "not-a-uuid".to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "invalid-tenant".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(error.code, surfaces::SurfaceActionErrorCode::InvalidRequest);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        let details = row
            .details_json
            .as_ref()
            .expect("invalid tenant audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["reason_code"], "invalid_tenant_id");
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_lookup_failure_emits_validation_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.surfaces[0].interactions[0].required_permission = None;
        state
            .surface_registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                registration,
            )
            .expect("surface registration should succeed");
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "lookup-failure".to_string(),
                target_provider_id: Some("missing-provider".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(error.code, surfaces::SurfaceActionErrorCode::InvalidRequest);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        let details = row
            .details_json
            .as_ref()
            .expect("lookup failure audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "missing-provider");
        assert_eq!(details["reason_code"], "invalid_provider");
        assert!(details.get("provider_kind").is_none());
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_success_emits_success_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.surfaces[0].interactions[0].required_permission = None;
        state
            .surface_registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                registration,
            )
            .expect("surface registration should succeed");
        let (mut rx, _cancel) = state
            .service_connections
            .register(
                service_id,
                BTreeSet::from([Capability::UiSurfaces]),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        let proxy = Arc::clone(&state.surface_proxy);
        tokio::spawn(async move {
            if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
                proxy.complete(
                    request.request_id,
                    surfaces::SurfaceActionResponse {
                        request_id: request.request_id,
                        success: true,
                        result: Some(serde_json::json!({"ok": true})),
                        error: None,
                    },
                );
            }
        });
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "surface-success".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(reply.success);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(
            row.target_display.as_deref(),
            Some("ssh.guest.panel/refresh")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("success audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["provider_service_app_name"], "uptrakit-agent-ssh");
        assert!(details.get("reason_code").is_none());
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_provider_unavailable_emits_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.surfaces[0].interactions[0].required_permission = None;
        state
            .surface_registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                registration,
            )
            .expect("surface registration should succeed");
        let (rx, _cancel) = state
            .service_connections
            .register(
                service_id,
                BTreeSet::from([Capability::UiSurfaces]),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        drop(rx);
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "surface-provider-unavailable".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(
            error.code,
            surfaces::SurfaceActionErrorCode::ProviderUnavailable
        );

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        let details = row
            .details_json
            .as_ref()
            .expect("failed audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["provider_service_app_name"], "uptrakit-agent-ssh");
        assert_eq!(details["reason_code"], "provider_unavailable");
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    async fn insert_test_service_row(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        service_id: Uuid,
        service_app_name: &str,
    ) {
        use sea_orm::{ActiveModelTrait, Set};

        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("svc-{service_id}")),
            friendly_name: Set(format!("Service {service_id}")),
            ip_address: Set(None),
            status: Set(uptrakit_shared_types::ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(Some(service_app_name.to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[cfg(feature = "db-sqlite")]
    async fn build_db_audited_state(
        db: sea_orm::DatabaseConnection,
        tenant_id: Uuid,
    ) -> Arc<AppState> {
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        state
    }

    #[cfg(feature = "db-sqlite")]
    async fn insert_test_system_service_row(
        db: &sea_orm::DatabaseConnection,
        service_id: Uuid,
        service_app_name: &str,
    ) {
        use sea_orm::{ActiveModelTrait, Set};

        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::system_service::ActiveModel {
            id: Set(service_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("sys-{service_id}")),
            friendly_name: Set(format!("System Service {service_id}")),
            ip_address: Set(None),
            status: Set(uptrakit_shared_db::entity::system_service::SystemServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            cert_lifetime_hours: Set(None),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(Some(service_app_name.to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[cfg(feature = "db-sqlite")]
    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> uptrakit_shared_db::entity::audit_log::Model {
        use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

        for _ in 0..50 {
            if let Some(row) = uptrakit_shared_db::entity::audit_log::Entity::find()
                .filter(uptrakit_shared_db::entity::audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(uptrakit_shared_db::entity::audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    #[cfg(feature = "db-sqlite")]
    async fn system_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> uptrakit_shared_db::entity::system_audit_log::Model {
        use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

        for _ in 0..50 {
            if let Some(row) = uptrakit_shared_db::entity::system_audit_log::Entity::find()
                .filter(
                    uptrakit_shared_db::entity::system_audit_log::Column::ActionType
                        .eq(action_type),
                )
                .order_by_desc(uptrakit_shared_db::entity::system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected system audit row for action {action_type}");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn forwarded_service_audit_event_writes_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::AuditEvent(AuditEventPayload {
                    action_type: uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STARTED
                        .to_string(),
                    tenant_id: Some(tenant_id.to_string()),
                    target_type: Some("update_history".to_string()),
                    target_id: Some(Uuid::now_v7().to_string()),
                    target_display: Some("nginx on node-1".to_string()),
                    outcome: uptrakit_audit_log::AuditOutcome::Success
                        .as_str()
                        .to_string(),
                    details_json: Some(serde_json::json!({ "interactive": false }).to_string()),
                    request_id: None,
                }),
                None,
            )
            .await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STARTED,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("update_history"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn forwarded_runtime_audit_event_from_tenant_service_writes_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::AuditEvent(AuditEventPayload {
                    action_type: uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_UPDATE_GATE
                        .to_string(),
                    tenant_id: Some(tenant_id.to_string()),
                    target_type: None,
                    target_id: None,
                    target_display: None,
                    outcome: uptrakit_audit_log::AuditOutcome::Denied
                        .as_str()
                        .to_string(),
                    details_json: Some(
                        serde_json::json!({
                            "message_name": "ExecuteUpdate",
                            "gate": "freeze",
                        })
                        .to_string(),
                    ),
                    request_id: None,
                }),
                None,
            )
            .await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_UPDATE_GATE,
        )
        .await;
        assert_eq!(row.tenant_id, tenant_id);
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn forwarded_scheduler_audit_event_from_system_service_writes_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_system_service_row(&db, service_id, "uptrakit-scheduler").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: true,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-scheduler".to_string()),
            service_tenant_id: None,
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::AuditEvent(AuditEventPayload {
                    action_type:
                        uptrakit_audit_log::AuditActionType::SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP
                            .to_string(),
                    tenant_id: None,
                    target_type: None,
                    target_id: None,
                    target_display: None,
                    outcome: uptrakit_audit_log::AuditOutcome::Success
                        .as_str()
                        .to_string(),
                    details_json: Some(
                        serde_json::json!({
                            "tenant_deleted": 1,
                            "system_deleted": 2,
                            "retention_days": 90,
                        })
                        .to_string(),
                    ),
                    request_id: None,
                }),
                None,
            )
            .await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP,
        )
        .await;
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn local_service_certificate_issue_audit_event_writes_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;

        emit_service_certificate_issue_audit_event(
            &state,
            service_id,
            time::OffsetDateTime::now_utc() + time::Duration::days(30),
        )
        .await;

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_ISSUE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn local_system_service_certificate_issue_audit_event_writes_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_system_service_row(&db, service_id, "uptrakit-scheduler").await;

        emit_service_certificate_issue_audit_event(
            &state,
            service_id,
            time::OffsetDateTime::now_utc() + time::Duration::days(30),
        )
        .await;

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_ISSUE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn local_system_service_certificate_renew_audit_event_writes_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_system_service_row(&db, service_id, "uptrakit-scheduler").await;

        emit_service_certificate_renew_audit_event(
            &state,
            service_id,
            true,
            time::OffsetDateTime::now_utc() + time::Duration::days(30),
        )
        .await;

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn setup_enrolled_session_emits_enrollment_completed_audit_for_already_approved_service()
    {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;

        let session = setup_enrolled_session(&state, service_id, false).await;
        assert!(session.approved);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn local_service_enrollment_completed_audit_event_writes_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;

        emit_service_enrollment_completed_audit_event(&state, service_id).await;

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn invalid_forwarded_service_audit_event_is_dropped_without_disconnect() {
        let surface_registry = Arc::new(crate::surface_registry::SurfaceRegistry::new(
            crate::surface_registry::SurfaceRegistryConfig::default(),
        ));
        let surface_proxy = Arc::new(crate::surface_proxy::SurfaceProxy::new());
        let state = build_handler_test_state(surface_registry, surface_proxy).await;
        let mut processor = MessageProcessor {
            state,
            service_id: Uuid::now_v7(),
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent".to_string()),
            service_tenant_id: Some(Uuid::now_v7()),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::AuditEvent(AuditEventPayload {
                    action_type: "auth.login.failed".to_string(),
                    tenant_id: Some(Uuid::now_v7().to_string()),
                    target_type: None,
                    target_id: None,
                    target_display: None,
                    outcome: "validation_failed".to_string(),
                    details_json: None,
                    request_id: None,
                }),
                None,
            )
            .await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn invalid_surface_registration_emits_validation_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.effective_tenant_binding.tenant_id = None;

        let response = processor.handle_surface_registration(registration).await;

        let [ControllerMessage::Error(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one Error reply");
        };
        assert_eq!(reply.code, ErrorCode::BadRequest);
        assert!(reply.message.contains("invalid surface registration"));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_provider"));
        assert_eq!(row.target_id.as_deref(), Some("provider-a"));
        assert_eq!(row.target_display.as_deref(), Some("provider-a"));
        let details = row
            .details_json
            .as_ref()
            .expect("validation failure audit should include details");
        assert_eq!(details["provider_id"], "provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["framework_generation"], "1.0");
        assert_eq!(details["capability_count"], 4);
        assert_eq!(details["surface_count"], 1);
        assert_eq!(details["reason_code"], "invalid_tenant_binding");
        assert!(details.get("surfaces").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn incompatible_surface_registration_emits_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.framework_generation = surfaces::FrameworkGeneration::new(2, 0);

        let response = processor.handle_surface_registration(registration).await;

        let [ControllerMessage::Error(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one Error reply");
        };
        assert_eq!(reply.code, ErrorCode::BadRequest);
        assert!(reply.message.contains("UnsupportedGeneration"));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_provider"));
        assert_eq!(row.target_id.as_deref(), Some("provider-a"));
        assert_eq!(row.target_display.as_deref(), Some("provider-a"));
        let details = row
            .details_json
            .as_ref()
            .expect("rejection audit should include details");
        assert_eq!(details["provider_id"], "provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["framework_generation"], "2.0");
        assert_eq!(details["capability_count"], 4);
        assert_eq!(details["surface_count"], 1);
        assert_eq!(details["reason_code"], "unsupported_generation");
        assert!(details.get("surfaces").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn successful_system_surface_registration_emits_success_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_system_service_row(&db, service_id, "uptrakit-scheduler").await;
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: true,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-scheduler".to_string()),
            service_tenant_id: None,
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let mut registration = test_surface_registration("provider-system", tenant_id);
        registration.effective_tenant_binding.scope = surfaces::Scope::Global;
        registration.effective_tenant_binding.tenant_id = None;

        let response = processor.handle_surface_registration(registration).await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-scheduler"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_provider"));
        assert_eq!(row.target_id.as_deref(), Some("provider-system"));
        assert_eq!(row.target_display.as_deref(), Some("provider-system"));
        let details = row
            .details_json
            .as_ref()
            .expect("success audit should include details");
        assert_eq!(details["provider_id"], "provider-system");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["framework_generation"], "1.0");
        assert_eq!(details["capability_count"], 4);
        assert_eq!(details["surface_count"], 1);
        assert!(details.get("reason_code").is_none());
        assert!(details.get("surfaces").is_none());
    }

    #[tokio::test]
    async fn surface_action_request_is_blocked_until_rollout_is_active() {
        let tenant_id = Uuid::now_v7();
        let rollout = crate::app_state::SurfaceRuntimeRolloutState::phase0(
            true,
            crate::app_state::default_surface_runtime_requirements(false),
            std::collections::BTreeMap::new(),
        );
        rollout.set_local_requirement_satisfied(crate::app_state::SURFACE_PROVIDER_APP_MQTT, true);

        let surface_registry = Arc::new(crate::surface_registry::SurfaceRegistry::new(
            crate::surface_registry::SurfaceRegistryConfig::default(),
        ));
        let surface_proxy = Arc::new(crate::surface_proxy::SurfaceProxy::new());
        let service_id = Uuid::now_v7();
        let state =
            build_handler_test_state(Arc::clone(&surface_registry), Arc::clone(&surface_proxy))
                .await;
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.surfaces[0].interactions[0].required_permission = None;
        surface_registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                registration,
            )
            .expect("surface registration should succeed");

        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id: Uuid::now_v7(),
                tenant_id: tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "inactive-rollout".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(response)] = response.replies.as_slice()
        else {
            panic!("expected a single surface action response");
        };
        assert!(!response.success);
        let error = response.error.as_ref().expect("error payload should exist");
        assert_eq!(
            error.code,
            surfaces::SurfaceActionErrorCode::ProviderUnavailable
        );
    }

    #[cfg(feature = "db-sqlite")]
    mod db_sqlite {
        use super::*;
        use std::collections::BTreeMap;

        use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
        use time::OffsetDateTime;
        use tokio_util::sync::CancellationToken;
        use uptrakit_wire::{DisconnectReason, DisconnectingPayload, RegisterPayload};

        use crate::embedded_support::EmbeddedServiceNotifier;
        use uptrakit_shared_db::entity::{
            host, service, service_host, software_item, update_history,
        };
        use uptrakit_shared_types::ServiceStatus;

        #[derive(Default)]
        struct MockEmbeddedNotifier {
            disconnected: parking_lot::Mutex<Vec<Uuid>>,
        }

        impl EmbeddedServiceNotifier for MockEmbeddedNotifier {
            fn on_external_connected(
                &self,
                _service_id: Uuid,
                _capabilities: &BTreeSet<Capability>,
                _hostname: Option<&str>,
                _is_system: bool,
            ) {
            }

            fn on_external_disconnected(&self, service_id: &Uuid) {
                self.disconnected.lock().push(*service_id);
            }

            fn on_machine_id_reported(&self, _service_id: &Uuid, _machine_id: &str) {}

            fn is_capability_yielded(&self, _capability: &Capability) -> bool {
                false
            }
        }

        async fn insert_service_row(
            db: &sea_orm::DatabaseConnection,
            tenant_id: Uuid,
            service_id: Uuid,
            service_app_name: &str,
        ) {
            let now = OffsetDateTime::now_utc();
            service::ActiveModel {
                id: Set(service_id),
                tenant_id: Set(tenant_id),
                capabilities: Set("[]".to_string()),
                hostname: Set(format!("svc-{service_id}")),
                friendly_name: Set(format!("Service {service_id}")),
                ip_address: Set(None),
                status: Set(ServiceStatus::Approved),
                enrollment_secret_hash: Set(format!("secret-{service_id}")),
                client_version: Set(None),
                last_seen_at: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
                deactivated_at: Set(None),
                ping_interval_seconds: Set(None),
                enrollment_token_id: Set(None),
                cert_lifetime_hours: Set(None),
                service_app_name: Set(Some(service_app_name.to_string())),
                is_embedded: Set(false),
                embedded_owner_key: Set(None),
            }
            .insert(db)
            .await
            .unwrap();
        }

        async fn insert_linked_host_and_item(
            db: &sea_orm::DatabaseConnection,
            tenant_id: Uuid,
            service_id: Uuid,
        ) -> (Uuid, Uuid) {
            let now = OffsetDateTime::now_utc();
            insert_service_row(db, tenant_id, service_id, "uptrakit-agent").await;

            let host_id = host::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                machine_id: Set(format!("machine-{service_id}")),
                hostname: Set(format!("host-{service_id}")),
                friendly_name: Set(format!("Host {service_id}")),
                os_type: Set(None),
                os_version: Set(None),
                architecture: Set(None),
                ip_address: Set(None),
                host_features: Set(None),
                last_seen_at: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
                deactivated_at: Set(None),
            }
            .insert(db)
            .await
            .unwrap()
            .id;

            let software_item_id = software_item::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                name: Set("demo".to_string()),
                featured: Set(false),
                icon_url: Set(None),
                last_checked_at: Set(None),
                deactivated_at: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(db)
            .await
            .unwrap()
            .id;

            service_host::ActiveModel {
                service_id: Set(service_id),
                host_id: Set(host_id),
                linked_at: Set(now),
            }
            .insert(db)
            .await
            .unwrap();

            (host_id, software_item_id)
        }

        async fn relink_service_host(
            db: &sea_orm::DatabaseConnection,
            service_id: Uuid,
            host_id: Uuid,
        ) {
            service_host::ActiveModel {
                service_id: Set(service_id),
                host_id: Set(host_id),
                linked_at: Set(OffsetDateTime::now_utc()),
            }
            .insert(db)
            .await
            .unwrap();
        }

        async fn insert_owned_in_progress_update(
            db: &sea_orm::DatabaseConnection,
            tenant_id: Uuid,
            host_id: Uuid,
            software_item_id: Uuid,
            owner_service_id: Uuid,
            owner_instance_id: Option<Uuid>,
        ) -> Uuid {
            let now = OffsetDateTime::now_utc();
            let id = Uuid::now_v7();
            update_history::ActiveModel {
                id: Set(id),
                tenant_id: Set(tenant_id),
                host_id: Set(host_id),
                software_item_id: Set(software_item_id),
                host_software_item_id: Set(None),
                from_version: Set(Some("1.0.0".to_string())),
                to_version: Set(Some("1.1.0".to_string())),
                status: Set(update_history::UpdateStatus::InProgress),
                output: Set(String::new()),
                output_bytes: Set(0),
                actor_type: Set("user".to_string()),
                actor_id: Set(String::new()),
                execution_owner_service_id: Set(Some(owner_service_id)),
                execution_owner_instance_id: Set(owner_instance_id),
                started_at: Set(Some(now)),
                completed_at: Set(None),
                created_at: Set(now),
                update_category: Set("security".to_string()),
                batch_id: Set(None),
                interactive: Set(false),
                output_truncated: Set(false),
                pre_update_protection_status: Set(None),
                pre_update_protection_summary: Set(None),
                recovery_hint: Set(None),
            }
            .insert(db)
            .await
            .unwrap();
            id
        }

        async fn run_embedded_register_once(
            state: Arc<AppState>,
            service_id: Uuid,
            tenant_id: Uuid,
            capabilities: BTreeSet<Capability>,
            runtime_instance_id: Uuid,
        ) {
            let _ = state
                .service_connections
                .register(
                    service_id,
                    capabilities.clone(),
                    None,
                    None,
                    Some("uptrakit-agent".to_string()),
                )
                .await;

            let (service_tx, service_rx) = tokio::sync::mpsc::channel(4);
            let cancel = CancellationToken::new();
            let handler_capabilities = capabilities.clone();
            let handle = tokio::spawn(async move {
                run_embedded_message_handler(
                    Arc::clone(&state),
                    service_id,
                    tenant_id,
                    &handler_capabilities,
                    "uptrakit-agent",
                    service_rx,
                    cancel.clone(),
                )
                .await
            });

            service_tx
                .send(ServiceMessage::Register(
                    RegisterPayload::new(capabilities.clone())
                        .with_runtime_instance_id(runtime_instance_id),
                ))
                .await
                .unwrap();
            service_tx
                .send(ServiceMessage::Disconnecting(DisconnectingPayload::new(
                    DisconnectReason::Shutdown,
                )))
                .await
                .unwrap();

            handle.await.unwrap();
        }

        #[tokio::test]
        async fn embedded_system_handler_cleanup_releases_claims_and_unregisters_state() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let notifier = Arc::new(MockEmbeddedNotifier::default());
            let state = Arc::new(AppState {
                embedded_service_notifier: Some(notifier.clone()),
                ..(*state).clone()
            });

            let service_id = Uuid::now_v7();
            let mqtt_capabilities: BTreeSet<Capability> = [
                Capability::SystemService,
                Capability::UiSurfaces,
                Capability::WorkloadClaims,
            ]
            .into_iter()
            .collect();
            let _ = state
                .service_connections
                .register(
                    service_id,
                    mqtt_capabilities.clone(),
                    None,
                    None,
                    Some("uptrakit-mqtt".to_string()),
                )
                .await;

            state
                .surface_registry
                .register_service(
                    service_id,
                    "uptrakit-mqtt",
                    Some(tenant_id),
                    test_surface_registration("provider-mqtt", tenant_id),
                )
                .expect("service surface registration should succeed");

            let claim_key = format!("clients.{}", Uuid::now_v7());
            let claim_result = state.workload_claim_registry.try_claim(
                service_id,
                state.controller_id,
                BTreeMap::from([(claim_key.clone(), tenant_id)]),
            );
            assert!(claim_result.granted.contains(&claim_key));
            assert!(state.service_connections.is_connected(&service_id).await);
            assert_eq!(
                state.surface_registry.provider_id_for_service(&service_id),
                Some("provider-mqtt".to_string())
            );

            let (service_tx, service_rx) = tokio::sync::mpsc::channel(1);
            drop(service_tx);

            run_embedded_system_message_handler(
                state.clone(),
                service_id,
                Some(tenant_id),
                &mqtt_capabilities,
                "uptrakit-mqtt",
                service_rx,
                CancellationToken::new(),
            )
            .await;

            assert!(!state.service_connections.is_connected(&service_id).await);
            assert!(
                state
                    .surface_registry
                    .provider_id_for_service(&service_id)
                    .is_none()
            );
            assert!(
                state
                    .workload_claim_registry
                    .service_claims(service_id)
                    .is_empty()
            );
            assert_eq!(*notifier.disconnected.lock(), vec![service_id]);
        }

        #[tokio::test]
        async fn cleanup_authenticated_session_unregisters_runtime_state_even_with_stale_ui_snapshot()
         {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);

            assert_eq!(
                state.surface_registry.provider_id_for_service(&service_id),
                Some("provider-a".to_string())
            );

            cleanup_authenticated_session(
                &state,
                test_authenticated_session(service_id, time::OffsetDateTime::now_utc()),
            )
            .await;

            assert!(
                state
                    .surface_registry
                    .provider_id_for_service(&service_id)
                    .is_none(),
                "surface provider should be removed on disconnect"
            );
        }

        #[tokio::test]
        async fn reconnect_cleanup_same_instance_leaves_owned_update_in_progress() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let service_id = Uuid::now_v7();
            let runtime_id = Uuid::now_v7();
            let capabilities: BTreeSet<Capability> =
                [Capability::SoftwareDiscovery, Capability::UpdateHooks]
                    .into_iter()
                    .collect();
            let (host_id, software_item_id) =
                insert_linked_host_and_item(state.db(), tenant_id, service_id).await;
            let update_history_id = insert_owned_in_progress_update(
                state.db(),
                tenant_id,
                host_id,
                software_item_id,
                service_id,
                Some(runtime_id),
            )
            .await;

            run_embedded_register_once(
                Arc::clone(&state),
                service_id,
                tenant_id,
                capabilities,
                runtime_id,
            )
            .await;

            let row = update_history::Entity::find_by_id(update_history_id)
                .one(state.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, update_history::UpdateStatus::InProgress);
        }

        #[tokio::test]
        async fn reconnect_cleanup_new_instance_fails_prior_owned_update_even_without_host_links() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let service_id = Uuid::now_v7();
            let old_runtime_id = Uuid::now_v7();
            let new_runtime_id = Uuid::now_v7();
            let capabilities: BTreeSet<Capability> =
                [Capability::SoftwareDiscovery, Capability::UpdateHooks]
                    .into_iter()
                    .collect();
            let (host_id, software_item_id) =
                insert_linked_host_and_item(state.db(), tenant_id, service_id).await;
            let update_history_id = insert_owned_in_progress_update(
                state.db(),
                tenant_id,
                host_id,
                software_item_id,
                service_id,
                Some(old_runtime_id),
            )
            .await;

            service_host::Entity::delete_many()
                .filter(service_host::Column::ServiceId.eq(service_id))
                .exec(state.db())
                .await
                .unwrap();

            run_embedded_register_once(
                Arc::clone(&state),
                service_id,
                tenant_id,
                capabilities,
                new_runtime_id,
            )
            .await;

            let row = update_history::Entity::find_by_id(update_history_id)
                .one(state.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, update_history::UpdateStatus::Failed);
            assert_eq!(row.output, "Update interrupted: agent restarted");
        }

        #[tokio::test]
        async fn connect_phase_does_not_fail_update_owned_by_different_linked_service() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let owner_service_id = Uuid::now_v7();
            let reconnecting_service_id = Uuid::now_v7();
            let old_runtime_id = Uuid::now_v7();
            let new_runtime_id = Uuid::now_v7();
            let (host_id, software_item_id) =
                insert_linked_host_and_item(state.db(), tenant_id, owner_service_id).await;
            insert_service_row(
                state.db(),
                tenant_id,
                reconnecting_service_id,
                "uptrakit-agent",
            )
            .await;
            relink_service_host(state.db(), reconnecting_service_id, host_id).await;
            let update_history_id = insert_owned_in_progress_update(
                state.db(),
                tenant_id,
                host_id,
                software_item_id,
                owner_service_id,
                Some(old_runtime_id),
            )
            .await;

            updates::recover_owned_updates_on_connect_with_dispatch_mode(
                &state,
                reconnecting_service_id,
                Some(new_runtime_id),
                updates::ReconnectSuccessorDispatchMode::Immediate,
            )
            .await
            .unwrap();
            let _ = updates::load_pending_update_records(&state, reconnecting_service_id)
                .await
                .unwrap();

            let row = update_history::Entity::find_by_id(update_history_id)
                .one(state.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, update_history::UpdateStatus::InProgress);
        }

        #[tokio::test]
        async fn cancelled_authenticated_session_cleans_runtime_state_after_force_disconnect() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            let connected_at = register_test_connection(&state, service_id).await;
            state
                .service_connections
                .force_disconnect(&service_id)
                .await;

            handle_cancelled_authenticated_session_after_close(
                &state,
                test_authenticated_session(service_id, connected_at),
            )
            .await;

            assert!(
                state
                    .surface_registry
                    .provider_id_for_service(&service_id)
                    .is_none()
            );
        }

        #[tokio::test]
        async fn cancelled_authenticated_session_skips_runtime_cleanup_for_genuine_supersession() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            let superseded_connected_at = register_test_connection(&state, service_id).await;
            let _replacement_connected_at = register_test_connection(&state, service_id).await;

            handle_cancelled_authenticated_session_after_close(
                &state,
                test_authenticated_session(service_id, superseded_connected_at),
            )
            .await;

            assert_eq!(
                state.surface_registry.provider_id_for_service(&service_id),
                Some("provider-a".to_string())
            );
            assert!(state.service_connections.is_connected(&service_id).await);
        }

        #[tokio::test]
        async fn finalized_authenticated_session_skips_runtime_cleanup_when_session_is_replaced() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            let superseded_connected_at = register_test_connection(&state, service_id).await;
            let _replacement_connected_at = register_test_connection(&state, service_id).await;

            finalize_authenticated_session(
                &state,
                test_authenticated_session(service_id, superseded_connected_at),
            )
            .await;

            assert_eq!(
                state.surface_registry.provider_id_for_service(&service_id),
                Some("provider-a".to_string())
            );
            assert!(state.service_connections.is_connected(&service_id).await);
        }

        #[tokio::test]
        async fn rotating_surface_provider_id_fails_old_provider_in_flight_requests() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            state
                .surface_registry
                .register_service(
                    service_id,
                    "uptrakit-agent-ssh",
                    Some(tenant_id),
                    test_surface_registration("provider-a", tenant_id),
                )
                .expect("provider-a registration should succeed");

            let (_rx, _cancel) = state
                .service_connections
                .register(
                    service_id,
                    BTreeSet::from([Capability::UiSurfaces]),
                    None,
                    None,
                    Some("uptrakit-agent-ssh".to_string()),
                )
                .await;

            let state_for_invoke = Arc::clone(&state);
            let invoke_task = tokio::spawn(async move {
                state_for_invoke
                    .surface_proxy
                    .invoke(
                        &state_for_invoke.service_connections,
                        &state_for_invoke.surface_registry,
                        crate::surface_proxy::SurfaceInvokeRequest {
                            tenant_id,
                            surface_id: "ssh.guest.panel".to_string(),
                            interaction_id: "refresh".to_string(),
                            idempotency_key: "rotate-provider".to_string(),
                            target_provider_id: Some("provider-a".to_string()),
                            caller_origin: crate::surface_proxy::SurfaceCallerOrigin::UserSession {
                                user_id: Uuid::now_v7(),
                                session_id: "session-1".to_string(),
                            },
                            params: serde_json::Map::new(),
                            encrypted_sensitive_params: None,
                        },
                        Some(std::time::Duration::from_secs(30)),
                    )
                    .await
            });

            tokio::task::yield_now().await;

            let processor = MessageProcessor {
                state: Arc::clone(&state),
                service_id,
                cert: None,
                is_system: false,
                has_update_tracking: false,
                has_software_discovery: false,
                has_update_hooks: false,
                has_ui_surfaces: true,
                has_workload_claims: false,
                runtime_instance_id: None,
                service_app_name: Some("uptrakit-agent-ssh".to_string()),
                service_tenant_id: Some(tenant_id),
                linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                report_tracker: ReportTracker::new(),
            };

            let response = processor
                .handle_surface_registration(test_surface_registration("provider-b", tenant_id))
                .await;
            assert!(response.replies.is_empty());
            assert!(matches!(response.action, ProcessorAction::Continue));

            let invoke_result =
                tokio::time::timeout(std::time::Duration::from_secs(1), invoke_task)
                    .await
                    .expect("old-provider invoke should complete promptly after provider rotation")
                    .expect("invoke task should join");
            assert!(matches!(
                invoke_result,
                Err(crate::surface_proxy::SurfaceProxyError::ServiceDisconnected)
            ));
        }
    }
}
