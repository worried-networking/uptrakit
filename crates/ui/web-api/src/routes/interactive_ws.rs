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
use uptrakit_shared_db::entity::{host, update_history, update_output_line};
use uptrakit_web_api_types::update_history::{
    OutputLineSSE, StdinAttentionSSE, UpdateCompletedSSE,
};
use uuid::Uuid;

use crate::AppState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::{
    AuthFailure, AuthenticatedApiTokenId, AuthenticatedUser, authenticate_api_token,
    authenticate_jwt, authenticated_user_audit_actor,
};
use crate::update_output_broadcaster::BroadcastEvent;

/// Maximum size of a single WebSocket message from the client (256 KB).
const MAX_INTERACTIVE_WS_MESSAGE_SIZE: usize = 256 * 1024;

#[derive(Clone, Copy)]
struct InteractiveAuditActor {
    actor_type: uptrakit_audit_log::AuditActorType,
    actor_id: Option<Uuid>,
}

impl InteractiveAuditActor {
    fn anonymous(actor_type: uptrakit_audit_log::AuditActorType) -> Self {
        Self {
            actor_type,
            actor_id: None,
        }
    }

    fn from_authenticated(
        user: &AuthenticatedUser,
        api_token_id: Option<AuthenticatedApiTokenId>,
    ) -> Self {
        let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);
        Self {
            actor_type,
            actor_id,
        }
    }
}

struct InteractiveAuditCtx<'a> {
    state: &'a AppState,
    actor: InteractiveAuditActor,
}

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
        None => {
            emit_interactive_auth_failure_audit(
                &state,
                record_id,
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::User),
                "missing",
                uptrakit_audit_log::AuditOutcome::Denied,
                "missing_token",
            );
            return error_response(StatusCode::UNAUTHORIZED, "Authentication required");
        }
    };

    // 2. Validate the token.
    let (auth_user, api_token_id) = if token.starts_with("upk_") {
        match authenticate_api_token(&state, &token).await {
            Ok((user, token_id)) => (user, Some(AuthenticatedApiTokenId(token_id))),
            Err(e) => {
                if let Some((actor, outcome, reason_code)) =
                    classify_interactive_auth_failure(&token, &e)
                {
                    emit_interactive_auth_failure_audit(
                        &state,
                        record_id,
                        actor,
                        auth_method_for_token(&token),
                        outcome,
                        reason_code,
                    );
                }
                return e.into_response();
            }
        }
    } else {
        match authenticate_jwt(&state, &token).await {
            Ok(user) => (user, None),
            Err(e) => {
                if let Some((actor, outcome, reason_code)) =
                    classify_interactive_auth_failure(&token, &e)
                {
                    emit_interactive_auth_failure_audit(
                        &state,
                        record_id,
                        actor,
                        auth_method_for_token(&token),
                        outcome,
                        reason_code,
                    );
                }
                return e.into_response();
            }
        }
    };
    let audit_actor = InteractiveAuditActor::from_authenticated(&auth_user, api_token_id);

    // 3. Check TriggerUpdates permission.
    //
    // NOTE: This is an intentional approved exception to the standard Axum extractor pattern
    // (e.g. `CanTriggerUpdates`). WebSocket connections from the browser cannot set custom
    // HTTP headers, so the auth token arrives as a `?token=` query parameter. The custom
    // extraction logic above (steps 1-2) handles both sources. The permission check must
    // therefore live inline here rather than in a middleware extractor.
    if !auth_user.has_permission(Permission::TriggerUpdates) {
        emit_interactive_session_audit(
            InteractiveAuditCtx {
                state: &state,
                actor: audit_actor,
            },
            record_id,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("permission_denied"),
            None,
        );
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    // 4. Verify the update record exists (tenant-scoped) and is in-progress.
    let tenant_db =
        uptrakit_web_api_queries::TenantDb::new(state.db().clone(), state.default_tenant_id);
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
            emit_interactive_session_audit(
                InteractiveAuditCtx {
                    state: &state,
                    actor: audit_actor,
                },
                record_id,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("update_history_not_found"),
                None,
            );
            return error_response(StatusCode::NOT_FOUND, "Update history record not found");
        }
        Err(e) => {
            tracing::error!("Failed to load update history for interactive WS: {e}");
            emit_interactive_session_audit(
                InteractiveAuditCtx {
                    state: &state,
                    actor: audit_actor,
                },
                record_id,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("update_history_lookup_failed"),
                Some(serde_json::json!({
                    "lookup_stage": "update_history",
                })),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if record.status != update_history::UpdateStatus::InProgress {
        emit_interactive_session_audit(
            InteractiveAuditCtx {
                state: &state,
                actor: audit_actor,
            },
            record_id,
            record.execution_owner_service_id,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("update_not_in_progress"),
            Some(serde_json::json!({
                "update_status": record.status.to_string(),
            })),
        );
        return error_response(StatusCode::CONFLICT, "Update is not in progress");
    }

    // 5. Claim interactive session (single-writer enforcement).
    if let Err(owner_id) = state
        .interactive_sessions
        .try_claim(record_id, auth_user.user_id)
    {
        emit_interactive_session_audit(
            InteractiveAuditCtx {
                state: &state,
                actor: audit_actor,
            },
            record_id,
            record.execution_owner_service_id,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("interactive_session_already_claimed"),
            Some(serde_json::json!({
                "active_owner_user_id": owner_id,
            })),
        );
        return error_response(
            StatusCode::CONFLICT,
            format!("Interactive session already held by user {owner_id}"),
        );
    }

    // 6. Resolve the executing agent's service_id from the update record.
    //
    // `execution_owner_service_id` is set by `claim_or_replay_update_start_db`
    // when the agent claims the update (transitions to InProgress). It is the
    // authoritative identifier for the service executing this specific update.
    //
    // Using this field is more reliable than a `service_host` join because:
    //   - The join with `.one()` has no ordering guarantee and may return a
    //     stale row when a host has been managed by more than one agent over
    //     time (e.g. after agent re-enrollment or a controller restart that
    //     provisioned a new embedded service record).
    //   - The execution owner is the exact service that opened the broadcast
    //     channel and is receiving stdin forwarding.
    let service_id = match record.execution_owner_service_id {
        Some(id) => id,
        None => {
            emit_interactive_session_audit(
                InteractiveAuditCtx {
                    state: &state,
                    actor: audit_actor,
                },
                record_id,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("execution_owner_missing"),
                None,
            );
            state
                .interactive_sessions
                .release(record_id, auth_user.user_id);
            return error_response(StatusCode::CONFLICT, "No agent has claimed this update yet");
        }
    };

    // 7. Verify the agent is still connected.
    if !state.service_connections.is_connected(&service_id).await {
        emit_interactive_session_audit(
            InteractiveAuditCtx {
                state: &state,
                actor: audit_actor,
            },
            record_id,
            Some(service_id),
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("service_not_connected"),
            None,
        );
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
    emit_interactive_session_audit(
        InteractiveAuditCtx {
            state: &state,
            actor: audit_actor,
        },
        record_id,
        Some(service_id),
        uptrakit_audit_log::AuditOutcome::Success,
        None,
        None,
    );

    ws.max_message_size(MAX_INTERACTIVE_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| {
            handle_interactive_session(socket, state, record_id, service_id, auth_user, audit_actor)
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
    audit_actor: InteractiveAuditActor,
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
        .all(state.db())
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
                            audit_actor,
                        )
                        .await;
                    }
                    Some(Ok(Message::Binary(data))) => {
                        // Treat raw binary as stdin data (no JSON envelope).
                        use base64::Engine;
                        let byte_count = data.len();
                        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
                        let payload = UpdateStdinDataPayload::new(update_history_id, encoded);
                        forward_interactive_stdin(
                            &state,
                            update_history_id,
                            service_id,
                            audit_actor,
                            payload,
                            "binary",
                            Some(byte_count),
                        )
                        .await;
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
    audit_actor: InteractiveAuditActor,
) {
    let client_msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!("invalid interactive WS message: {e}");
            return;
        }
    };

    let (payload, signal) = match client_msg {
        ClientMessage::Stdin { data } => {
            let byte_count = decode_text_stdin_byte_count(&data);
            let payload = UpdateStdinDataPayload::new(update_history_id, data);
            forward_interactive_stdin(
                state,
                update_history_id,
                service_id,
                audit_actor,
                payload,
                "text",
                byte_count,
            )
            .await;
            return;
        }
        ClientMessage::Signal { signal } => (
            UpdateStdinDataPayload::with_signal(update_history_id, signal),
            Some(signal),
        ),
    };

    let msg = ControllerMessage::UpdateStdinData(payload);
    let sent = state.service_connections.send(&service_id, msg).await;
    if !sent {
        tracing::warn!(
            %service_id,
            "failed to forward stdin/signal to agent (disconnected?)"
        );
    }

    if let Some(signal) = signal {
        let outcome = if sent {
            uptrakit_audit_log::AuditOutcome::Success
        } else {
            uptrakit_audit_log::AuditOutcome::Failed
        };
        let reason_code = if sent {
            None
        } else {
            Some("service_disconnected")
        };
        emit_interactive_signal_audit(
            InteractiveAuditCtx {
                state,
                actor: audit_actor,
            },
            update_history_id,
            service_id,
            signal,
            outcome,
            reason_code,
        );
    }
}

async fn forward_interactive_stdin(
    state: &AppState,
    update_history_id: Uuid,
    service_id: Uuid,
    audit_actor: InteractiveAuditActor,
    payload: UpdateStdinDataPayload,
    input_mode: &'static str,
    byte_count: Option<usize>,
) {
    let msg = ControllerMessage::UpdateStdinData(payload);
    let sent = state.service_connections.send(&service_id, msg).await;
    if !sent {
        tracing::warn!(
            %service_id,
            "failed to forward stdin to agent (disconnected?)"
        );
    }

    let outcome = if sent {
        uptrakit_audit_log::AuditOutcome::Success
    } else {
        uptrakit_audit_log::AuditOutcome::Failed
    };
    let reason_code = if sent {
        None
    } else {
        Some("service_disconnected")
    };
    emit_interactive_stdin_audit(
        InteractiveAuditCtx {
            state,
            actor: audit_actor,
        },
        update_history_id,
        service_id,
        input_mode,
        byte_count,
        outcome,
        reason_code,
    );
}

fn decode_text_stdin_byte_count(data: &str) -> Option<usize> {
    use base64::Engine;

    base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()
        .map(|bytes| bytes.len())
}

fn auth_method_for_token(token: &str) -> &'static str {
    if token.starts_with("upk_") {
        "api_token"
    } else {
        "jwt"
    }
}

fn classify_interactive_auth_failure(
    token: &str,
    failure: &AuthFailure,
) -> Option<(
    InteractiveAuditActor,
    uptrakit_audit_log::AuditOutcome,
    &'static str,
)> {
    if token.starts_with("upk_") {
        match failure {
            AuthFailure::InvalidApiToken => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::ApiToken),
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_or_revoked_api_token",
            )),
            AuthFailure::UserNotFound => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::ApiToken),
                uptrakit_audit_log::AuditOutcome::Denied,
                "user_not_found",
            )),
            AuthFailure::UserDeactivated => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::ApiToken),
                uptrakit_audit_log::AuditOutcome::Denied,
                "user_deactivated",
            )),
            AuthFailure::InternalError => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::ApiToken),
                uptrakit_audit_log::AuditOutcome::Failed,
                "api_token_authenticate_failed",
            )),
            _ => None,
        }
    } else {
        match failure {
            AuthFailure::InvalidOrExpiredToken => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::User),
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_or_expired_token",
            )),
            AuthFailure::InvalidTokenSubject => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::User),
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_token_subject",
            )),
            AuthFailure::TokenRevoked => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::User),
                uptrakit_audit_log::AuditOutcome::Denied,
                "token_revoked",
            )),
            AuthFailure::InvalidOidcSessionMissingProvider => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::Oidc),
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_oidc_session_missing_provider",
            )),
            AuthFailure::UserNotFound => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::User),
                uptrakit_audit_log::AuditOutcome::Denied,
                "user_not_found",
            )),
            AuthFailure::UserDeactivated => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::User),
                uptrakit_audit_log::AuditOutcome::Denied,
                "user_deactivated",
            )),
            AuthFailure::InternalError => Some((
                InteractiveAuditActor::anonymous(uptrakit_audit_log::AuditActorType::User),
                uptrakit_audit_log::AuditOutcome::Failed,
                "jwt_authenticate_failed",
            )),
            AuthFailure::InvalidApiToken => None,
        }
    }
}

fn emit_interactive_auth_failure_audit(
    state: &AppState,
    update_history_id: Uuid,
    actor: InteractiveAuditActor,
    auth_method: &'static str,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: &'static str,
) {
    emit_interactive_session_audit(
        InteractiveAuditCtx { state, actor },
        update_history_id,
        None,
        outcome,
        Some(reason_code),
        Some(serde_json::json!({
            "auth_method": auth_method,
        })),
    );
}

fn emit_interactive_session_audit(
    ctx: InteractiveAuditCtx<'_>,
    update_history_id: Uuid,
    service_id: Option<Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
    extra_details: Option<serde_json::Value>,
) {
    emit_interactive_control_audit(
        ctx,
        update_history_id,
        service_id,
        outcome,
        "session_establish",
        reason_code,
        extra_details,
    );
}

fn emit_interactive_stdin_audit(
    ctx: InteractiveAuditCtx<'_>,
    update_history_id: Uuid,
    service_id: Uuid,
    input_mode: &'static str,
    byte_count: Option<usize>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
) {
    let mut details =
        serde_json::Map::from_iter([("input_mode".to_string(), serde_json::json!(input_mode))]);
    if let Some(byte_count) = byte_count {
        details.insert("byte_count".to_string(), serde_json::json!(byte_count));
    }

    emit_interactive_control_audit(
        ctx,
        update_history_id,
        Some(service_id),
        outcome,
        "forward_stdin",
        reason_code,
        Some(serde_json::Value::Object(details)),
    );
}

fn emit_interactive_signal_audit(
    ctx: InteractiveAuditCtx<'_>,
    update_history_id: Uuid,
    service_id: Uuid,
    signal: i32,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
) {
    emit_interactive_control_audit(
        ctx,
        update_history_id,
        Some(service_id),
        outcome,
        "forward_signal",
        reason_code,
        Some(serde_json::json!({
            "signal": signal,
        })),
    );
}

fn emit_interactive_control_audit(
    ctx: InteractiveAuditCtx<'_>,
    update_history_id: Uuid,
    service_id: Option<Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    control_action: &'static str,
    reason_code: Option<&'static str>,
    extra_details: Option<serde_json::Value>,
) {
    let mut details = serde_json::Map::from_iter([(
        "control_action".to_string(),
        serde_json::json!(control_action),
    )]);
    if let Some(service_id) = service_id {
        details.insert("service_id".to_string(), serde_json::json!(service_id));
    }
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }
    if let Some(serde_json::Value::Object(extra)) = extra_details {
        details.extend(extra);
    }

    let entry = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_INTERACTIVE_CONTROL,
    )
    .tenant_scope(ctx.state.default_tenant_id)
    .actor(ctx.actor.actor_type, ctx.actor.actor_id)
    .target("update_history", update_history_id.to_string(), None)
    .outcome(outcome)
    .details(serde_json::Value::Object(details))
    .build();

    if let Ok(entry) = entry {
        ctx.state.audit_emitter.emit_best_effort(entry);
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

    #[cfg(feature = "db-sqlite")]
    use std::collections::BTreeSet;

    #[cfg(feature = "db-sqlite")]
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
    };
    #[cfg(feature = "db-sqlite")]
    use tokio::net::TcpListener;
    #[cfg(feature = "db-sqlite")]
    use tokio_tungstenite::tungstenite::Message;
    #[cfg(feature = "db-sqlite")]
    use uptrakit_internal_wire::ControllerMessage;
    #[cfg(feature = "db-sqlite")]
    use uptrakit_shared_db::entity::{audit_log, software_item, update_history};
    #[cfg(feature = "db-sqlite")]
    use uptrakit_shared_types::UpdateStatus;

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

    #[cfg(feature = "db-sqlite")]
    async fn latest_interactive_control_row(
        db: &sea_orm::DatabaseConnection,
        control_action: &str,
    ) -> audit_log::Model {
        for _ in 0..50 {
            let rows =
                audit_log::Entity::find()
                    .filter(audit_log::Column::ActionType.eq(
                        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_INTERACTIVE_CONTROL,
                    ))
                    .order_by_desc(audit_log::Column::OccurredAt)
                    .all(db)
                    .await
                    .expect("query interactive control audit rows");

            if let Some(row) = rows.into_iter().find(|row| {
                row.details_json
                    .as_ref()
                    .and_then(|details| details.get("control_action"))
                    .and_then(serde_json::Value::as_str)
                    == Some(control_action)
            }) {
                return row;
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected interactive control audit row for {control_action}");
    }

    #[cfg(feature = "db-sqlite")]
    async fn latest_interactive_control_row_matching(
        db: &sea_orm::DatabaseConnection,
        control_action: &str,
        predicate: impl Fn(&audit_log::Model) -> bool,
    ) -> audit_log::Model {
        for _ in 0..50 {
            let rows =
                audit_log::Entity::find()
                    .filter(audit_log::Column::ActionType.eq(
                        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_INTERACTIVE_CONTROL,
                    ))
                    .order_by_desc(audit_log::Column::OccurredAt)
                    .all(db)
                    .await
                    .expect("query interactive control audit rows");

            if let Some(row) = rows.into_iter().find(|row| {
                row.details_json
                    .as_ref()
                    .and_then(|details| details.get("control_action"))
                    .and_then(serde_json::Value::as_str)
                    == Some(control_action)
                    && predicate(row)
            }) {
                return row;
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected interactive control audit row for {control_action}");
    }

    #[cfg(feature = "db-sqlite")]
    async fn serve_app() -> (String, crate::test_harness::TestApp) {
        let app = crate::test_harness::TestApp::new().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let router = app.router.clone();

        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve");
        });

        (format!("ws://127.0.0.1:{}", addr.port()), app)
    }

    #[cfg(feature = "db-sqlite")]
    async fn insert_software_item(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
    ) -> software_item::Model {
        let now = time::OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            name: Set("Interactive Test Item".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software item")
    }

    #[cfg(feature = "db-sqlite")]
    async fn insert_update_history_row(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        status: UpdateStatus,
        execution_owner_service_id: Option<Uuid>,
    ) -> update_history::Model {
        let now = time::OffsetDateTime::now_utc();
        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(status),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(execution_owner_service_id),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(None),
            interactive: Set(true),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .expect("insert update history")
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_connect_success_writes_session_establish_audit() {
        let (base_url, app) = serve_app().await;
        let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
        let host = crate::test_harness::fixtures::insert_host(&app.db, app.tenant_id).await;
        let software_item = insert_software_item(&app.db, app.tenant_id).await;
        let service_id = Uuid::now_v7();
        let (_service_rx, _handle) = app
            .state
            .service_connections
            .register(service_id, BTreeSet::new(), None, None, None)
            .await;
        let update = insert_update_history_row(
            &app.db,
            app.tenant_id,
            host.id,
            software_item.id,
            UpdateStatus::InProgress,
            Some(service_id),
        )
        .await;
        app.state
            .broadcast
            .update_output_broadcaster
            .create_channel(update.id)
            .await;

        let (mut ws, response) = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{}/interactive?token={token}",
            update.id
        ))
        .await
        .expect("connect interactive websocket");

        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

        let row = latest_interactive_control_row(&app.db, "session_establish").await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(update.id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["control_action"],
            serde_json::json!("session_establish")
        );
        assert_eq!(details["service_id"], serde_json::json!(service_id));

        ws.close(None).await.expect("close websocket");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_connect_conflict_writes_denied_audit() {
        let (base_url, app) = serve_app().await;
        let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
        let host = crate::test_harness::fixtures::insert_host(&app.db, app.tenant_id).await;
        let software_item = insert_software_item(&app.db, app.tenant_id).await;
        let update = insert_update_history_row(
            &app.db,
            app.tenant_id,
            host.id,
            software_item.id,
            UpdateStatus::Completed,
            None,
        )
        .await;

        let err = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{}/interactive?token={token}",
            update.id
        ))
        .await
        .expect_err("connection should be rejected");

        let response = match err {
            tokio_tungstenite::tungstenite::Error::Http(response) => response,
            other => panic!("expected http error, got {other:?}"),
        };
        assert_eq!(response.status(), StatusCode::CONFLICT);

        let row = latest_interactive_control_row(&app.db, "session_establish").await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("update_not_in_progress")
        );
        assert_eq!(details["update_status"], serde_json::json!("completed"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_missing_token_writes_denied_session_establish_audit() {
        let (base_url, app) = serve_app().await;
        let update_id = Uuid::now_v7();

        let err = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{update_id}/interactive"
        ))
        .await
        .expect_err("connection should be rejected");

        let response = match err {
            tokio_tungstenite::tungstenite::Error::Http(response) => response,
            other => panic!("expected http error, got {other:?}"),
        };
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let row = latest_interactive_control_row(&app.db, "session_establish").await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, None);
        assert_eq!(
            row.target_id.as_deref(),
            Some(update_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["auth_method"], serde_json::json!("missing"));
        assert_eq!(details["reason_code"], serde_json::json!("missing_token"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_invalid_jwt_writes_denied_session_establish_audit() {
        let (base_url, app) = serve_app().await;
        let update_id = Uuid::now_v7();

        let err = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{update_id}/interactive?token=not-a-jwt"
        ))
        .await
        .expect_err("connection should be rejected");

        let response = match err {
            tokio_tungstenite::tungstenite::Error::Http(response) => response,
            other => panic!("expected http error, got {other:?}"),
        };
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let row = latest_interactive_control_row(&app.db, "session_establish").await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, None);
        let details = row.details_json.expect("details");
        assert_eq!(details["auth_method"], serde_json::json!("jwt"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_or_expired_token")
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_invalid_api_token_writes_denied_session_establish_audit() {
        let (base_url, app) = serve_app().await;
        let update_id = Uuid::now_v7();

        let err = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{update_id}/interactive?token=upk_invalid"
        ))
        .await
        .expect_err("connection should be rejected");

        let response = match err {
            tokio_tungstenite::tungstenite::Error::Http(response) => response,
            other => panic!("expected http error, got {other:?}"),
        };
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let row = latest_interactive_control_row(&app.db, "session_establish").await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::ApiToken.as_str()
        );
        assert_eq!(row.actor_id, None);
        let details = row.details_json.expect("details");
        assert_eq!(details["auth_method"], serde_json::json!("api_token"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_or_revoked_api_token")
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_update_history_lookup_failure_writes_failed_session_establish_audit() {
        let (base_url, app) = serve_app().await;
        let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
        let update_id = Uuid::now_v7();

        app.db
            .execute_unprepared("DROP TABLE update_history")
            .await
            .expect("drop update_history table");

        let err = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{update_id}/interactive?token={token}"
        ))
        .await
        .expect_err("connection should be rejected");

        let response = match err {
            tokio_tungstenite::tungstenite::Error::Http(response) => response,
            other => panic!("expected http error, got {other:?}"),
        };
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let row = latest_interactive_control_row_matching(&app.db, "session_establish", |row| {
            row.outcome == uptrakit_audit_log::AuditOutcome::Failed.as_str()
        })
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(update_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("update_history_lookup_failed")
        );
        assert_eq!(details["lookup_stage"], serde_json::json!("update_history"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_signal_forward_writes_success_audit_without_stdin_contents() {
        let (base_url, app) = serve_app().await;
        let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
        let host = crate::test_harness::fixtures::insert_host(&app.db, app.tenant_id).await;
        let software_item = insert_software_item(&app.db, app.tenant_id).await;
        let service_id = Uuid::now_v7();
        let (mut service_rx, _handle) = app
            .state
            .service_connections
            .register(service_id, BTreeSet::new(), None, None, None)
            .await;
        let update = insert_update_history_row(
            &app.db,
            app.tenant_id,
            host.id,
            software_item.id,
            UpdateStatus::InProgress,
            Some(service_id),
        )
        .await;
        app.state
            .broadcast
            .update_output_broadcaster
            .create_channel(update.id)
            .await;

        let (mut ws, _response) = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{}/interactive?token={token}",
            update.id
        ))
        .await
        .expect("connect interactive websocket");

        ws.send(Message::Text(
            serde_json::json!({
                "type": "signal",
                "signal": 2,
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send signal");

        let forwarded = tokio::time::timeout(std::time::Duration::from_secs(5), service_rx.recv())
            .await
            .expect("timely service message")
            .expect("forwarded message");
        match forwarded {
            ControllerMessage::UpdateStdinData(payload) => {
                assert_eq!(payload.update_history_id, update.id);
                assert_eq!(payload.signal, Some(2));
                assert_eq!(payload.data, "");
            }
            other => panic!("expected UpdateStdinData, got {other:?}"),
        }

        let row = latest_interactive_control_row_matching(&app.db, "forward_signal", |row| {
            row.outcome == uptrakit_audit_log::AuditOutcome::Success.as_str()
        })
        .await;
        let details = row.details_json.expect("details");
        assert_eq!(details["signal"], serde_json::json!(2));
        assert_eq!(details["service_id"], serde_json::json!(service_id));
        assert!(details.get("data").is_none());

        ws.close(None).await.expect("close websocket");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_text_stdin_forward_writes_success_audit_without_stdin_contents() {
        let (base_url, app) = serve_app().await;
        let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
        let host = crate::test_harness::fixtures::insert_host(&app.db, app.tenant_id).await;
        let software_item = insert_software_item(&app.db, app.tenant_id).await;
        let service_id = Uuid::now_v7();
        let (mut service_rx, _handle) = app
            .state
            .service_connections
            .register(service_id, BTreeSet::new(), None, None, None)
            .await;
        let update = insert_update_history_row(
            &app.db,
            app.tenant_id,
            host.id,
            software_item.id,
            UpdateStatus::InProgress,
            Some(service_id),
        )
        .await;
        app.state
            .broadcast
            .update_output_broadcaster
            .create_channel(update.id)
            .await;

        let (mut ws, _response) = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{}/interactive?token={token}",
            update.id
        ))
        .await
        .expect("connect interactive websocket");

        ws.send(Message::Text(
            serde_json::json!({
                "type": "stdin",
                "data": "aGVsbG8=",
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send stdin");

        let forwarded = tokio::time::timeout(std::time::Duration::from_secs(5), service_rx.recv())
            .await
            .expect("timely service message")
            .expect("forwarded message");
        match forwarded {
            ControllerMessage::UpdateStdinData(payload) => {
                assert_eq!(payload.update_history_id, update.id);
                assert_eq!(payload.signal, None);
                assert_eq!(payload.data, "aGVsbG8=");
            }
            other => panic!("expected UpdateStdinData, got {other:?}"),
        }

        let row = latest_interactive_control_row_matching(&app.db, "forward_stdin", |row| {
            row.outcome == uptrakit_audit_log::AuditOutcome::Success.as_str()
        })
        .await;
        let details = row.details_json.expect("details");
        assert_eq!(details["input_mode"], serde_json::json!("text"));
        assert_eq!(details["byte_count"], serde_json::json!(5));
        assert_eq!(details["service_id"], serde_json::json!(service_id));
        assert!(details.get("data").is_none());

        ws.close(None).await.expect("close websocket");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_text_stdin_forward_failure_writes_failed_audit() {
        let (base_url, app) = serve_app().await;
        let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
        let host = crate::test_harness::fixtures::insert_host(&app.db, app.tenant_id).await;
        let software_item = insert_software_item(&app.db, app.tenant_id).await;
        let service_id = Uuid::now_v7();
        let (service_rx, _handle) = app
            .state
            .service_connections
            .register(service_id, BTreeSet::new(), None, None, None)
            .await;
        let update = insert_update_history_row(
            &app.db,
            app.tenant_id,
            host.id,
            software_item.id,
            UpdateStatus::InProgress,
            Some(service_id),
        )
        .await;
        app.state
            .broadcast
            .update_output_broadcaster
            .create_channel(update.id)
            .await;

        let (mut ws, _response) = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{}/interactive?token={token}",
            update.id
        ))
        .await
        .expect("connect interactive websocket");

        drop(service_rx);

        ws.send(Message::Text(
            serde_json::json!({
                "type": "stdin",
                "data": "aGVsbG8=",
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send stdin");

        let row = latest_interactive_control_row_matching(&app.db, "forward_stdin", |row| {
            row.outcome == uptrakit_audit_log::AuditOutcome::Failed.as_str()
        })
        .await;
        let details = row.details_json.expect("details");
        assert_eq!(details["input_mode"], serde_json::json!("text"));
        assert_eq!(details["byte_count"], serde_json::json!(5));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("service_disconnected")
        );

        ws.close(None).await.expect("close websocket");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_binary_stdin_forward_writes_success_audit_without_stdin_contents() {
        let (base_url, app) = serve_app().await;
        let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
        let host = crate::test_harness::fixtures::insert_host(&app.db, app.tenant_id).await;
        let software_item = insert_software_item(&app.db, app.tenant_id).await;
        let service_id = Uuid::now_v7();
        let (mut service_rx, _handle) = app
            .state
            .service_connections
            .register(service_id, BTreeSet::new(), None, None, None)
            .await;
        let update = insert_update_history_row(
            &app.db,
            app.tenant_id,
            host.id,
            software_item.id,
            UpdateStatus::InProgress,
            Some(service_id),
        )
        .await;
        app.state
            .broadcast
            .update_output_broadcaster
            .create_channel(update.id)
            .await;

        let (mut ws, _response) = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{}/interactive?token={token}",
            update.id
        ))
        .await
        .expect("connect interactive websocket");

        ws.send(Message::Binary(vec![0, 1, 2, 3].into()))
            .await
            .expect("send binary stdin");

        let forwarded = tokio::time::timeout(std::time::Duration::from_secs(5), service_rx.recv())
            .await
            .expect("timely service message")
            .expect("forwarded message");
        match forwarded {
            ControllerMessage::UpdateStdinData(payload) => {
                assert_eq!(payload.update_history_id, update.id);
                assert_eq!(payload.signal, None);
                assert_eq!(payload.data, "AAECAw==");
            }
            other => panic!("expected UpdateStdinData, got {other:?}"),
        }

        let row = latest_interactive_control_row_matching(&app.db, "forward_stdin", |row| {
            row.outcome == uptrakit_audit_log::AuditOutcome::Success.as_str()
        })
        .await;
        let details = row.details_json.expect("details");
        assert_eq!(details["input_mode"], serde_json::json!("binary"));
        assert_eq!(details["byte_count"], serde_json::json!(4));
        assert_eq!(details["service_id"], serde_json::json!(service_id));
        assert!(details.get("data").is_none());

        ws.close(None).await.expect("close websocket");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_binary_stdin_forward_failure_writes_failed_audit() {
        let (base_url, app) = serve_app().await;
        let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
        let host = crate::test_harness::fixtures::insert_host(&app.db, app.tenant_id).await;
        let software_item = insert_software_item(&app.db, app.tenant_id).await;
        let service_id = Uuid::now_v7();
        let (service_rx, _handle) = app
            .state
            .service_connections
            .register(service_id, BTreeSet::new(), None, None, None)
            .await;
        let update = insert_update_history_row(
            &app.db,
            app.tenant_id,
            host.id,
            software_item.id,
            UpdateStatus::InProgress,
            Some(service_id),
        )
        .await;
        app.state
            .broadcast
            .update_output_broadcaster
            .create_channel(update.id)
            .await;

        let (mut ws, _response) = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{}/interactive?token={token}",
            update.id
        ))
        .await
        .expect("connect interactive websocket");

        drop(service_rx);

        ws.send(Message::Binary(vec![0, 1, 2, 3].into()))
            .await
            .expect("send binary stdin");

        let row = latest_interactive_control_row_matching(&app.db, "forward_stdin", |row| {
            row.outcome == uptrakit_audit_log::AuditOutcome::Failed.as_str()
        })
        .await;
        let details = row.details_json.expect("details");
        assert_eq!(details["input_mode"], serde_json::json!("binary"));
        assert_eq!(details["byte_count"], serde_json::json!(4));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("service_disconnected")
        );

        ws.close(None).await.expect("close websocket");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn interactive_ws_signal_forward_failure_writes_failed_audit() {
        let (base_url, app) = serve_app().await;
        let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
        let host = crate::test_harness::fixtures::insert_host(&app.db, app.tenant_id).await;
        let software_item = insert_software_item(&app.db, app.tenant_id).await;
        let service_id = Uuid::now_v7();
        let (service_rx, _handle) = app
            .state
            .service_connections
            .register(service_id, BTreeSet::new(), None, None, None)
            .await;
        let update = insert_update_history_row(
            &app.db,
            app.tenant_id,
            host.id,
            software_item.id,
            UpdateStatus::InProgress,
            Some(service_id),
        )
        .await;
        app.state
            .broadcast
            .update_output_broadcaster
            .create_channel(update.id)
            .await;

        let (mut ws, _response) = tokio_tungstenite::connect_async(format!(
            "{base_url}/api/v1/update-history/{}/interactive?token={token}",
            update.id
        ))
        .await
        .expect("connect interactive websocket");

        drop(service_rx);

        ws.send(Message::Text(
            serde_json::json!({
                "type": "signal",
                "signal": 15,
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send signal");

        let row = latest_interactive_control_row_matching(&app.db, "forward_signal", |row| {
            row.outcome == uptrakit_audit_log::AuditOutcome::Failed.as_str()
        })
        .await;
        let details = row.details_json.expect("details");
        assert_eq!(details["signal"], serde_json::json!(15));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("service_disconnected")
        );

        ws.close(None).await.expect("close websocket");
    }
}
