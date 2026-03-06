use std::convert::Infallible;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use uptrakit_shared_types::DeviceAuthStatus;
use uptrakit_web_api_types::SecretString;
use uptrakit_web_api_types::device_auth::{DeviceAuthAuthorizedSse, DeviceAuthExpiredSse};

use crate::AppState;
use crate::auth::api_token::ApiTokenService;
use crate::auth::device_flow::{DeviceFlowError, DeviceFlowStatus};
use crate::auth::token::hash_token;
use crate::device_flow_broadcaster::DeviceFlowEvent;
use crate::error_response::error_response;
use crate::middleware::permission::CanViewAgents;

pub use uptrakit_web_api_types::device_auth::{
    DeviceAuthApproveRequest, DeviceAuthApproveResponse, DeviceAuthPollRequest,
    DeviceAuthPollResponse, DeviceAuthStartRequest, DeviceAuthStartResponse,
};

/// Start a device authorization flow
#[utoipa::path(
    post,
    path = "/api/v1/auth/device",
    request_body = DeviceAuthStartRequest,
    responses(
        (status = 200, description = "Device flow started", body = DeviceAuthStartResponse),
        (status = 500, description = "Internal error")
    ),
    tag = "Authentication"
)]
pub async fn device_auth_start(
    State(state): State<Arc<AppState>>,
    external_base_url: Option<axum::Extension<crate::extract::ExternalBaseUrl>>,
    headers: HeaderMap,
    Json(req): Json<DeviceAuthStartRequest>,
) -> Response {
    let (device_code, user_code) = match state.device_flow_store.create(req.client_name).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("Failed to create device flow: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Create a broadcaster channel for SSE subscribers.
    let device_code_hash = hash_token(&device_code);
    state
        .device_flow_broadcaster
        .create_channel(&device_code_hash)
        .await;

    // Derive verification URL: prefer ExternalBaseUrl, then Origin, then Host
    let host = external_base_url
        .map(|axum::Extension(u)| u.0)
        .or_else(|| {
            headers
                .get("origin")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            headers
                .get("host")
                .and_then(|v| v.to_str().ok())
                .map(|h| format!("https://{h}"))
        })
        .unwrap_or_default();

    let verification_url = format!("{host}/device?code={user_code}");

    let response = DeviceAuthStartResponse {
        device_code,
        user_code,
        verification_url,
        expires_in: 600,
        interval: 5,
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Poll for device authorization status
#[utoipa::path(
    post,
    path = "/api/v1/auth/device/poll",
    request_body = DeviceAuthPollRequest,
    responses(
        (status = 200, description = "Device flow status", body = DeviceAuthPollResponse),
        (status = 404, description = "Device flow not found"),
        (status = 429, description = "Polling too fast")
    ),
    tag = "Authentication"
)]
pub async fn device_auth_poll(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeviceAuthPollRequest>,
) -> Response {
    // Check status
    let status = match state.device_flow_store.get_status(&req.device_code).await {
        Ok(s) => s,
        Err(e) => match e.current_context() {
            DeviceFlowError::NotFound => {
                return error_response(StatusCode::NOT_FOUND, "Device flow not found or expired");
            }
            _ => {
                tracing::error!("Device flow status check failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
    };

    match status {
        DeviceFlowStatus::Pending => (
            StatusCode::OK,
            Json(DeviceAuthPollResponse {
                status: DeviceAuthStatus::Pending,
                token: None,
                token_name: None,
            }),
        )
            .into_response(),
        DeviceFlowStatus::Expired => (
            StatusCode::OK,
            Json(DeviceAuthPollResponse {
                status: DeviceAuthStatus::Expired,
                token: None,
                token_name: None,
            }),
        )
            .into_response(),
        DeviceFlowStatus::Authorized { .. } => {
            // Consume the flow (one-time use)
            let (user_id, client_name) =
                match state.device_flow_store.consume(&req.device_code).await {
                    Ok(result) => result,
                    Err(e) => match e.current_context() {
                        DeviceFlowError::NotFound => {
                            return error_response(
                                StatusCode::NOT_FOUND,
                                "Device flow not found or expired",
                            );
                        }
                        _ => {
                            tracing::error!("Device flow consume failed: {e}");
                            return error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal server error",
                            );
                        }
                    },
                };

            let token_name = client_name.unwrap_or_else(|| "cli-device-auth".into());

            // Create an API token for the user
            let service = ApiTokenService::new(state.db().clone());
            match service.create_token(user_id, &token_name).await {
                Ok(created) => (
                    StatusCode::OK,
                    Json(DeviceAuthPollResponse {
                        status: DeviceAuthStatus::Authorized,
                        token: Some(SecretString::new(created.plaintext_token)),
                        token_name: Some(token_name),
                    }),
                )
                    .into_response(),
                Err(e) => {
                    tracing::error!("Failed to create API token for device flow: {e:?}");
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
            }
        }
    }
}

/// Approve a device authorization (authenticated)
#[utoipa::path(
    post,
    path = "/api/v1/auth/device/approve",
    request_body = DeviceAuthApproveRequest,
    responses(
        (status = 200, description = "Device authorized", body = DeviceAuthApproveResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Device flow not found"),
        (status = 409, description = "Already authorized")
    ),
    tag = "Authentication",
    extensions(("x-required-permission" = json!("view_agents"))),
    security(("bearer_token" = []))
)]
pub async fn device_auth_approve(
    State(state): State<Arc<AppState>>,
    CanViewAgents(auth_user): CanViewAgents,
    Json(req): Json<DeviceAuthApproveRequest>,
) -> Response {
    let normalized = req.user_code.replace('-', "").to_uppercase();

    match state
        .device_flow_store
        .approve(&normalized, auth_user.user_id)
        .await
    {
        Ok(()) => {
            // Notify SSE subscribers that the flow was approved.
            if let Ok(hash) = state
                .device_flow_store
                .get_device_code_hash_by_user_code(&normalized)
                .await
            {
                state
                    .device_flow_broadcaster
                    .notify_status_changed(&hash)
                    .await;
            }
            (
                StatusCode::OK,
                Json(DeviceAuthApproveResponse {
                    message: "Device authorized".into(),
                }),
            )
                .into_response()
        }
        Err(e) => match e.current_context() {
            DeviceFlowError::NotFound => {
                error_response(StatusCode::NOT_FOUND, "Device flow not found or expired")
            }
            DeviceFlowError::AlreadyAuthorized => {
                error_response(StatusCode::CONFLICT, "Device flow already authorized")
            }
            _ => {
                tracing::error!("Device flow approve failed: {e}");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        },
    }
}

/// Query parameters for the device auth SSE stream.
#[derive(Debug, Deserialize)]
pub struct DeviceAuthStreamQuery {
    pub device_code: String,
}

/// SSE stream for device authorization status.
///
/// Unauthenticated endpoint (same as poll). The CLI connects here instead of
/// polling and receives the token immediately when the user approves in the
/// browser.
///
/// # Events
///
/// - `authorized` — contains the API token and token name.
/// - `expired` — the device flow expired before approval.
pub async fn device_auth_stream(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DeviceAuthStreamQuery>,
) -> Response {
    let device_code = query.device_code;
    let device_code_hash = hash_token(&device_code);
    let shutdown_token = state.shutdown_token.clone();

    // Subscribe to the broadcaster for live notifications.
    let broadcast_rx = state
        .device_flow_broadcaster
        .subscribe(&device_code_hash)
        .await;

    // Check current status in the DB.
    let current_status = match state.device_flow_store.get_status(&device_code).await {
        Ok(s) => s,
        Err(e) => match e.current_context() {
            DeviceFlowError::NotFound => {
                return error_response(StatusCode::NOT_FOUND, "Device flow not found or expired");
            }
            _ => {
                tracing::error!("Device flow status check failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
    };

    let stream = async_stream::stream! {
        match current_status {
            DeviceFlowStatus::Authorized { .. } => {
                // Already authorized — consume and yield token immediately.
                if let Some(event) = consume_and_yield(&state, &device_code).await {
                    yield Ok::<_, Infallible>(event);
                }
                state.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                return;
            }
            DeviceFlowStatus::Expired => {
                let payload = DeviceAuthExpiredSse {
                    message: "Device flow expired".to_string(),
                };
                if let Ok(json) = serde_json::to_string(&payload) {
                    yield Ok::<_, Infallible>(Event::default().event("expired").data(json));
                }
                state.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                return;
            }
            DeviceFlowStatus::Pending => {
                // Fall through to wait on broadcast channel.
            }
        }

        // Wait for broadcast events.
        if let Some(mut rx) = broadcast_rx {
            let timeout = tokio::time::sleep(std::time::Duration::from_secs(600));
            tokio::pin!(timeout);

            loop {
                tokio::select! {
                    ev = rx.recv() => {
                        match ev {
                            Ok(DeviceFlowEvent::StatusChanged) => {
                                if let Some(event) = consume_and_yield(&state, &device_code).await {
                                    yield Ok::<_, Infallible>(event);
                                }
                                state.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                                return;
                            }
                            Ok(DeviceFlowEvent::Expired) => {
                                let payload = DeviceAuthExpiredSse {
                                    message: "Device flow expired".to_string(),
                                };
                                if let Ok(json) = serde_json::to_string(&payload) {
                                    yield Ok::<_, Infallible>(Event::default().event("expired").data(json));
                                }
                                state.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                                return;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                return;
                            }
                        }
                    }
                    _ = &mut timeout => {
                        let payload = DeviceAuthExpiredSse {
                            message: "Device flow expired".to_string(),
                        };
                        if let Ok(json) = serde_json::to_string(&payload) {
                            yield Ok::<_, Infallible>(Event::default().event("expired").data(json));
                        }
                        state.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                        return;
                    }
                    _ = shutdown_token.cancelled() => {
                        state.device_flow_broadcaster.remove_channel(&device_code_hash).await;
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

/// Consume the device flow and return an SSE `authorized` event with the token.
async fn consume_and_yield(state: &AppState, device_code: &str) -> Option<Event> {
    let (user_id, client_name) = match state.device_flow_store.consume(device_code).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("Device flow consume failed during SSE: {e}");
            return None;
        }
    };

    let token_name = client_name.unwrap_or_else(|| "cli-device-auth".into());
    let service = ApiTokenService::new(state.db().clone());
    match service.create_token(user_id, &token_name).await {
        Ok(created) => {
            let payload = DeviceAuthAuthorizedSse {
                token: SecretString::new(created.plaintext_token),
                token_name,
            };
            serde_json::to_string(&payload)
                .ok()
                .map(|json| Event::default().event("authorized").data(json))
        }
        Err(e) => {
            tracing::error!("Failed to create API token during SSE: {e:?}");
            None
        }
    }
}
