#![expect(
    dead_code,
    reason = "all functions will be called from local_executor.rs when wired"
)]

use uptrakit_plugin_infrastructure_registry::PluginOps;
use uuid::Uuid;

use super::params::{
    optional_string_param, required_string_param, required_uuid_param,
    strict_bool_param_with_default, strict_optional_bool_param,
};

pub(crate) fn notification_channel_type_for_surface_id(surface_id: &str) -> Option<&str> {
    // Notification channel surfaces follow the naming convention "notifications.{channel_type}".
    // Extracting the suffix avoids hardcoding individual plugin-type identifiers here.
    surface_id.strip_prefix("notifications.")
}

fn allowlisted_notification_channel_provider(provider_id: &str, channel_type: &str) -> bool {
    // Canonical notification plugin provider IDs follow two naming conventions:
    // - "plugin.{channel_type}" (legacy short form, e.g. "plugin.email")
    // - "plugin.notifications_{channel_type}" (current long form, e.g. "plugin.notifications_email")
    // Deriving them at runtime from `channel_type` avoids hardcoding individual plugin-type
    // identifiers and handles any future notification plugins automatically.
    provider_id == format!("plugin.{channel_type}")
        || provider_id == format!("plugin.notifications_{channel_type}")
}

pub(crate) fn allowlisted_notification_channel_controller_local_action<'a>(
    provider_id: &str,
    surface_id: &'a str,
    interaction_id: &str,
) -> Option<&'a str> {
    if !matches!(interaction_id, "create" | "edit" | "test" | "delete") {
        return None;
    }
    let channel_type = notification_channel_type_for_surface_id(surface_id)?;
    allowlisted_notification_channel_provider(provider_id, channel_type).then_some(channel_type)
}

pub(crate) async fn execute_allowlisted_notification_channel_action(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    plugin_ops: &dyn PluginOps,
    channel_type: &str,
    interaction_id: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use uptrakit_web_api_types::validation::Validate as _;

    match interaction_id {
        "create" => {
            let req = build_notification_channel_create_request(channel_type, params)?;
            req.validate().map_err(|error| error.to_string())?;
            let response = uptrakit_web_api_queries::queries::notifications::create_channel(
                tenant_db, &req, plugin_ops,
            )
            .await
            .map_err(|error| error.to_string())?;
            serde_json::to_value(response)
                .map_err(|error| format!("failed to serialize create response: {error}"))
        }
        "edit" => {
            let channel_id = required_uuid_param(params, "id")?;
            require_notification_channel_type(tenant_db, channel_id, channel_type).await?;
            let req = build_notification_channel_update_request(channel_type, params)?;
            req.validate().map_err(|error| error.to_string())?;
            let response = uptrakit_web_api_queries::queries::notifications::update_channel(
                tenant_db, channel_id, &req, plugin_ops,
            )
            .await
            .map_err(|error| error.to_string())?;
            let Some(response) = response else {
                return Err("Channel not found".to_string());
            };
            serde_json::to_value(response)
                .map_err(|error| format!("failed to serialize update response: {error}"))
        }
        "delete" => {
            let channel_id = required_uuid_param(params, "id")?;
            require_notification_channel_type(tenant_db, channel_id, channel_type).await?;
            let deleted = uptrakit_web_api_queries::queries::notifications::delete_channel(
                tenant_db, channel_id,
            )
            .await
            .map_err(|error| error.to_string())?;
            if !deleted {
                return Err("Channel not found".to_string());
            }
            Ok(serde_json::json!({}))
        }
        "test" => {
            let channel_id = required_uuid_param(params, "id")?;
            execute_notification_channel_test_action(
                tenant_db,
                plugin_ops,
                channel_id,
                channel_type,
            )
            .await
        }
        _ => Err(format!(
            "action `{interaction_id}` is not allowlisted for notification controller_local execution"
        )),
    }
}

async fn execute_notification_channel_test_action(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    plugin_ops: &dyn PluginOps,
    channel_id: Uuid,
    expected_channel_type: &str,
) -> Result<serde_json::Value, String> {
    let channel =
        require_notification_channel_type(tenant_db, channel_id, expected_channel_type).await?;
    let config_json: serde_json::Value = serde_json::from_str(channel.config.expose_secret())
        .map_err(|error| format!("Failed to parse channel config: {error}"))?;
    let channel_type_id = uptrakit_shared_types::PluginTypeId::new(&channel.channel_type);
    let channel_transport = plugin_ops
        .transport(&channel_type_id)
        .ok_or_else(|| format!("Unsupported channel type: {}", channel.channel_type))?;

    let settings_bag = uptrakit_web_api_queries::notification_settings::build_settings_bag(
        tenant_db.db(),
        tenant_db.tenant_id,
    )
    .await;
    let test_msg = uptrakit_plugin_infrastructure_registry::DeliveryMessage::new(
        "Test Notification",
        "This is a test notification from Uptrakit.",
        None,
        serde_json::json!({"test": true}),
        vec![],
    );

    channel_transport
        .deliver(&config_json, &settings_bag, &test_msg)
        .await
        .map_err(|error| error.to_string())?;

    serde_json::to_value(
        uptrakit_web_api_types::notifications::TestNotificationResponse {
            success: true,
            message: "Test notification delivered successfully".to_string(),
        },
    )
    .map_err(|error| format!("failed to serialize test response: {error}"))
}

async fn require_notification_channel_type(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    channel_id: Uuid,
    expected_channel_type: &str,
) -> Result<uptrakit_shared_db::entity::notification_channel::Model, String> {
    let model = tenant_db
        .find_by_id::<uptrakit_shared_db::entity::notification_channel::Entity, _>(channel_id)
        .one(tenant_db.db())
        .await
        .map_err(|error| format!("failed to load notification channel: {error}"))?;
    let Some(model) = model else {
        return Err("Channel not found".to_string());
    };
    if model.channel_type != expected_channel_type {
        return Err("Channel type mismatch for selected notification surface".to_string());
    }
    Ok(model)
}

pub(crate) fn build_notification_channel_create_request(
    channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::notifications::CreateNotificationChannelRequest, String> {
    validate_or_reject_mismatched_channel_type(channel_type, params)?;

    Ok(
        uptrakit_web_api_types::notifications::CreateNotificationChannelRequest {
            name: required_string_param(params, "name")?,
            channel_type: channel_type.to_string(),
            config: resolve_notification_channel_config(channel_type, params)?,
            enabled: strict_bool_param_with_default(params, "enabled", true)?,
        },
    )
}

pub(crate) fn build_notification_channel_update_request(
    channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::notifications::UpdateNotificationChannelRequest, String> {
    Ok(
        uptrakit_web_api_types::notifications::UpdateNotificationChannelRequest {
            name: optional_string_param(params, "name")?,
            config: Some(resolve_notification_channel_config(channel_type, params)?),
            enabled: strict_optional_bool_param(params, "enabled")?,
        },
    )
}

fn validate_or_reject_mismatched_channel_type(
    expected_channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let Some(channel_type) = params.get("channel_type") else {
        return Ok(());
    };
    let Some(channel_type) = channel_type.as_str() else {
        return Err("field `channel_type` must be a string".to_string());
    };
    if channel_type != expected_channel_type {
        return Err(format!(
            "field `channel_type` must be `{expected_channel_type}` for this surface"
        ));
    }
    Ok(())
}

fn resolve_notification_channel_config(
    channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::notifications::channels::JsonObjectInput, String> {
    if let Some(config) = params.get("config") {
        let Some(config) = config.as_object() else {
            return Err("field `config` must be a JSON object".to_string());
        };
        // `to_addresses` requires special parsing (string or newline-delimited list → Vec).
        // Check by key presence rather than by channel type to stay plugin-type agnostic.
        let value = if config.contains_key("to_addresses") {
            let to_addresses = parse_to_addresses_param(config, "to_addresses")?;
            serde_json::json!({ "to_addresses": to_addresses })
        } else {
            serde_json::Value::Object(config.clone())
        };
        return notification_channel_config_input(value);
    }
    notification_channel_config_input(build_notification_channel_config_from_flat_params(
        channel_type,
        params,
    )?)
}

fn notification_channel_config_input(
    value: serde_json::Value,
) -> Result<uptrakit_web_api_types::notifications::channels::JsonObjectInput, String> {
    uptrakit_web_api_types::notifications::channels::JsonObjectMap::try_from(value)
        .map(Into::into)
        .map_err(|error| error.message)
}

fn build_notification_channel_config_from_flat_params(
    channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    // Channel-record meta keys that belong to the row header, not the config payload.
    const META_KEYS: &[&str] = &["name", "channel_type", "enabled", "id"];

    let mut config = serde_json::Map::new();
    for (key, value) in params {
        if META_KEYS.contains(&key.as_str()) {
            continue;
        }
        if key == "to_addresses" {
            // `to_addresses` may arrive as a newline/comma-delimited string; normalise to Vec.
            let parsed = parse_to_addresses_param(params, "to_addresses")?;
            config.insert(
                key.clone(),
                serde_json::to_value(parsed)
                    .map_err(|e| format!("failed to serialize to_addresses: {e}"))?,
            );
        } else {
            config.insert(key.clone(), value.clone());
        }
    }

    if config.is_empty() {
        return Err(format!(
            "no configuration fields provided for `{channel_type}` channel"
        ));
    }

    Ok(serde_json::Value::Object(config))
}

fn parse_to_addresses_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = params.get(key) else {
        return Err(format!("missing required field `{key}`"));
    };

    match value {
        serde_json::Value::String(text) => {
            let addresses = text
                .split([',', '\n'])
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                return Err(format!("field `{key}` must include at least one address"));
            }
            Ok(addresses)
        }
        serde_json::Value::Array(values) => {
            let mut addresses = Vec::new();
            for value in values {
                let Some(value) = value.as_str() else {
                    return Err(format!("field `{key}` array entries must be strings"));
                };
                let value = value.trim();
                if !value.is_empty() {
                    addresses.push(value.to_string());
                }
            }
            if addresses.is_empty() {
                return Err(format!("field `{key}` must include at least one address"));
            }
            Ok(addresses)
        }
        _ => Err(format!(
            "field `{key}` must be either a string or an array of strings"
        )),
    }
}

use super::SurfaceProxyError;

pub(crate) fn notification_channel_action_type(
    interaction_id: &str,
) -> Option<uptrakit_audit_log::RegisteredAuditAction> {
    match interaction_id {
        "create" => Some(uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE),
        "edit" => Some(uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_UPDATE),
        "delete" => Some(uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_DELETE),
        "test" => Some(uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_TEST),
        _ => None,
    }
}

fn classify_notification_channel_error(
    interaction_id: &str,
    error: &SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    let message = match error {
        SurfaceProxyError::SchemaValidationFailed(message)
        | SurfaceProxyError::SensitiveFieldRejected(message)
        | SurfaceProxyError::PermissionDenied(message) => message.as_str(),
        SurfaceProxyError::Conflict { code, .. } => {
            return (uptrakit_audit_log::AuditOutcome::Failed, code);
        }
        _ => "",
    };

    if message.contains("Channel not found") {
        return if interaction_id == "test" {
            (
                uptrakit_audit_log::AuditOutcome::Failed,
                "channel_not_found",
            )
        } else {
            (
                uptrakit_audit_log::AuditOutcome::Denied,
                "channel_not_found",
            )
        };
    }
    if message.contains("Channel type mismatch") {
        return (
            uptrakit_audit_log::AuditOutcome::Denied,
            "channel_type_mismatch",
        );
    }
    if message.contains("Unsupported channel type") {
        return (
            uptrakit_audit_log::AuditOutcome::Failed,
            "unsupported_channel_type",
        );
    }
    if message.contains("Failed to parse channel config") {
        return (
            uptrakit_audit_log::AuditOutcome::Failed,
            "channel_config_parse_failed",
        );
    }
    if message.contains("field `")
        || message.contains("invalid")
        || message.contains("must be")
        || matches!(error, SurfaceProxyError::SensitiveFieldRejected(_))
    {
        return (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_request",
        );
    }
    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

pub(crate) fn emit_notification_channel_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    interaction_id: &str,
    channel_type: &str,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else {
        return;
    };
    let Some(caller_user_id) = caller_user_id else {
        return;
    };
    let Some(action_type) = notification_channel_action_type(interaction_id) else {
        return;
    };

    let requested_id = request_params
        .get("id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_name = request_params
        .get("name")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code, target_id, target_display) = match result {
        Ok(response) => {
            let target_id = response
                .get("id")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
                .or_else(|| requested_id.clone())
                .or_else(|| (interaction_id == "create").then(|| "pending".to_string()));
            let target_display = response
                .get("name")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
                .or_else(|| requested_name.clone());
            (
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                target_id,
                target_display,
            )
        }
        Err(error) => {
            let (outcome, reason_code) = classify_notification_channel_error(interaction_id, error);
            let target_id = requested_id
                .clone()
                .or_else(|| (interaction_id == "create").then(|| "pending".to_string()));
            (
                outcome,
                Some(reason_code),
                target_id,
                requested_name.clone(),
            )
        }
    };

    let mut details = serde_json::json!({
        "channel_type": channel_type,
        "create_source": format!("surface_proxy.notification_channel.{interaction_id}"),
    });
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(tenant_id)
        .actor(
            uptrakit_audit_log::AuditActorType::User,
            Some(caller_user_id),
        )
        .target_opt(
            Some("notification_channel".to_string()),
            target_id,
            target_display,
        )
        .outcome(outcome)
        .details(details)
        .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}
