#![expect(
    dead_code,
    reason = "all functions will be called from local_executor.rs when wired"
)]

use uuid::Uuid;

use super::SurfaceProxyError;

const PLUGIN_TYPE_INFRASTRUCTURE_PROXMOX: &str = "infrastructure_proxmox";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProxmoxUpdateProtectionAction {
    SaveGlobalDefaults,
    SaveItemOverrides,
}

pub(crate) fn allowlisted_proxmox_update_protection_controller_local_action(
    surface_id: &str,
    interaction_id: &str,
) -> Option<ProxmoxUpdateProtectionAction> {
    match (surface_id, interaction_id) {
        ("proxmox.settings.update-protection", "save-global-defaults") => {
            Some(ProxmoxUpdateProtectionAction::SaveGlobalDefaults)
        }
        ("proxmox.software-item.update-protection", "save-item-overrides") => {
            Some(ProxmoxUpdateProtectionAction::SaveItemOverrides)
        }
        _ => None,
    }
}

fn proxmox_update_protection_action_type(
    action: ProxmoxUpdateProtectionAction,
) -> uptrakit_audit_log::RegisteredAuditAction {
    match action {
        ProxmoxUpdateProtectionAction::SaveGlobalDefaults => {
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE
        }
        ProxmoxUpdateProtectionAction::SaveItemOverrides => {
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE
        }
    }
}

fn proxmox_update_protection_mutation_source(
    action: ProxmoxUpdateProtectionAction,
) -> &'static str {
    match action {
        ProxmoxUpdateProtectionAction::SaveGlobalDefaults => {
            "surface_proxy.proxmox_update_protection.save_global_defaults"
        }
        ProxmoxUpdateProtectionAction::SaveItemOverrides => {
            "surface_proxy.proxmox_update_protection.save_item_overrides"
        }
    }
}

fn classify_proxmox_update_protection_error(
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
        || message.contains("invalid protection mode")
        || message.contains("invalid backup target selection")
        || message.contains("missing target key")
        || message.contains("belongs to a different Proxmox configuration")
    {
        return (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_request",
        );
    }
    if message.contains("not found in tenant scope")
        || message.contains("not assigned to software item")
        || message.contains("not present in cache")
    {
        return (
            uptrakit_audit_log::AuditOutcome::Denied,
            "resource_not_available",
        );
    }
    if message.contains("failed to save")
        || message.contains("failed to clear")
        || message.contains("database error")
    {
        return (uptrakit_audit_log::AuditOutcome::Failed, "storage_error");
    }
    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

pub(crate) fn emit_proxmox_update_protection_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    action: ProxmoxUpdateProtectionAction,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else {
        return;
    };
    let Some(caller_user_id) = caller_user_id else {
        return;
    };

    let requested_plugin_config_id = request_params
        .get("plugin_config_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_software_item_id = request_params
        .get("software_item_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_mode = request_params
        .get("mode")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code, target_type, target_id, details_target_plugin_config_id) =
        match (action, result) {
            (ProxmoxUpdateProtectionAction::SaveGlobalDefaults, Ok(response)) => (
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                Some("plugin_config".to_string()),
                response
                    .get("plugin_config_id")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .or_else(|| requested_plugin_config_id.clone()),
                response
                    .get("plugin_config_id")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .or_else(|| requested_plugin_config_id.clone()),
            ),
            (ProxmoxUpdateProtectionAction::SaveGlobalDefaults, Err(error)) => {
                let (outcome, reason_code) = classify_proxmox_update_protection_error(error);
                (
                    outcome,
                    Some(reason_code),
                    Some("plugin_config".to_string()),
                    requested_plugin_config_id.clone(),
                    requested_plugin_config_id.clone(),
                )
            }
            (ProxmoxUpdateProtectionAction::SaveItemOverrides, Ok(response)) => (
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                Some("software_item".to_string()),
                response
                    .get("software_item_id")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .or_else(|| requested_software_item_id.clone()),
                response
                    .get("plugin_config_id")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .or_else(|| requested_plugin_config_id.clone()),
            ),
            (ProxmoxUpdateProtectionAction::SaveItemOverrides, Err(error)) => {
                let (outcome, reason_code) = classify_proxmox_update_protection_error(error);
                (
                    outcome,
                    Some(reason_code),
                    Some("software_item".to_string()),
                    requested_software_item_id.clone(),
                    requested_plugin_config_id.clone(),
                )
            }
        };

    let mut details = serde_json::json!({
        "plugin_type": PLUGIN_TYPE_INFRASTRUCTURE_PROXMOX,
        "mutation_source": proxmox_update_protection_mutation_source(action),
    });
    if let Some(mode) = requested_mode.as_deref() {
        details["mode"] = serde_json::json!(mode);
    }
    if let Some(plugin_config_id) = details_target_plugin_config_id.as_deref() {
        details["plugin_config_id"] = serde_json::json!(plugin_config_id);
    }
    if let Ok(response) = result
        && let Some(cleared) = response.get("cleared").and_then(|v| v.as_bool())
    {
        details["cleared"] = serde_json::json!(cleared);
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }

    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::builder(proxmox_update_protection_action_type(action))
            .tenant_scope(tenant_id)
            .actor(
                uptrakit_audit_log::AuditActorType::User,
                Some(caller_user_id),
            )
            .target_opt(target_type, target_id, None)
            .outcome(outcome)
            .details(details)
            .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}
