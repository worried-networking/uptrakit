// All functions in this module are called exclusively from `local_executor.rs`, which is not
// yet wired into the module tree. `local_executor.rs` shares function names with inline
// helpers in `surface_proxy.rs` (the legacy path) that must be deduplicated first. Until
// that refactor lands, every item here lacks a compiled caller, triggering dead_code. Remove
// this allow once `local_executor.rs` is incorporated.
#![allow(dead_code)]

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
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, SurfaceProxyError> {
    use uptrakit_web_api_types::validation::Validate as _;

    let request = build_proxmox_add_config_create_request(params)
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
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    result: &serde_json::Value,
) {
    let Some(caller_user_id) = caller_user_id else {
        return;
    };
    let Some(plugin_config_id) = result.get("id").and_then(|value| value.as_str()) else {
        return;
    };
    let Some(config_name) = result.get("name").and_then(|value| value.as_str()) else {
        return;
    };
    tracing::warn!(
        target: "security_audit",
        user_id = %caller_user_id,
        tenant_id = %tenant_id,
        plugin_config_id = %plugin_config_id,
        plugin_type = "infrastructure_proxmox",
        config_name = %config_name,
        "plugin config created"
    );
}

pub(crate) fn build_proxmox_add_config_create_request(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::plugin_configs::CreatePluginConfigRequest, String> {
    Ok(
        uptrakit_web_api_types::plugin_configs::CreatePluginConfigRequest {
            name: required_string_param(params, "name")?,
            plugin_type: uptrakit_shared_types::plugin_ids::INFRASTRUCTURE_PROXMOX.clone(),
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

#[cfg(test)]
mod tests {
    use super::{
        allowlisted_proxmox_add_config_controller_local_action, allowlisted_proxmox_provider,
        build_proxmox_add_config_create_request,
    };

    #[test]
    fn allowlist_accepts_only_expected_provider_surface_and_action() {
        assert!(allowlisted_proxmox_provider(
            "plugin.infrastructure_proxmox"
        ));
        assert!(allowlisted_proxmox_provider("infrastructure_proxmox"));
        assert!(!allowlisted_proxmox_provider("plugin.releases_docker"));

        assert!(allowlisted_proxmox_add_config_controller_local_action(
            "proxmox.hosts",
            "add-config"
        ));
        assert!(!allowlisted_proxmox_add_config_controller_local_action(
            "proxmox.hosts",
            "save-config"
        ));
    }

    #[test]
    fn build_request_normalizes_flat_params_and_defaults_verify_tls() {
        let params = serde_json::json!({
            "name": "PVE Cluster",
            "api_url": "https://pve.local:8006",
            "api_token": "root@pam!uptrakit=secret-token",
            "node_filter": "node-a, , node-b"
        });
        let params = params.as_object().expect("params should be an object");

        let request = build_proxmox_add_config_create_request(params)
            .expect("request should build from flat params");
        assert_eq!(request.name, "PVE Cluster");
        assert_eq!(request.plugin_type.as_str(), "infrastructure_proxmox");
        assert_eq!(
            request.config,
            serde_json::json!({
                "api_url": "https://pve.local:8006",
                "api_token": "root@pam!uptrakit=secret-token",
                "verify_tls": true,
                "node_filter": ["node-a", "node-b"],
            })
        );
        assert!(request.enabled);
    }

    #[test]
    fn build_request_accepts_nested_config_and_legacy_verify_tls_string() {
        let params = serde_json::json!({
            "name": "PVE Cluster",
            "config": {
                "api_url": "https://pve.local:8006",
                "api_token": "root@pam!uptrakit=secret-token",
                "verify_tls": "false",
                "node_filter": ["node-a", "node-b"]
            }
        });
        let params = params.as_object().expect("params should be an object");

        let request = build_proxmox_add_config_create_request(params)
            .expect("request should build from nested config");
        assert_eq!(
            request.config,
            serde_json::json!({
                "api_url": "https://pve.local:8006",
                "api_token": "root@pam!uptrakit=secret-token",
                "verify_tls": false,
                "node_filter": ["node-a", "node-b"],
            })
        );
    }

    #[test]
    fn build_request_rejects_invalid_verify_tls_value() {
        let params = serde_json::json!({
            "name": "PVE Cluster",
            "config": {
                "api_url": "https://pve.local:8006",
                "api_token": "root@pam!uptrakit=secret-token",
                "verify_tls": "not-bool"
            }
        });
        let params = params.as_object().expect("params should be an object");

        let err = build_proxmox_add_config_create_request(params)
            .expect_err("invalid verify_tls should fail");
        assert!(
            err.contains("verify_tls"),
            "expected verify_tls error, got: {err}"
        );
    }
}
