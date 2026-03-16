//! WebSocket endpoint for interactive update sessions.
//!
//! Provides bidirectional stdin/stdout forwarding between a browser terminal
//! and the agent executing an update. Requires `ManageSoftware` permission.
//!
//! This entire module is gated on the `interactive` feature.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, RelationTrait};
use uptrakit_internal_wire::{ControllerMessage, UpdateStdinDataPayload};
use uptrakit_shared_db::entity::{host, service_host, update_history, update_output_line};
use uptrakit_web_api_types::update_history::{
    OutputLineSSE, StdinAttentionSSE, UpdateCompletedSSE,
};
use uuid::Uuid;

use crate::AppState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::{
    AuthenticatedUser, authenticate_api_token, authenticate_jwt,
};
use crate::update_output_broadcaster::BroadcastEvent;

/// Maximum size of a single WebSocket message from the client (256 KB).
const MAX_INTERACTIVE_WS_MESSAGE_SIZE: usize = 256 * 1024;

/// Client-to-server WebSocket message for interactive sessions.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    /// Send raw bytes to the process stdin.
    Stdin {
        /// Base64-encoded data.
        data: String,
    },
    /// Send a signal to the process group.
    Signal {
        /// Signal number (e.g. 2 = SIGINT, 15 = SIGTERM).
        signal: i32,
    },
}

/// Server-to-client WebSocket message wrapping broadcast events.
#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Output(OutputLineSSE),
    Completed(UpdateCompletedSSE),
    StdinAttention(StdinAttentionSSE),
    Error { message: String },
}

/// WebSocket endpoint for interactive update sessions.
///
/// Authenticates via Bearer token in the `Authorization` header or as a
/// `token` query parameter (necessary for browser WebSocket connections
/// which cannot set custom headers).
///
/// Requires `ManageSoftware` permission (stdin implies code execution trust).
///
/// Single-writer: only one user can hold an interactive session per update.
#[tracing::instrument(skip_all)]
pub async fn interactive_ws(
    State(state): State<Arc<AppState>>,
    Path(record_id): Path<Uuid>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // 1. Extract token from query param or Authorization header.
    let token = match query
        .get("token")
        .cloned()
        .or_else(|| extract_bearer(&headers))
    {
        Some(t) => t,
        None => return error_response(StatusCode::UNAUTHORIZED, "Authentication required"),
    };

    // 2. Validate the token.
    let auth_user = if token.starts_with("upk_") {
        match authenticate_api_token(&state, &token).await {
            Ok(user) => user,
            Err(e) => return e.into_response(),
        }
    } else {
        match authenticate_jwt(&state, &token).await {
            Ok(user) => user,
            Err(e) => return e.into_response(),
        }
    };

    // 3. Check TriggerUpdates permission.
    //
    // NOTE: This is an intentional approved exception to the standard Axum extractor pattern
    // (e.g. `CanTriggerUpdates`). WebSocket connections from the browser cannot set custom
    // HTTP headers, so the auth token arrives as a `?token=` query parameter. The custom
    // extraction logic above (steps 1-2) handles both sources. The permission check must
    // therefore live inline here rather than in a middleware extractor.
    if !auth_user.has_permission(Permission::TriggerUpdates) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    // 4. Verify the update record exists (tenant-scoped) and is in-progress.
    let tenant_db =
        uptrakit_web_api_queries::TenantDb::new(state.db.clone(), state.default_tenant_id);
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
            tracing::error!("Failed to load update history for interactive WS: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if record.status != update_history::UpdateStatus::InProgress {
        return error_response(StatusCode::CONFLICT, "Update is not in progress");
    }

    // 5. Claim interactive session (single-writer enforcement).
    if let Err(owner_id) = state
        .interactive_sessions
        .try_claim(record_id, auth_user.user_id)
    {
        return error_response(
            StatusCode::CONFLICT,
            format!("Interactive session already held by user {owner_id}"),
        );
    }

    // 6. Look up the agent (service) linked to this update's host (tenant-scoped via
    //    join on service — service_host has no tenant_id column).
    use uptrakit_shared_db::entity::service;
    let service_id = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.eq(record.host_id))
        .one(tenant_db.db())
        .await
    {
        Ok(Some(link)) => link.service_id,
        Ok(None) => {
            state
                .interactive_sessions
                .release(record_id, auth_user.user_id);
            return error_response(StatusCode::NOT_FOUND, "No agent linked to this host");
        }
        Err(e) => {
            state
                .interactive_sessions
                .release(record_id, auth_user.user_id);
            tracing::error!("Failed to find agent for interactive session: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // 7. Verify the agent is connected.
    if !state.service_connections.is_connected(&service_id).await {
        state
            .interactive_sessions
            .release(record_id, auth_user.user_id);
        return error_response(StatusCode::CONFLICT, "Agent is not connected");
    }

    // 8. Accept the WebSocket upgrade.
    tracing::info!(
        user_id = %auth_user.user_id,
        %record_id,
        %service_id,
        "interactive session established"
    );

    ws.max_message_size(MAX_INTERACTIVE_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| {
            handle_interactive_session(socket, state, record_id, service_id, auth_user)
        })
}

/// Main loop for an interactive WebSocket session.
///
/// Relays stdin from the client to the agent and output from the broadcaster
/// back to the client.
async fn handle_interactive_session(
    socket: WebSocket,
    state: Arc<AppState>,
    update_history_id: Uuid,
    service_id: Uuid,
    user: AuthenticatedUser,
) {
    let (mut sink, mut stream) = socket.split();

    // Subscribe to the output broadcaster for this update.
    let broadcast_rx = state
        .broadcast
        .update_output_broadcaster
        .subscribe(update_history_id)
        .await;

    let mut broadcast_rx = match broadcast_rx {
        Some(rx) => rx,
        None => {
            // No active broadcast channel — update may have already completed.
            let msg = ServerMessage::Error {
                message: "No active output stream for this update".to_string(),
            };
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
            let _ = sink.send(Message::Close(None)).await;
            state
                .interactive_sessions
                .release(update_history_id, user.user_id);
            return;
        }
    };

    // Replay existing output lines from the DB so the client sees output that
    // arrived before this WebSocket connected. The subscription was created
    // above *before* this query so that no lines are lost: any line that
    // arrives between the query and the main loop is buffered in the
    // broadcast receiver. Lines already covered by the DB replay are skipped
    // in the main loop via the `seq < replay_count` guard.
    let replay_count = match update_output_line::Entity::find()
        .filter(update_output_line::Column::UpdateHistoryId.eq(update_history_id))
        .order_by_asc(update_output_line::Column::CreatedAt)
        .order_by_asc(update_output_line::Column::Id)
        .all(&state.db)
        .await
    {
        Ok(lines) => {
            let count = lines.len() as u64;
            for (seq, line) in lines.into_iter().enumerate() {
                let payload = ServerMessage::Output(OutputLineSSE {
                    id: line.id,
                    text: line.output,
                    stream: line.stream.to_string(),
                    timestamp: line.created_at,
                    seq: seq as u64,
                });
                if let Ok(json) = serde_json::to_string(&payload)
                    && sink.send(Message::Text(json.into())).await.is_err()
                {
                    // Client disconnected during replay.
                    state
                        .interactive_sessions
                        .release(update_history_id, user.user_id);
                    return;
                }
            }
            count
        }
        Err(e) => {
            tracing::warn!(%update_history_id, "failed to load output lines for replay: {e}");
            0
        }
    };

    let shutdown_token = state.shutdown_token.clone();

    loop {
        tokio::select! {
            // Client → server: stdin or signal
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_client_message(
                            &state,
                            &text,
                            update_history_id,
                            service_id,
                        )
                        .await;
                    }
                    Some(Ok(Message::Binary(data))) => {
                        // Treat raw binary as stdin data (no JSON envelope).
                        use base64::Engine;
                        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
                        let payload = UpdateStdinDataPayload::new(update_history_id, encoded);
                        let msg = ControllerMessage::UpdateStdinData(payload);
                        if !state.service_connections.send(&service_id, msg).await {
                            tracing::warn!("failed to forward stdin to agent (disconnected?)");
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Err(e)) => {
                        tracing::debug!("interactive WS recv error: {e}");
                        break;
                    }
                }
            }

            // Server → client: broadcast events
            ev = broadcast_rx.recv() => {
                match ev {
                    Ok(BroadcastEvent::Line { id, text, stream, timestamp, seq }) => {
                        // Skip lines already sent via DB replay.
                        if seq < replay_count {
                            continue;
                        }
                        let payload = ServerMessage::Output(OutputLineSSE {
                            id,
                            text,
                            stream: stream.to_string(),
                            timestamp,
                            seq,
                        });
                        if let Ok(json) = serde_json::to_string(&payload)
                            && sink.send(Message::Text(json.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Ok(BroadcastEvent::Completed { status, error }) => {
                        let payload = ServerMessage::Completed(UpdateCompletedSSE {
                            status,
                            error,
                        });
                        if let Ok(json) = serde_json::to_string(&payload) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        break;
                    }
                    Ok(BroadcastEvent::StdinAttention { hint }) => {
                        let payload = ServerMessage::StdinAttention(StdinAttentionSSE { hint });
                        if let Ok(json) = serde_json::to_string(&payload)
                            && sink.send(Message::Text(json.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(lagged = n, "interactive WS subscriber lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }

            // Shutdown
            _ = shutdown_token.cancelled() => break,
        }
    }

    // Release the interactive session.
    state
        .interactive_sessions
        .release(update_history_id, user.user_id);

    tracing::info!(
        user_id = %user.user_id,
        %update_history_id,
        "interactive session closed"
    );
}

/// Parse and forward a client text message to the agent.
async fn handle_client_message(
    state: &AppState,
    text: &str,
    update_history_id: Uuid,
    service_id: Uuid,
) {
    let client_msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!("invalid interactive WS message: {e}");
            return;
        }
    };

    let payload = match client_msg {
        ClientMessage::Stdin { data } => UpdateStdinDataPayload::new(update_history_id, data),
        ClientMessage::Signal { signal } => {
            UpdateStdinDataPayload::with_signal(update_history_id, signal)
        }
    };

    let msg = ControllerMessage::UpdateStdinData(payload);
    if !state.service_connections.send(&service_id, msg).await {
        tracing::warn!(
            %service_id,
            "failed to forward stdin/signal to agent (disconnected?)"
        );
    }
}

/// Extract a bearer token from the Authorization header.
fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stdin_message() {
        let json = r#"{"type":"stdin","data":"aGVsbG8="}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Stdin { data } => assert_eq!(data, "aGVsbG8="),
            _ => panic!("expected Stdin"),
        }
    }

    #[test]
    fn parse_signal_message() {
        let json = r#"{"type":"signal","signal":2}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Signal { signal } => assert_eq!(signal, 2),
            _ => panic!("expected Signal"),
        }
    }

    #[test]
    fn serialize_output_message() {
        let msg = ServerMessage::Output(OutputLineSSE {
            id: Uuid::nil(),
            text: "hello".to_string(),
            stream: "stdout".to_string(),
            timestamp: time::OffsetDateTime::UNIX_EPOCH,
            seq: 0,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"output"#));
        assert!(json.contains(r#""text":"hello"#));
    }

    #[test]
    fn serialize_completed_message() {
        let msg = ServerMessage::Completed(UpdateCompletedSSE {
            status: "completed".to_string(),
            error: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"completed"#));
    }

    #[test]
    fn serialize_stdin_attention_message() {
        let msg = ServerMessage::StdinAttention(StdinAttentionSSE {
            hint: Some("Press Y/n".to_string()),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"stdin_attention"#));
        assert!(json.contains("Press Y/n"));
    }

    #[test]
    fn serialize_error_message() {
        let msg = ServerMessage::Error {
            message: "oops".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"error"#));
    }
}
