//! Route handlers for batch update operations.
//!
//! Provides endpoints for triggering host-wide and item-wide batch updates,
//! listing batches, retrieving batch details, and streaming batch progress.

use std::collections::HashMap;
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

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uptrakit_shared_db::entity::{host, software_item, update_batch, update_history};
use uptrakit_shared_types::{BatchStatus, UpdateStatus};
use uptrakit_web_api_types::update_batches::{
    HostBatchUpdateRequest, ItemBatchUpdateRequest, UpdateBatchDetailResponse,
    UpdateBatchListQuery, UpdateBatchSummaryResponse,
};

use crate::extract::Validated;

#[cfg(feature = "nats")]
use futures_util::StreamExt as _;

use crate::AppState;
use crate::actions::update_batches as batch_actions;
use crate::api_error::ApiError;
use crate::batch_progress_broadcaster::BatchProgressEvent;
use crate::error_response::error_response;
use crate::middleware::permission::{CanTriggerUpdates, CanViewSoftware};
use crate::queries::update_batches as batch_queries;
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
    extensions(("x-required-permission" = json!("trigger_updates"))),
    responses(
        (status = 200, description = "Batch update triggered", body = BatchUpdateResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Host not found")
    ),
    tag = "Update Batches",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn trigger_host_batch_update(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerUpdates(user): CanTriggerUpdates,
    Path(host_id): Path<Uuid>,
    Validated(req): Validated<HostBatchUpdateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = state.mutation_context();
    let resp = batch_actions::trigger_host_batch(
        &tenant_db,
        &ctx,
        &state.broadcast.batch_progress_broadcaster,
        host_id,
        user.user_id,
        req.category_filter.as_deref(),
        req.exclude_item_ids.as_deref(),
    )
    .await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
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
    extensions(("x-required-permission" = json!("trigger_updates"))),
    responses(
        (status = 200, description = "Batch update triggered", body = BatchUpdateResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Update Batches",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn trigger_item_batch_update(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerUpdates(user): CanTriggerUpdates,
    Path(item_id): Path<Uuid>,
    Validated(req): Validated<ItemBatchUpdateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = state.mutation_context();
    let resp = batch_actions::trigger_item_batch(
        &tenant_db,
        &ctx,
        &state.broadcast.batch_progress_broadcaster,
        item_id,
        user.user_id,
        req.to_version,
        req.host_ids.as_deref(),
    )
    .await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
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
#[tracing::instrument(skip_all)]
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
#[tracing::instrument(skip_all)]
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
// SSE batch progress — helper types and functions
// ---------------------------------------------------------------------------

/// Preloaded context for streaming batch progress events.
///
/// Holds the batch record and all related data needed to build replay events,
/// avoiding repeated database queries during the SSE stream.
struct BatchContext {
    batch: update_batch::Model,
    children: Vec<update_history::Model>,
    host_names: HashMap<Uuid, String>,
    item_names: HashMap<Uuid, String>,
}

impl BatchContext {
    /// Whether the batch has reached a terminal status.
    fn is_terminal(&self) -> bool {
        matches!(
            self.batch.status,
            BatchStatus::Completed | BatchStatus::PartiallyCompleted
        )
    }
}

/// Load the batch record, child update history rows, and related host/item
/// names from the database.
///
/// Returns an error response suitable for returning directly from a handler
/// when any query fails.
async fn load_batch_context(
    tenant_db: &TenantDb,
    batch_id: Uuid,
) -> Result<BatchContext, Response> {
    // Load the batch record (tenant-scoped).
    let batch = match tenant_db
        .find_by_id::<update_batch::Entity, _>(batch_id)
        .one(tenant_db.db())
        .await
    {
        Ok(Some(b)) => b,
        Ok(None) => {
            return Err(error_response(
                StatusCode::NOT_FOUND,
                "Update batch not found",
            ));
        }
        Err(e) => {
            tracing::error!("Failed to load update batch for SSE: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    // Load child update_history records.
    let children = match update_history::Entity::find()
        .filter(update_history::Column::BatchId.eq(batch_id))
        .order_by_asc(update_history::Column::Id)
        .all(tenant_db.db())
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load batch children for SSE: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    // Collect unique IDs for batch-loading names.
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

    let host_names = match host::Entity::find()
        .filter(host::Column::Id.is_in(host_ids))
        .all(tenant_db.db())
        .await
    {
        Ok(records) => records
            .into_iter()
            .map(|h| (h.id, h.friendly_name))
            .collect(),
        Err(e) => {
            tracing::error!("Failed to load host names for SSE replay: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let item_names = match software_item::Entity::find()
        .filter(software_item::Column::Id.is_in(si_ids))
        .all(tenant_db.db())
        .await
    {
        Ok(records) => records.into_iter().map(|si| (si.id, si.name)).collect(),
        Err(e) => {
            tracing::error!("Failed to load software item names for SSE replay: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    Ok(BatchContext {
        batch,
        children,
        host_names,
        item_names,
    })
}

/// Build a replay [`BatchProgressEvent`] from a child update_history record.
///
/// Returns the event and whether the child counts as completed, failed, or
/// pending (as a tuple of `(completed_delta, failed_delta, pending_delta)`).
fn build_replay_event(
    child: &update_history::Model,
    host_names: &HashMap<Uuid, String>,
    item_names: &HashMap<Uuid, String>,
) -> (BatchProgressEvent, i64, i64, i64) {
    let host_name = host_names
        .get(&child.host_id)
        .cloned()
        .unwrap_or_else(|| "Unknown Host".to_string());
    let sw_name = item_names
        .get(&child.software_item_id)
        .cloned()
        .unwrap_or_else(|| "Unknown Software".to_string());

    match child.status {
        UpdateStatus::Completed => (
            BatchProgressEvent::UpdateCompleted {
                update_history_id: child.id,
                software_item_name: sw_name,
                host_name,
            },
            1,
            0,
            0,
        ),
        UpdateStatus::Failed => (
            BatchProgressEvent::UpdateFailed {
                update_history_id: child.id,
                software_item_name: sw_name,
                host_name,
                error: None,
            },
            0,
            1,
            0,
        ),
        UpdateStatus::InProgress => (
            BatchProgressEvent::UpdateStarted {
                update_history_id: child.id,
                software_item_name: sw_name,
                host_name,
            },
            0,
            0,
            1,
        ),
        UpdateStatus::Pending | UpdateStatus::Queued => (
            BatchProgressEvent::UpdateDispatched {
                update_history_id: child.id,
                software_item_name: sw_name,
                host_name,
            },
            0,
            0,
            1,
        ),
        _ => {
            tracing::warn!(
                "Unknown update status {:?}, treating as pending",
                child.status
            );
            (
                BatchProgressEvent::UpdateDispatched {
                    update_history_id: child.id,
                    software_item_name: sw_name,
                    host_name,
                },
                0,
                0,
                1,
            )
        }
    }
}

/// Calculate progress counts from the batch children.
///
/// Returns `(completed, failed, pending)`.
fn calculate_batch_progress(children: &[update_history::Model]) -> (i64, i64, i64) {
    let mut completed: i64 = 0;
    let mut failed: i64 = 0;
    let mut pending: i64 = 0;
    for child in children {
        match child.status {
            UpdateStatus::Completed => completed += 1,
            UpdateStatus::Failed => failed += 1,
            _ => pending += 1,
        }
    }
    (completed, failed, pending)
}

/// Returns the SSE event name for a [`BatchProgressEvent`].
fn sse_event_name(event: &BatchProgressEvent) -> &'static str {
    match event {
        BatchProgressEvent::UpdateDispatched { .. }
        | BatchProgressEvent::UpdateStarted { .. }
        | BatchProgressEvent::UpdateCompleted { .. }
        | BatchProgressEvent::UpdateFailed { .. } => "update",
        BatchProgressEvent::Progress { .. } => "progress",
        BatchProgressEvent::BatchCompleted { .. } => "batch_completed",
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
#[tracing::instrument(skip_all)]
pub async fn stream_batch_progress(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    State(state): State<Arc<AppState>>,
    Path(batch_id): Path<Uuid>,
) -> Response {
    let ctx = match load_batch_context(&tenant_db, batch_id).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    let is_terminal = ctx.is_terminal();

    // Subscribe to the broadcast channel BEFORE replaying DB state to avoid gaps.
    let broadcast_rx = state
        .broadcast
        .batch_progress_broadcaster
        .subscribe(batch_id)
        .await;

    let (completed, failed, pending) = calculate_batch_progress(&ctx.children);
    let total = ctx.batch.total_count;
    let batch_status_str = ctx.batch.status.as_str().to_string();
    let shutdown_token = state.shutdown_token.clone();

    let stream = async_stream::stream! {
        // Replay per-item status from DB.
        for child in &ctx.children {
            let (event, _, _, _) = build_replay_event(child, &ctx.host_names, &ctx.item_names);
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

        // Stream from local broadcast channel when available.
        if let Some(mut rx) = broadcast_rx {
            loop {
                tokio::select! {
                    ev = rx.recv() => {
                        match ev {
                            Ok(event) => {
                                let is_done = matches!(event, BatchProgressEvent::BatchCompleted { .. });
                                if let Ok(json) = serde_json::to_string(&event) {
                                    yield Ok::<_, Infallible>(Event::default().event(sse_event_name(&event)).data(json));
                                }
                                if is_done {
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
                    _ = shutdown_token.cancelled() => {
                        return;
                    }
                }
            }
        }

        // No local channel: the batch is running on another controller instance.
        // Fall back to a NATS subscription when NATS is configured.
        #[cfg(feature = "nats")]
        if let Some(mut nats_sub) = state.broadcast.batch_progress_broadcaster.subscribe_nats(batch_id).await {
            tracing::debug!(
                batch_id = %batch_id,
                "no local broadcast channel; falling back to NATS subscription for SSE stream"
            );
            loop {
                tokio::select! {
                    msg = nats_sub.next() => {
                        let Some(msg) = msg else { return; };
                        let Ok(event) = serde_json::from_slice::<BatchProgressEvent>(&msg.payload) else {
                            tracing::warn!(batch_id = %batch_id, "received unparseable NATS batch progress event");
                            continue;
                        };
                        let is_done = matches!(event, BatchProgressEvent::BatchCompleted { .. });
                        if let Ok(json) = serde_json::to_string(&event) {
                            yield Ok::<_, Infallible>(Event::default().event(sse_event_name(&event)).data(json));
                        }
                        if is_done {
                            return;
                        }
                    }
                    _ = shutdown_token.cancelled() => {
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

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures::{register_and_get_token, seed_permissions_for_owner};
    use http::StatusCode;
    use serde_json::Value;

    #[tokio::test]
    async fn list_batches_unauthenticated_returns_401() {
        let app = TestApp::new().await;
        let client = app.client();
        let status = client.get("/api/v1/update-batches").send_status().await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_batches_authenticated_empty_db_returns_empty_list() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["view_software"]).await;
        let token = register_and_get_token(&client).await;

        let (status, body): (StatusCode, Value) = client
            .get("/api/v1/update-batches")
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, StatusCode::OK);
        let items = body["items"].as_array().expect("items array");
        assert!(items.is_empty(), "expected empty list on empty DB");
    }

    #[tokio::test]
    async fn get_batch_unauthenticated_returns_401() {
        let app = TestApp::new().await;
        let client = app.client();
        let id = uuid::Uuid::now_v7();
        let status = client
            .get(&format!("/api/v1/update-batches/{id}"))
            .send_status()
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_batch_not_found_returns_404() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["view_software"]).await;
        let token = register_and_get_token(&client).await;

        let id = uuid::Uuid::now_v7();
        let status = client
            .get(&format!("/api/v1/update-batches/{id}"))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn trigger_host_batch_update_not_found_returns_404() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["trigger_updates"]).await;
        let token = register_and_get_token(&client).await;

        let host_id = uuid::Uuid::now_v7();
        let body = serde_json::json!({});
        let status = client
            .post_json(&format!("/api/v1/hosts/{host_id}/batch-update"), &body)
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn trigger_item_batch_update_not_found_returns_404() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["trigger_updates"]).await;
        let token = register_and_get_token(&client).await;

        let item_id = uuid::Uuid::now_v7();
        // ItemBatchUpdateRequest requires `to_version`.
        let body = serde_json::json!({ "to_version": "1.0.0" });
        let status = client
            .post_json(
                &format!("/api/v1/software-items/{item_id}/batch-update"),
                &body,
            )
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
