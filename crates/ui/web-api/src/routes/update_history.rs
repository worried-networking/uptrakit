use std::convert::Infallible;
use std::sync::Arc;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::CanViewSoftware;
use crate::queries::update_history as uh_queries;
use crate::tenant_db::TenantDb;
use crate::update_output_broadcaster::BroadcastEvent;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, RelationTrait};
use uptrakit_shared_db::entity::{host, update_history, update_output_line};
use uptrakit_web_api_types::update_history::{
    OutputLineSSE, StdinAttentionSSE, UpdateCompletedSSE,
};
use uuid::Uuid;

pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::update_history::{
    UpdateHistoryQuery, UpdateHistoryResponse, UpdateStatus,
};

// --- Endpoints ---

/// List update history records (filterable by host_id, software_item_id, status).
#[utoipa::path(
    get,
    path = "/api/v1/update-history",
    params(
        ("host_id" = Option<String>, Query, description = "Filter by host UUID"),
        ("software_item_id" = Option<String>, Query, description = "Filter by software item UUID"),
        ("status" = Option<String>, Query, description = "Filter by status (pending, in_progress, completed, failed)"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of update history records", body = PaginatedResponse<UpdateHistoryResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Update History",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_update_history(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(query): Query<UpdateHistoryQuery>,
) -> Response {
    match uh_queries::list_update_history(&tenant_db, &query).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list update history");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single update history record by ID.
#[utoipa::path(
    get,
    path = "/api/v1/update-history/{id}",
    params(("id" = Uuid, Path, description = "Update history record UUID")),
    responses(
        (status = 200, description = "Update history record", body = UpdateHistoryResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Record not found")
    ),
    tag = "Update History",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_update_history(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Path(record_id): Path<Uuid>,
) -> Response {
    match uh_queries::get_update_history(&tenant_db, record_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Update history record not found"),
        Err(e) => {
            tracing::error!(error = %e, "Failed to get update history record");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ---------------------------------------------------------------------------
// SSE output stream
// ---------------------------------------------------------------------------

/// Stream update output in real-time via Server-Sent Events.
///
/// For in-progress updates: replays stored output lines, then streams live
/// output from the broadcaster. For completed/failed updates: replays the
/// stored output and sends a `completed` event.
#[utoipa::path(
    get,
    path = "/api/v1/update-history/{id}/output/stream",
    params(("id" = Uuid, Path, description = "Update history record UUID")),
    responses(
        (status = 200, description = "SSE output stream", content_type = "text/event-stream"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Record not found")
    ),
    tag = "Update History",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn stream_update_output(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    State(state): State<Arc<AppState>>,
    Path(record_id): Path<Uuid>,
) -> Response {
    // 1. Load the update_history record, scoped to the tenant via host JOIN.
    //    A single query replaces the previous two-step (load-then-verify) pattern,
    //    preventing a TOCTOU window where record data was returned before the
    //    tenant check.
    let record = match tenant_db
        .find_via_tenant_join::<update_history::Entity, host::Entity>(
            update_history::Relation::Host.def(),
        )
        .filter(update_history::Column::Id.eq(record_id))
        .one(tenant_db.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Update history record not found");
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to load update history for SSE");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // 2. Subscribe to the broadcast channel BEFORE loading DB lines to avoid
    //    a gap where lines arrive after the DB query but before subscription.
    let broadcast_rx = state
        .broadcast
        .update_output_broadcaster
        .subscribe(record_id)
        .await;

    // 3. Load existing output lines from the DB for replay.
    let db_lines = match update_output_line::Entity::find()
        .filter(update_output_line::Column::UpdateHistoryId.eq(record_id))
        .order_by_asc(update_output_line::Column::CreatedAt)
        .order_by_asc(update_output_line::Column::Id)
        .all(tenant_db.db())
        .await
    {
        Ok(lines) => lines,
        Err(e) => {
            tracing::error!(error = %e, "Failed to load output lines for SSE stream");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let is_terminal = matches!(
        record.status,
        update_history::UpdateStatus::Completed | update_history::UpdateStatus::Failed
    );
    let _shutdown_token = state.shutdown_token.clone();
    let terminal_status = match record.status {
        update_history::UpdateStatus::Completed => "completed",
        update_history::UpdateStatus::Failed => "failed",
        _ => "",
    };

    // If the update is already terminal and there are no streaming lines,
    // fall back to the consolidated output stored on the record.
    let _has_db_lines = !db_lines.is_empty();
    let replay_count = db_lines.len() as u64;

    let stream = async_stream::stream! {
        // Replay DB lines.
        if !db_lines.is_empty() {
            for (seq, line) in db_lines.into_iter().enumerate() {
                let payload = OutputLineSSE {
                    id: line.id,
                    text: line.output,
                    stream: line.stream.to_string(),
                    timestamp: line.created_at,
                    seq: seq as u64,
                };
                if let Ok(json) = serde_json::to_string(&payload) {
                    yield Ok::<_, Infallible>(Event::default().event("output").data(json));
                }
            }
        } else if is_terminal && !record.output.is_empty() {
            // No transient lines, but the update has consolidated output.
            let payload = OutputLineSSE {
                id: record_id,
                text: record.output.clone(),
                stream: "stdout".to_string(),
                timestamp: record.completed_at.or(record.started_at).unwrap_or_else(time::OffsetDateTime::now_utc),
                seq: 0,
            };
            if let Ok(json) = serde_json::to_string(&payload) {
                yield Ok::<_, Infallible>(Event::default().event("output").data(json));
            }
        }

        // If update is already terminal, send completed and stop.
        if is_terminal {
            let payload = UpdateCompletedSSE {
                status: terminal_status.to_string(),
                error: None,
            };
            if let Ok(json) = serde_json::to_string(&payload) {
                yield Ok::<_, Infallible>(Event::default().event("completed").data(json));
            }
            return;
        }

        // Stream from broadcast (if we got a subscription).
        if let Some(mut rx) = broadcast_rx {
            loop {
                tokio::select! {
                    ev = rx.recv() => {
                        match ev {
                            Ok(BroadcastEvent::Line { id, text, stream, timestamp, seq }) => {
                                // Skip lines already replayed from DB.
                                if seq < replay_count {
                                    continue;
                                }
                                let payload = OutputLineSSE {
                                    id,
                                    text,
                                    stream: stream.to_string(),
                                    timestamp,
                                    seq,
                                };
                                if let Ok(json) = serde_json::to_string(&payload) {
                                    yield Ok::<_, Infallible>(Event::default().event("output").data(json));
                                }
                            }
                            Ok(BroadcastEvent::Completed { status, error }) => {
                                let payload = UpdateCompletedSSE { status, error };
                                if let Ok(json) = serde_json::to_string(&payload) {
                                    yield Ok::<_, Infallible>(Event::default().event("completed").data(json));
                                }
                                return;
                            }
                            Ok(BroadcastEvent::StdinAttention { hint }) => {
                                let payload = StdinAttentionSSE { hint };
                                if let Ok(json) = serde_json::to_string(&payload) {
                                    yield Ok::<_, Infallible>(Event::default().event("stdin_attention").data(json));
                                }
                            }
                            Ok(BroadcastEvent::AgentClaimed { service_id }) => {
                                let payload = serde_json::json!({ "service_id": service_id });
                                if let Ok(json) = serde_json::to_string(&payload) {
                                    yield Ok::<_, Infallible>(Event::default().event("agent_claimed").data(json));
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::debug!(lagged = n, "SSE subscriber lagged, continuing");
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                // Channel closed without a Completed event (e.g. server shutdown).
                                return;
                            }
                        }
                    }
                    _ = _shutdown_token.cancelled() => {
                        // Server is shutting down; terminate the SSE stream.
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
