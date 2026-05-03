use uptrakit_plugin_infrastructure_registry::PluginOps;
use uuid::Uuid;

use super::SurfaceProxyError;
use super::params::{parse_csv_array_or_string_array_param, required_string_param};

pub(crate) fn allowlisted_proxmox_provider(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "plugin.infrastructure_proxmox" | "infrastructure_proxmox"
    )
}

pub(crate) fn allowlisted_proxmox_add_config_controller_local_action(
    surface_id: &str,
    interaction_id: &str,
) -> bool {
    surface_id == "proxmox.hosts" && interaction_id == "add-config"
}

pub(crate) async fn execute_allowlisted_proxmox_add_config_action(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    plugin_ops: &dyn PluginOps,
    plugin_type: uptrakit_shared_types::PluginTypeId,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, SurfaceProxyError> {
    use uptrakit_web_api_types::validation::Validate as _;

    let request = build_proxmox_add_config_create_request(plugin_type, params)
        .map_err(SurfaceProxyError::SchemaValidationFailed)?;
    request
        .validate()
        .map_err(|error| SurfaceProxyError::SchemaValidationFailed(error.to_string()))?;
    plugin_ops
        .validate_config(&request.plugin_type, &request.config)
        .map_err(|error| SurfaceProxyError::SchemaValidationFailed(error.to_string()))?;
    let response = uptrakit_web_api_queries::queries::plugin_configs::create_plugin_config(
        plugin_ops, tenant_db, request,
    )
    .await
    .map_err(|error| match error.current_context() {
        uptrakit_web_api_queries::queries::plugin_configs::PluginConfigError::DuplicateName => {
            SurfaceProxyError::Conflict {
                message: error.to_string(),
                code: "duplicate_name",
            }
        }
        _ => SurfaceProxyError::SchemaValidationFailed(error.to_string()),
    })?;
    serde_json::to_value(response).map_err(|error| {
        SurfaceProxyError::SchemaValidationFailed(format!(
            "failed to serialize proxmox add-config response: {error}"
        ))
    })
}

pub(crate) fn emit_proxmox_add_config_audit_event(
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
    let requested_name = request_params
        .get("name")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code, error_kind, target_id, target_display, plugin_type) = match result {
        Ok(result) => {
            let Some(plugin_config_id) = result.get("id").and_then(|v| v.as_str()) else {
                return;
            };
            let config_name = result
                .get("name")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
                .or_else(|| requested_name.clone());
            let plugin_type = result
                .get("plugin_type")
                .and_then(|v| v.as_str())
                .unwrap_or("infrastructure_proxmox");
            (
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                None,
                Some(plugin_config_id.to_string()),
                config_name,
                plugin_type.to_string(),
            )
        }
        Err(error) => {
            let (outcome, reason_code, error_kind) = classify_proxmox_add_config_error(error);
            (
                outcome,
                Some(reason_code),
                error_kind,
                None,
                requested_name,
                "infrastructure_proxmox".to_string(),
            )
        }
    };

    let mut details = serde_json::json!({
        "plugin_type": plugin_type,
        "create_source": "surface_proxy.proxmox_add_config",
    });
    if let Some(config_name) = target_display.as_deref() {
        details["config_name"] = serde_json::json!(config_name);
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }
    if let Some(error_kind) = error_kind {
        details["error_kind"] = serde_json::json!(error_kind);
    }

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .tenant_scope(tenant_id)
    .actor(
        uptrakit_audit_log::AuditActorType::User,
        Some(caller_user_id),
    )
    .target_opt(Some("plugin_config".to_string()), target_id, target_display)
    .outcome(outcome)
    .details(details)
    .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}

fn classify_proxmox_add_config_error(
    error: &SurfaceProxyError,
) -> (
    uptrakit_audit_log::AuditOutcome,
    &'static str,
    Option<&'static str>,
) {
    match error {
        SurfaceProxyError::SchemaValidationFailed(_)
        | SurfaceProxyError::SensitiveFieldRejected(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "validation_failed",
            None,
        ),
        SurfaceProxyError::Conflict { code, .. } => (
            uptrakit_audit_log::AuditOutcome::Failed,
            code,
            Some("conflict"),
        ),
        _ => (uptrakit_audit_log::AuditOutcome::Failed, "failed", None),
    }
}

pub(crate) fn build_proxmox_add_config_create_request(
    plugin_type: uptrakit_shared_types::PluginTypeId,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::plugin_configs::CreatePluginConfigRequest, String> {
    Ok(
        uptrakit_web_api_types::plugin_configs::CreatePluginConfigRequest {
            name: required_string_param(params, "name")?,
            plugin_type,
            config: resolve_proxmox_add_config(params)?,
            enabled: true,
        },
    )
}

fn resolve_proxmox_add_config(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if let Some(config) = params.get("config") {
        let Some(config) = config.as_object() else {
            return Err("field `config` must be a JSON object".to_string());
        };
        return build_proxmox_config_from_params(config);
    }
    build_proxmox_config_from_params(params)
}

fn build_proxmox_config_from_params(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "api_url": required_string_param(params, "api_url")?,
        "api_token": required_string_param(params, "api_token")?,
        "verify_tls": proxmox_verify_tls_param_with_default(params, "verify_tls", true)?,
        "node_filter": parse_csv_array_or_string_array_param(params, "node_filter")?,
    }))
}

fn proxmox_verify_tls_param_with_default(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    let Some(value) = params.get(key) else {
        return Ok(default);
    };
    match value {
        serde_json::Value::Bool(value) => Ok(*value),
        serde_json::Value::String(value) => match value.trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!(
                "field `{key}` must be a boolean or the string `true`/`false`"
            )),
        },
        _ => Err(format!(
            "field `{key}` must be a boolean or the string `true`/`false`"
        )),
    }
}
