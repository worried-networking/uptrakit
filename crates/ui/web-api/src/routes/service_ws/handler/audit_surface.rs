//! Surface-interaction audit helpers.
//!
//! Classification, formatting and emit fns for the surface registration
//! and surface action message flows.

#![expect(
    clippy::string_slice,
    reason = "slice index is at a validated char boundary"
)]

use crate::AppState;
use uptrakit_wire::surfaces;

use super::surface_wire;

pub(super) fn surface_action_target_display(
    surface_id: &surfaces::SurfaceId,
    interaction_id: &surfaces::InteractionId,
) -> String {
    format!("{surface_id}/{interaction_id}")
}

pub(super) fn surface_provider_kind_name(provider_kind: surfaces::ProviderKind) -> &'static str {
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

pub(super) fn truncate_surface_registration_audit_value(value: &str) -> String {
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

pub(super) fn classify_surface_registration_validation_error(
    error: &uptrakit_wire::limits::WireValidationError,
) -> &'static str {
    match error.field {
        "effective_tenant_binding.tenant_id" => "invalid_tenant_binding",
        "provider.provider_id" => "invalid_provider_id",
        _ => "invalid_request",
    }
}

pub(super) fn surface_registration_rejection_reason_code(
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
        _ => {
            tracing::warn!(?code, "unhandled SurfaceProviderRejectionCode variant");
            "unknown_rejection_code"
        }
    }
}

pub(super) fn classify_surface_registration_error_for_audit(
    error: &crate::surface_registry::SurfaceRegistryError,
) -> &'static str {
    match error {
        crate::surface_registry::SurfaceRegistryError::ProviderRejected(rejection) => rejection
            .reasons
            .first()
            .map(|reason| surface_registration_rejection_reason_code(&reason.code))
            .unwrap_or("provider_rejected"),
        crate::surface_registry::SurfaceRegistryError::ProviderConflict(_) => "provider_conflict",
        _ => {
            tracing::warn!(
                ?error,
                "unhandled SurfaceRegistryError variant in audit classification"
            );
            "registration_error"
        }
    }
}

pub(super) struct ServiceAuditCtx<'a> {
    pub(super) state: &'a AppState,
    pub(super) service_id: uuid::Uuid,
    pub(super) service_app_name: Option<&'a str>,
}

pub(super) fn emit_surface_registration_audit_event(
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

    let builder = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
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
        Ok(entry) => ctx.state.audit_emitter.emit_event(entry),
        Err(error) => tracing::warn!(
            service_id = %ctx.service_id,
            provider_id = %payload.provider.provider_id,
            outcome = %outcome,
            error = %error,
            "failed to build surface registration audit entry"
        ),
    }
}

pub(super) async fn emit_surface_action_scope_denied_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
    service_app_name: Option<&str>,
    service_tenant_id: uuid::Uuid,
    payload: &uptrakit_wire::surfaces::SurfaceActionRequest,
) {
    let entry = match uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
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

    state.audit_emitter.emit_event(entry);
}

pub(super) fn emit_surface_action_invoke_audit_event(
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

    let entry = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
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
        Ok(entry) => ctx.state.audit_emitter.emit_event(entry),
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

pub(super) fn classify_surface_action_response_for_audit(
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

    (outcome, Some(surface_wire::action_error_code(&error.code)))
}

pub(super) fn classify_surface_proxy_error_for_audit(
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
        _ => {
            tracing::warn!(
                ?error,
                "unhandled SurfaceProxyError variant in audit classification"
            );
            (uptrakit_audit_log::AuditOutcome::Failed, "proxy_error")
        }
    }
}

pub(super) fn classify_surface_lookup_error_for_audit(
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
        _ => {
            tracing::warn!(
                ?error,
                "unhandled SurfaceRegistryLookupError variant in audit classification"
            );
            (
                uptrakit_audit_log::AuditOutcome::Failed,
                "surface_lookup_error",
            )
        }
    }
}

pub(super) fn classify_surface_action_request_validation_error(
    error: &uptrakit_wire::limits::WireValidationError,
) -> &'static str {
    if error.field == "tenant_id" {
        "invalid_tenant_id"
    } else {
        "invalid_request"
    }
}

pub(super) fn resolve_surface_action_audit_tenant_id(
    service_tenant_id: Option<uuid::Uuid>,
    payload: &uptrakit_wire::surfaces::SurfaceActionRequest,
) -> Option<uuid::Uuid> {
    service_tenant_id.or_else(|| uuid::Uuid::parse_str(&payload.tenant_id).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_wire::surfaces;

    #[test]
    fn surface_action_target_display_includes_surface_and_interaction() {
        let surface_id = surfaces::SurfaceId::new("notifications.email").unwrap();
        let interaction_id = surfaces::InteractionId::new("configure_smtp").unwrap();

        assert_eq!(
            surface_action_target_display(&surface_id, &interaction_id),
            "notifications.email/configure_smtp"
        );
    }
}
