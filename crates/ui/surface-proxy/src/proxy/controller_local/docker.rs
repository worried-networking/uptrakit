#![expect(
    dead_code,
    reason = "all functions will be called from local_executor.rs when wired"
)]

use uuid::Uuid;

use super::SurfaceProxyError;

const PLUGIN_TYPE_RELEASES_DOCKER: &str = "releases_docker";

pub(crate) fn allowlisted_docker_switch_tag_controller_local_action(
    provider_id: &str,
    surface_id: &str,
    interaction_id: &str,
) -> bool {
    matches!(provider_id, "plugin.releases_docker" | "releases_docker")
        && surface_id == "docker.item-host-actions"
        && interaction_id == "switch-tag"
}

fn classify_docker_switch_tag_error(
    error: &SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    if matches!(error, SurfaceProxyError::PermissionDenied(_)) {
        return (
            uptrakit_audit_log::AuditOutcome::Denied,
            "permission_denied",
        );
    }
    let message = match error {
        SurfaceProxyError::SchemaValidationFailed(message)
        | SurfaceProxyError::SensitiveFieldRejected(message) => message.as_str(),
        SurfaceProxyError::Conflict { code, .. } => {
            return (uptrakit_audit_log::AuditOutcome::Failed, code);
        }
        _ => "",
    };

    if message.contains("missing required parameter")
        || message.contains("invalid UUID")
        || message.contains("invalid image reference")
    {
        return (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_request",
        );
    }
    if message.contains("no plugin assignments found for this host")
        || message.contains("host_software_item not found for host")
    {
        return (
            uptrakit_audit_log::AuditOutcome::Denied,
            "host_assignment_not_found",
        );
    }
    if message.contains("database error")
        || message.contains("failed to begin transaction")
        || message.contains("failed to update plugin row")
        || message.contains("failed to update host_software_item")
        || message.contains("failed to commit transaction")
    {
        return (uptrakit_audit_log::AuditOutcome::Failed, "storage_error");
    }
    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

pub(crate) fn emit_docker_switch_tag_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else {
        return;
    };
    let Some(caller_user_id) = caller_user_id else {
        return;
    };

    let requested_software_item_id = request_params
        .get("software_item_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_host_id = request_params
        .get("host_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_new_image_ref = request_params
        .get("new_image_ref")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code) = match result {
        Ok(_) => (uptrakit_audit_log::AuditOutcome::Success, None),
        Err(error) => {
            let (outcome, reason_code) = classify_docker_switch_tag_error(error);
            (outcome, Some(reason_code))
        }
    };

    let mut details = serde_json::json!({
        "plugin_type": PLUGIN_TYPE_RELEASES_DOCKER,
        "mutation_source": "surface_proxy.docker_switch_tag",
    });
    if let Some(host_id) = requested_host_id.as_deref() {
        details["host_id"] = serde_json::json!(host_id);
    }
    if let Some(new_image_ref) = requested_new_image_ref.as_deref() {
        details["new_image_ref"] = serde_json::json!(new_image_ref);
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
    )
    .tenant_scope(tenant_id)
    .actor(
        uptrakit_audit_log::AuditActorType::User,
        Some(caller_user_id),
    )
    .target_opt(
        Some("software_item".to_string()),
        requested_software_item_id,
        None,
    )
    .outcome(outcome)
    .details(details)
    .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}
