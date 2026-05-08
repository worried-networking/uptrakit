use serde_json::json;
use uptrakit_wire::{
    ServiceTransport,
    surfaces::{SurfaceActionRequest, SurfaceActionResponse},
};

use crate::host_ops;

use super::{SSH_HOSTS_SURFACE_ID, SurfaceRuntimeContext};

#[path = "infra_plugin_orchestration.rs"]
mod infra_plugin_orchestration;

/// Dispatch a surface request to the appropriate handler.
///
/// Actions that complete quickly (`list-hosts`, `remove-host`) respond inline.
/// Long-running actions (`bootstrap`) are spawned as background tasks via `bg_tx`.
#[tracing::instrument(skip_all, fields(
    request_id = %request.request_id,
    surface_id = %request.surface_id,
    interaction_id = %request.interaction_id,
))]
pub(super) async fn handle_surface_action_request(
    request: SurfaceActionRequest,
    ctx: &SurfaceRuntimeContext<'_>,
    conn: &mut dyn ServiceTransport,
) {
    if request.surface_id.as_str() != SSH_HOSTS_SURFACE_ID {
        tracing::warn!(
            surface_id = %request.surface_id,
            "received request for unknown surface"
        );
        let response = super::make_surface_error_response(request.request_id, "unknown surface");
        super::send_response(conn, response).await;
        return;
    }

    if !super::is_registered_interaction(request.interaction_id.as_str()) {
        tracing::warn!(
            surface_id = %request.surface_id,
            action_id = %request.interaction_id,
            "received request for unregistered interaction"
        );
        let response = super::make_surface_error_response(request.request_id, "unknown action");
        super::send_response(conn, response).await;
        return;
    }

    match request.interaction_id.as_str() {
        "list-hosts" => {
            let response = handle_list_hosts(request.request_id, &request.params, ctx.db).await;
            super::send_response(conn, response).await;
        }
        "remove-host" => {
            let response = handle_remove_host(request.request_id, &request.params, ctx.db).await;
            super::send_response(conn, response).await;
        }
        "bootstrap-connect" => {
            super::spawn_bootstrap_connect(request, ctx);
        }
        "bootstrap" => {
            let response = super::make_surface_error_response(
                request.request_id,
                "workflow entry interaction cannot be executed directly",
            );
            super::send_response(conn, response).await;
        }
        "bootstrap-execute" => {
            super::spawn_bootstrap_execute(request, ctx);
        }
        "sync-connect" => {
            super::sync::spawn_sync_connect(request, ctx);
        }
        "sync-host" => {
            let response = super::make_surface_error_response(
                request.request_id,
                "workflow entry interaction cannot be executed directly",
            );
            super::send_response(conn, response).await;
        }
        "sync-execute" => {
            super::sync::spawn_sync_execute(request, ctx);
        }
        _ => {
            // Delegate to infrastructure plugins.
            infra_plugin_orchestration::spawn_infra_plugin_action(request, ctx);
        }
    }
}

/// List SSH hosts from the local database with pagination.
async fn handle_list_hosts(
    request_id: uuid::Uuid,
    params: &serde_json::Map<String, serde_json::Value>,
    db: &sea_orm::DatabaseConnection,
) -> SurfaceActionResponse {
    let page = params
        .get("page")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .clamp(1, 1000);

    match host_ops::list_hosts_paginated(db, page, per_page).await {
        Ok(result) => {
            let items: Vec<serde_json::Value> = result
                .items
                .into_iter()
                .map(|h| {
                    json!({
                        "id": h.id,
                        "name": h.name,
                        "hostname": h.hostname,
                        "port": h.port,
                        "username": h.username,
                    })
                })
                .collect();
            super::make_surface_success_response(
                request_id,
                json!({
                    "items": items,
                    "total": result.total,
                    "page": result.page,
                    "per_page": result.per_page,
                    "total_pages": result.total_pages,
                }),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list hosts");
            super::make_surface_error_response(request_id, "failed to list hosts")
        }
    }
}

/// Remove a host from the local database.
async fn handle_remove_host(
    request_id: uuid::Uuid,
    params: &serde_json::Map<String, serde_json::Value>,
    db: &sea_orm::DatabaseConnection,
) -> SurfaceActionResponse {
    let host_id = match params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return super::make_surface_error_response(request_id, "missing required field 'id'");
        }
    };

    match host_ops::remove_host(db, host_id).await {
        Ok(true) => super::make_surface_success_response(request_id, json!({ "removed": true })),
        Ok(false) => super::make_surface_error_response(request_id, "host not found"),
        Err(e) => {
            tracing::error!(error = %e, host = %host_id, "failed to remove host");
            super::make_surface_error_response(request_id, "failed to remove host")
        }
    }
}
