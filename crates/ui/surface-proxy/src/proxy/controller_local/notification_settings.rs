use uuid::Uuid;

use super::SurfaceProxyError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotificationSettingsAction {
    ConfigureSmtp,
    SaveGlobalSmtp,
    SaveGlobalTelegram,
}

pub(crate) fn allowlisted_notification_settings_controller_local_action(
    provider_id: &str,
    surface_id: &str,
    interaction_id: &str,
) -> Option<NotificationSettingsAction> {
    let channel_type = surface_id
        .strip_prefix("notifications.")
        .and_then(|rest| rest.split('.').next())?;
    let short_form = format!("plugin.{channel_type}");
    let long_form = format!("plugin.notifications_{channel_type}");
    if provider_id != short_form && provider_id != long_form {
        return None;
    }
    match (surface_id, interaction_id) {
        ("notifications.email", "configure_smtp") => {
            Some(NotificationSettingsAction::ConfigureSmtp)
        }
        ("notifications.email.global_smtp", "save_global_smtp") => {
            Some(NotificationSettingsAction::SaveGlobalSmtp)
        }
        ("notifications.telegram.global_settings", "save_global_telegram") => {
            Some(NotificationSettingsAction::SaveGlobalTelegram)
        }
        _ => None,
    }
}

fn notification_settings_audit_action_type(
    action: NotificationSettingsAction,
) -> uptrakit_audit_log::RegisteredAuditAction {
    match action {
        NotificationSettingsAction::ConfigureSmtp => {
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE
        }
        NotificationSettingsAction::SaveGlobalSmtp
        | NotificationSettingsAction::SaveGlobalTelegram => {
            uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE
        }
    }
}

fn notification_settings_target(
    action: NotificationSettingsAction,
) -> (&'static str, &'static str) {
    match action {
        NotificationSettingsAction::ConfigureSmtp => ("tenant_setting", "smtp"),
        NotificationSettingsAction::SaveGlobalSmtp => ("global_setting", "global_smtp"),
        NotificationSettingsAction::SaveGlobalTelegram => ("global_setting", "global_telegram"),
    }
}

fn notification_settings_scope(action: NotificationSettingsAction) -> &'static str {
    match action {
        NotificationSettingsAction::ConfigureSmtp => "tenant",
        NotificationSettingsAction::SaveGlobalSmtp
        | NotificationSettingsAction::SaveGlobalTelegram => "global",
    }
}

fn notification_settings_mutation_source(action: NotificationSettingsAction) -> &'static str {
    match action {
        NotificationSettingsAction::ConfigureSmtp => {
            "surface_proxy.notification_settings.configure_smtp"
        }
        NotificationSettingsAction::SaveGlobalSmtp => {
            "surface_proxy.notification_settings.save_global_smtp"
        }
        NotificationSettingsAction::SaveGlobalTelegram => {
            "surface_proxy.notification_settings.save_global_telegram"
        }
    }
}

fn classify_notification_settings_error(
    error: &SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match error {
        SurfaceProxyError::SensitiveFieldRejected(_) => {
            return (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "invalid_request",
            );
        }
        SurfaceProxyError::PermissionDenied(_) => {
            return (
                uptrakit_audit_log::AuditOutcome::Denied,
                "permission_denied",
            );
        }
        SurfaceProxyError::Conflict { code, .. } => {
            return (uptrakit_audit_log::AuditOutcome::Failed, code);
        }
        SurfaceProxyError::SchemaValidationFailed(message) => {
            let lowered = message.to_ascii_lowercase();
            if lowered.contains("required")
                || lowered.contains("invalid")
                || lowered.contains("must be")
                || lowered.contains("unknown action")
            {
                return (
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    "invalid_request",
                );
            }
            if lowered.contains("forbidden")
                || lowered.contains("not authorized")
                || lowered.contains("permission")
            {
                return (
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "permission_denied",
                );
            }
            if lowered.contains("internal server error")
                || lowered.contains("failed to")
                || lowered.contains("database")
            {
                return (uptrakit_audit_log::AuditOutcome::Failed, "storage_error");
            }
        }
        _ => {}
    }
    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

pub(crate) fn emit_notification_settings_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    action: NotificationSettingsAction,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else {
        return;
    };
    let Some(caller_user_id) = caller_user_id else {
        return;
    };

    let (outcome, reason_code) = match result {
        Ok(_) => (uptrakit_audit_log::AuditOutcome::Success, None),
        Err(error) => {
            let (outcome, reason_code) = classify_notification_settings_error(error);
            (outcome, Some(reason_code))
        }
    };

    let mut requested_keys = request_params.keys().cloned().collect::<Vec<_>>();
    requested_keys.sort();

    let (target_type, target_id) = notification_settings_target(action);
    let mut details = serde_json::json!({
        "setting_area": target_id,
        "setting_scope": notification_settings_scope(action),
        "mutation_source": notification_settings_mutation_source(action),
        "requested_keys": requested_keys,
    });
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }
    let builder =
        uptrakit_audit_log::AuditEntry::builder(notification_settings_audit_action_type(action))
            .tenant_scope(tenant_id)
            .actor(
                uptrakit_audit_log::AuditActorType::User,
                Some(caller_user_id),
            )
            .target(
                target_type,
                target_id.to_string(),
                Some(target_id.to_string()),
            );

    if let Ok(entry) = builder.outcome(outcome).details(details).build() {
        audit_emitter.emit_best_effort(entry);
    }
}
