//! Route handlers for batch update operations.
//!
//! Provides endpoints for triggering host-wide and item-wide batch updates,
//! listing batches, retrieving batch details, and streaming batch progress.

use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use uuid::Uuid;

use uptrakit_shared_db::entity::{host, update_batch, update_history};
use uptrakit_shared_types::BatchStatus;
use uptrakit_web_api_types::update_batches::{
    HostBatchUpdateRequest, ItemBatchUpdateRequest, UpdateBatchDetailResponse, UpdateBatchListQuery,
    UpdateBatchSummaryResponse,
};
use uptrakit_web_api_types::validation::Validate;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::AppState;
use crate::batch_progress_broadcaster::BatchProgressEvent;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSoftware, CanViewSoftware};
use crate::queries::{update_batches as batch_queries, update_triggers::TriggerUpdateError};
use crate::tenant_db::TenantDb;

pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::update_batches::{
    BatchSkippedItem, BatchUpdateItem, BatchUpdateResponse,
};

// ---------------------------------------------------------------------------
// Host batch update
// ---------------------------------------------------------------------------

/// Trigger a batch update for all outdated software items on a host.
///
/// Finds all software items where `installed_version != latest_version`,
/// optionally filtered by update category. Creates a batch and dispatches
/// updates sequentially per host.
#[utoipa::path(
    post,
    path = "/api/v1/hosts/{host_id}/batch-update",
    params(("host_id" = Uuid, Path, description = "Host UUID")),
    request_body = HostBatchUpdateRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Batch update triggered", body = BatchUpdateResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Host not found")
    ),
    tag = "Update Batches",
    security(("bearer_token" = []))
)]
pub async fn trigger_host_batch_update(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(user): CanManageSoftware,
    Path(host_id): Path<Uuid>,
    Json(req): Json<HostBatchUpdateRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let candidates = match batch_queries::find_outdated_items_for_host(
        tenant_db.db(),
        tenant_db.tenant_id,
        host_id,
        req.category_filter.as_deref(),
        req.exclude_item_ids.as_deref(),
    )
    .await
    {
        Ok(c) => c,
        Err(report) => return trigger_error_to_response(report),
    };

    match batch_queries::create_batch(
        tenant_db.db(),
        &state.notification_service,
        &batch_queries::CreateBatchParams {
            tenant_id: tenant_db.tenant_id,
            batch_type: "host_update",
            actor_type: "user",
            actor_id: &user.user_id.to_string(),
        },
        candidates,
    )
    .await
    {
        Ok(resp) => {
            // Create broadcast channel for batch progress SSE streaming.
            if let Some(batch_id) = resp.batch_id {
                state
                    .batch_progress_broadcaster
                    .create_channel(batch_id)
                    .await;
            }
            // Push updated software states so MQTT entities reflect in_progress
            state
                .notification_service
                .push_software_states_for_tenant(tenant_db.db(), tenant_db.tenant_id)
                .await;
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(report) => trigger_error_to_response(report),
    }
}

// ---------------------------------------------------------------------------
// Item batch update
// ---------------------------------------------------------------------------

/// Trigger a batch update to roll out a software item to all (or selected) hosts.
///
/// Creates update_history records for each target host and dispatches the
/// first pending update per host.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/batch-update",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    request_body = ItemBatchUpdateRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Batch update triggered", body = BatchUpdateResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Update Batches",
    security(("bearer_token" = []))
)]
pub async fn trigger_item_batch_update(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(user): CanManageSoftware,
    Path(item_id): Path<Uuid>,
    Json(req): Json<ItemBatchUpdateRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let candidates = match batch_queries::find_outdated_hosts_for_item(
        tenant_db.db(),
        tenant_db.tenant_id,
        item_id,
        req.host_ids.as_deref(),
    )
    .await
    {
        Ok(mut c) => {
            // Override the to_version for all candidates to the requested version
            for candidate in &mut c {
                candidate.latest_version = req.to_version.clone();
            }
            c
        }
        Err(report) => return trigger_error_to_response(report),
    };

    match batch_queries::create_batch(
        tenant_db.db(),
        &state.notification_service,
        &batch_queries::CreateBatchParams {
            tenant_id: tenant_db.tenant_id,
            batch_type: "item_rollout",
            actor_type: "user",
            actor_id: &user.user_id.to_string(),
        },
        candidates,
    )
    .await
    {
        Ok(resp) => {
            // Create broadcast channel for batch progress SSE streaming.
            if let Some(batch_id) = resp.batch_id {
                state
                    .batch_progress_broadcaster
                    .create_channel(batch_id)
                    .await;
            }
            state
                .notification_service
                .push_software_states_for_tenant(tenant_db.db(), tenant_db.tenant_id)
                .await;
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(report) => trigger_error_to_response(report),
    }
}

// ---------------------------------------------------------------------------
// List batches
// ---------------------------------------------------------------------------

/// List update batches for the current tenant (paginated).
#[utoipa::path(
    get,
    path = "/api/v1/update-batches",
    params(
        ("status" = Option<String>, Query, description = "Filter by batch status (in_progress, completed, partially_completed)"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of update batches", body = PaginatedResponse<UpdateBatchSummaryResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Update Batches",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
pub async fn list_batches(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(query): Query<UpdateBatchListQuery>,
) -> Response {
    match batch_queries::list_batches(&tenant_db, &query).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list update batches: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ---------------------------------------------------------------------------
// Get batch detail
// ---------------------------------------------------------------------------

/// Get a single update batch with per-item update details.
#[utoipa::path(
    get,
    path = "/api/v1/update-batches/{id}",
    params(("id" = Uuid, Path, description = "Update batch UUID")),
    responses(
        (status = 200, description = "Update batch detail", body = UpdateBatchDetailResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Batch not found")
    ),
    tag = "Update Batches",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
pub async fn get_batch(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Path(batch_id): Path<Uuid>,
) -> Response {
    match batch_queries::get_batch_with_items(&tenant_db, batch_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Update batch not found"),
        Err(e) => {
            tracing::error!("Failed to get update batch: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ---------------------------------------------------------------------------
// SSE batch progress stream
// ---------------------------------------------------------------------------

/// Stream batch progress in real-time via Server-Sent Events.
///
/// For in-progress batches: replays current per-item status, then streams live
/// progress events from the broadcaster. For terminal batches: sends the final
/// state and a `batch_completed` event.
#[utoipa::path(
    get,
    path = "/api/v1/update-batches/{id}/stream",
    params(("id" = Uuid, Path, description = "Update batch UUID")),
    responses(
        (status = 200, description = "SSE batch progress stream", content_type = "text/event-stream"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Batch not found")
    ),
    tag = "Update Batches",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
pub async fn stream_batch_progress(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    State(state): State<Arc<AppState>>,
    Path(batch_id): Path<Uuid>,
) -> Response {
    // 1. Load the batch record (tenant-scoped).
    let batch = match tenant_db
        .find_by_id::<update_batch::Entity, _>(batch_id)
        .one(tenant_db.db())
        .await
    {
        Ok(Some(b)) => b,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Update batch not found"),
        Err(e) => {
            tracing::error!("Failed to load update batch for SSE: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let is_terminal = matches!(
        batch.status,
        BatchStatus::Completed | BatchStatus::PartiallyCompleted
    );

    // 2. Subscribe to the broadcast channel BEFORE loading DB state to avoid gaps.
    let broadcast_rx = state.batch_progress_broadcaster.subscribe(batch_id).await;

    // 3. Load current child update records for replay.
    let children = match update_history::Entity::find()
        .filter(update_history::Column::BatchId.eq(batch_id))
        .order_by_asc(update_history::Column::Id)
        .all(tenant_db.db())
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load batch children for SSE: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Batch-load host and software item names for replay events.
    let host_ids: Vec<Uuid> = children
        .iter()
        .map(|c| c.host_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let si_ids: Vec<Uuid> = children
        .iter()
        .map(|c| c.software_item_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let host_names: std::collections::HashMap<Uuid, String> = host::Entity::find()
        .filter(host::Column::Id.is_in(host_ids))
        .all(tenant_db.db())
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|h| (h.id, h.friendly_name))
        .collect();

    let si_names: std::collections::HashMap<Uuid, String> =
        uptrakit_shared_db::entity::software_item::Entity::find()
            .filter(uptrakit_shared_db::entity::software_item::Column::Id.is_in(si_ids))
            .all(tenant_db.db())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|si| (si.id, si.name))
            .collect();

    let total = batch.total_count;
    let batch_status_str = batch.status.as_str().to_string();

    let stream = async_stream::stream! {
        // Replay per-item status from DB.
        let mut completed: i64 = 0;
        let mut failed: i64 = 0;
        let mut pending: i64 = 0;

        for child in &children {
            let host_name = host_names
                .get(&child.host_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Host".to_string());
            let sw_name = si_names
                .get(&child.software_item_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Software".to_string());

            let event = match child.status {
                update_history::UpdateStatus::Completed => {
                    completed += 1;
                    BatchProgressEvent::UpdateCompleted {
                        update_history_id: child.id,
                        software_item_name: sw_name,
                        host_name,
                    }
                }
                update_history::UpdateStatus::Failed => {
                    failed += 1;
                    BatchProgressEvent::UpdateFailed {
                        update_history_id: child.id,
                        software_item_name: sw_name,
                        host_name,
                        error: None,
                    }
                }
                update_history::UpdateStatus::InProgress => {
                    pending += 1;
                    BatchProgressEvent::UpdateStarted {
                        update_history_id: child.id,
                        software_item_name: sw_name,
                        host_name,
                    }
                }
                update_history::UpdateStatus::Pending => {
                    pending += 1;
                    BatchProgressEvent::UpdateDispatched {
                        update_history_id: child.id,
                        software_item_name: sw_name,
                        host_name,
                    }
                }
                _ => {
                    tracing::warn!("Unknown update status {:?}, treating as pending", child.status);
                    pending += 1;
                    BatchProgressEvent::UpdateDispatched {
                        update_history_id: child.id,
                        software_item_name: sw_name,
                        host_name,
                    }
                }
            };

            if let Ok(json) = serde_json::to_string(&event) {
                yield Ok::<_, Infallible>(Event::default().event("update").data(json));
            }
        }

        // Send current progress summary.
        let progress = BatchProgressEvent::Progress {
            completed,
            failed,
            pending,
            total,
        };
        if let Ok(json) = serde_json::to_string(&progress) {
            yield Ok::<_, Infallible>(Event::default().event("progress").data(json));
        }

        // If the batch is already terminal, send completed and stop.
        if is_terminal {
            let completed_event = BatchProgressEvent::BatchCompleted {
                status: batch_status_str.clone(),
            };
            if let Ok(json) = serde_json::to_string(&completed_event) {
                yield Ok::<_, Infallible>(Event::default().event("batch_completed").data(json));
            }
            return;
        }

        // Stream from broadcast (if we got a subscription).
        if let Some(mut rx) = broadcast_rx {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let event_name = match &event {
                            BatchProgressEvent::UpdateDispatched { .. }
                            | BatchProgressEvent::UpdateStarted { .. }
                            | BatchProgressEvent::UpdateCompleted { .. }
                            | BatchProgressEvent::UpdateFailed { .. } => "update",
                            BatchProgressEvent::Progress { .. } => "progress",
                            BatchProgressEvent::BatchCompleted { .. } => "batch_completed",
                        };
                        let is_batch_completed = matches!(event, BatchProgressEvent::BatchCompleted { .. });
                        if let Ok(json) = serde_json::to_string(&event) {
                            yield Ok::<_, Infallible>(Event::default().event(event_name).data(json));
                        }
                        if is_batch_completed {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(lagged = n, "batch SSE subscriber lagged, continuing");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return;
                    }
                }
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default().interval(std::time::Duration::from_secs(15)))
        .into_response()
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn trigger_error_to_response(report: rootcause::Report<TriggerUpdateError>) -> Response {
    match report.current_context() {
        TriggerUpdateError::SoftwareItemNotFound => {
            error_response(StatusCode::NOT_FOUND, "Software item not found")
        }
        TriggerUpdateError::HostNotFound => {
            error_response(StatusCode::NOT_FOUND, "Host not found")
        }
        TriggerUpdateError::HostNotAssigned => error_response(
            StatusCode::BAD_REQUEST,
            "Host is not assigned to this software item",
        ),
        TriggerUpdateError::NoAgent => {
            error_response(StatusCode::NOT_FOUND, "No agent linked to this host")
        }
        TriggerUpdateError::AgentNotApproved => {
            error_response(StatusCode::BAD_REQUEST, "Agent is not approved")
        }
        TriggerUpdateError::UpdateAlreadyActive => error_response(
            StatusCode::CONFLICT,
            "An update is already pending or in progress",
        ),
        TriggerUpdateError::NoExecuteUpdatePlugin => error_response(
            StatusCode::BAD_REQUEST,
            "No execute_update plugin assigned for this host",
        ),
        TriggerUpdateError::PluginConfigNotFound => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Plugin config not found")
        }
        TriggerUpdateError::UnknownPluginType(pt) => {
            tracing::error!("Unknown plugin type: {pt}");
            error_response(StatusCode::BAD_REQUEST, "Unknown plugin type")
        }
        TriggerUpdateError::Database(_) => {
            tracing::error!("Database error in batch update: {report}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
