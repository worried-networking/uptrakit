use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::action::CanTriggerChecks;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, JoinType, QueryFilter, QuerySelect, RelationTrait,
};
use std::sync::Arc;
use uptrakit_shared_db::entity::{host, plugin_config, prelude::*, service, service_host};
use uptrakit_shared_types::PluginTypeId;
use uptrakit_web_api_types::autodiscovery::TriggerDiscoveryResponse;
use uuid::Uuid;

// ── Autodiscovery endpoints ───────────────────────────────────────────────────

/// Trigger autodiscovery for a specific plugin configuration.
///
/// Sends a `DiscoverSoftware` assignment to all connected agents.
/// Returns an error if the plugin type does not support discovery.
#[utoipa::path(
    post,
    path = "/api/v1/plugin-configs/{id}/discover",
    params(("id" = Uuid, Path, description = "Plugin config UUID")),
    responses(
        (status = 200, description = "Discovery triggered", body = TriggerDiscoveryResponse),
        (status = 400, description = "Plugin type does not support discovery"),
        (status = 404, description = "Plugin config not found")
    ),
    tag = "Plugin Configs",
    security(("oauth2" = ["checks:trigger"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn discover_plugin_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerChecks(_user): CanTriggerChecks,
    Path(config_id): Path<Uuid>,
) -> Response {
    // Load the plugin config and verify it belongs to the tenant.
    let cfg = match PluginConfig::find_by_id(config_id)
        .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Plugin config not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Validate plugin supports discovery.
    let plugin_type_id = PluginTypeId::new(&cfg.plugin_type);

    if !state
        .plugin
        .plugin_ops
        .discovery_plugins()
        .contains(&plugin_type_id)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Plugin type '{}' does not support autodiscovery",
                cfg.plugin_type
            ),
        );
    }

    // Single JOIN query: service_host → service (tenant-scoped) → host
    // This prevents cross-tenant data leaks and eliminates the N+1 pattern.
    #[derive(FromQueryResult)]
    struct AgentHostRow {
        service_id: uuid::Uuid,
        machine_id: String,
    }

    let rows: Vec<AgentHostRow> = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .join(JoinType::InnerJoin, service_host::Relation::Host.def())
        .select_only()
        .column(service_host::Column::ServiceId)
        .column(host::Column::MachineId)
        .filter(service::Column::DeactivatedAt.is_null())
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(host::Column::DeactivatedAt.is_null())
        .into_model::<AgentHostRow>()
        .all(tenant_db.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to query service-host links: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Group machine_ids by service_id.
    let mut by_service: std::collections::HashMap<uuid::Uuid, Vec<String>> =
        std::collections::HashMap::new();
    for row in rows {
        by_service
            .entry(row.service_id)
            .or_default()
            .push(row.machine_id);
    }

    let agents_notified = by_service.len() as u32;

    // One DiscoverSoftware message per (agent, host) pair.
    for (agent_id, machine_ids) in &by_service {
        for machine_id in machine_ids {
            let msg = uptrakit_wire::ControllerMessage::DiscoverSoftware(
                uptrakit_wire::DiscoverSoftwarePayload {
                    host_machine_id: machine_id.clone(),
                    plugins: vec![uptrakit_wire::DiscoveryPluginAssignment {
                        plugin_config_id: Some(cfg.id),
                        plugin_type: PluginTypeId::new(cfg.plugin_type.clone()),
                        config: cfg.config.clone(),
                    }],
                },
            );
            state
                .notification
                .notification_service
                .send(agent_id, msg)
                .await;
        }
    }

    (
        StatusCode::OK,
        Json(TriggerDiscoveryResponse {
            plugins_queued: agents_notified,
            message: format!(
                "Discovery triggered for plugin config '{}' on {} agent(s)",
                cfg.name, agents_notified
            ),
        }),
    )
        .into_response()
}
