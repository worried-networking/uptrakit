use crate::AppState;
use crate::config_test_proxy::ConfigTestProxyError;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanTestPluginConfigs;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, RelationTrait};
use std::sync::Arc;
use uptrakit_shared_db::entity::{host, plugin_config, prelude::*, service, service_host};
use uptrakit_shared_types::{PluginCapability, PluginTypeId};
use uptrakit_web_api_types::plugin_config_test::{
    TestPluginConfigRequest, TestPluginConfigResponse,
};
use uuid::Uuid;

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::plugin_configs::{
    CreatePluginConfigRequest, PluginConfigResponse, PluginTypeInfo, UpdatePluginConfigRequest,
};

pub use batch::{__path_batch_plugin_configs, batch_plugin_configs};
pub use crud::ListPluginConfigsParams;
pub(crate) use crud::plugin_field_to_api_field;
pub use crud::{
    __path_create_plugin_config, __path_delete_plugin_config, __path_get_plugin_config,
    __path_list_plugin_configs, __path_list_plugin_types, __path_update_plugin_config,
    create_plugin_config, delete_plugin_config, get_plugin_config, list_plugin_configs,
    list_plugin_types, update_plugin_config,
};
pub use discover::{__path_discover_plugin_config, discover_plugin_config};

async fn load_active_agent_service_for_host(
    tenant_db: &TenantDb,
    host_id: Uuid,
) -> Result<service::Model, Response> {
    let links = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.eq(host_id))
        .all(tenant_db.db())
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to query service-host links: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let service_ids: Vec<Uuid> = links.into_iter().map(|link| link.service_id).collect();
    if service_ids.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "No agent connected to this host",
        ));
    }

    let agents = match Service::find()
        .filter(service::Column::Id.is_in(service_ids))
        .filter(service::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(agents) => agents,
        Err(e) => {
            tracing::error!("Failed to load services for host: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let agent = agents
        .iter()
        .filter(|svc| svc.status == service::ServiceStatus::Approved)
        .max_by_key(|svc| svc.last_seen_at.unwrap_or(svc.updated_at))
        .cloned()
        .or_else(|| {
            agents
                .iter()
                .max_by_key(|svc| svc.last_seen_at.unwrap_or(svc.updated_at))
                .cloned()
        });

    match agent {
        Some(a) if a.status != service::ServiceStatus::Approved => Err(error_response(
            StatusCode::BAD_REQUEST,
            "Agent is not approved",
        )),
        Some(a) => Ok(a),
        None => Err(error_response(
            StatusCode::BAD_REQUEST,
            "No agent connected to this host",
        )),
    }
}

/// Test a plugin configuration without saving it.
///
/// Validates the plugin type, merges with an optional saved config, checks
/// for dangerous command patterns, then routes to the appropriate test path:
///
/// - **Controller-side** (plugins with `ControllerSideFetchReleases`):
///   validates config structure and returns success immediately.
/// - **Agent-side** (all others): requires `host_id`, resolves the host to a
///   connected service, sends a `TestPluginConfig` wire message, and waits for
///   the result (30 s timeout).
#[utoipa::path(
    post,
    path = "/api/v1/plugin-configs/test",
    extensions(("x-required-permission" = json!("test_plugin_configs"))),
    request_body = TestPluginConfigRequest,
    responses(
        (status = 200, description = "Test result", body = TestPluginConfigResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Host or plugin config not found"),
        (status = 502, description = "Agent did not respond"),
    ),
    tag = "Plugin Configs",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn test_plugin_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTestPluginConfigs(_user): CanTestPluginConfigs,
    Validated(body): Validated<TestPluginConfigRequest>,
) -> Response {
    // 1. Validate plugin type is known and supports per-instance plugin configs.
    let plugin_type_id = PluginTypeId::new(&body.plugin_type);
    if let Some(rejection) = crud::reject_config_model_none_plugin_type(&state, &plugin_type_id) {
        return rejection;
    }
    let caps = state.plugin.plugin_ops.capabilities(&plugin_type_id);

    // 2. Merge with saved config if plugin_config_id is provided.
    let config = if let Some(config_id) = body.plugin_config_id {
        let saved = match PluginConfig::find_by_id(config_id)
            .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id()))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .one(tenant_db.db())
            .await
        {
            Ok(Some(c)) => c,
            Ok(None) => {
                return error_response(StatusCode::NOT_FOUND, "Plugin config not found");
            }
            Err(e) => {
                tracing::error!("DB error loading plugin config: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
        // Shallow-merge incoming config on top of saved config.
        let mut merged = saved.config.clone();
        if let (Some(base), Some(overlay)) = (merged.as_object_mut(), body.config.as_object()) {
            for (k, v) in overlay {
                base.insert(k.clone(), v.clone());
            }
        }
        merged
    } else {
        body.config.clone()
    };

    // 3. Validate merged config.
    if let Err(e) = state
        .plugin
        .plugin_ops
        .validate_config(&plugin_type_id, &config)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Invalid plugin config: {e}"),
        );
    }

    // 4. Reject dangerous commands if enabled.
    if state.reject_dangerous_commands {
        let matches = command_safety::collect_dangerous_patterns(&config);
        if !matches.is_empty() {
            return error_response(
                StatusCode::BAD_REQUEST,
                command_safety::format_dangerous_pattern_rejection(&matches),
            );
        }
    }

    // 5. Determine test kind from capabilities.
    let is_controller_side = caps.contains(&PluginCapability::ControllerSideFetchReleases);

    if is_controller_side {
        // Controller-side test: config validation is sufficient. The plugin
        // fetches releases from external APIs on the controller, so a
        // successful config validation means the config is structurally valid.
        let mut resp = TestPluginConfigResponse::new(true, "connectivity".to_string(), 0);
        resp.output = Some("Plugin configuration is valid".to_string());
        return (StatusCode::OK, Json(resp)).into_response();
    }

    // Agent-side test: host_id is required.
    let host_id = match body.host_id {
        Some(id) => id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "host_id is required for agent-side plugin tests",
            );
        }
    };

    // 6. Resolve host → service.
    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let service_id = match load_active_agent_service_for_host(&tenant_db, host_id).await {
        Ok(service) => service.id,
        Err(resp) => return resp,
    };

    // 7. Determine test kind.
    let test_kind_str = body.test_kind.as_deref().unwrap_or("version_detection");
    let test_kind = match test_kind_str {
        "version_detection" => uptrakit_wire::ConfigTestKind::VersionDetection,
        "update_command_validation" => uptrakit_wire::ConfigTestKind::UpdateCommandValidation,
        "pre_update_hook" => uptrakit_wire::ConfigTestKind::PreUpdateHook,
        "post_update_hook" => uptrakit_wire::ConfigTestKind::PostUpdateHook,
        "connectivity" => uptrakit_wire::ConfigTestKind::Connectivity,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unknown test_kind: {test_kind_str}"),
            );
        }
    };

    // 8. Build payload and invoke via proxy.
    let request_id = Uuid::now_v7().to_string();
    let mut payload = uptrakit_wire::TestPluginConfigPayload::new(
        request_id,
        host_record.machine_id.clone(),
        test_kind,
        body.plugin_type.clone(),
        config,
    );
    payload.package_identifier = body.package_identifier.clone();

    let timeout = std::time::Duration::from_secs(30);
    match state
        .config_test_proxy
        .invoke(&state.service_connections, &service_id, payload, timeout)
        .await
    {
        Ok(result) => {
            let mut resp = TestPluginConfigResponse::new(
                result.success,
                test_kind_str.to_string(),
                result.duration_ms,
            );
            resp.output = result.output;
            resp.error = result.error;
            resp.detected_version = result.detected_version;
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(ConfigTestProxyError::Timeout) => error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "Agent did not respond within the timeout",
        ),
        Err(ConfigTestProxyError::ServiceDisconnected) => {
            error_response(StatusCode::BAD_GATEWAY, "Agent disconnected during test")
        }
        Err(ConfigTestProxyError::SendFailed) => error_response(
            StatusCode::BAD_GATEWAY,
            "Failed to send test request to agent",
        ),
    }
}

mod audit;
mod batch;
mod command_safety;
mod crud;
mod discover;

#[cfg(test)]
mod tests;
