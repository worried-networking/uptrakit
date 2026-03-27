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
use crate::api_error::ApiError;
use crate::auth::api_token::ApiTokenService;
use crate::auth::device_flow::DeviceFlowStatus;
use crate::auth::token::hash_token;
use crate::device_flow_broadcaster::DeviceFlowEvent;
use crate::error_response::error_response;
use crate::extract::ApiTokenSvc;
use crate::middleware::permission::CanViewServices;

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
#[tracing::instrument(skip_all)]
pub async fn device_auth_start(
    State(state): State<Arc<AppState>>,
    external_base_url: Option<axum::Extension<crate::extract::ExternalBaseUrl>>,
    headers: HeaderMap,
    Json(req): Json<DeviceAuthStartRequest>,
) -> Response {
    let (device_code, user_code) = match state.auth.device_flow_store.create(req.client_name).await
    {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("Failed to create device flow: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Create a broadcaster channel for SSE subscribers.
    let device_code_hash = hash_token(&device_code);
    state
        .broadcast
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
#[tracing::instrument(skip_all)]
pub async fn device_auth_poll(
    State(state): State<Arc<AppState>>,
    api_token_svc: ApiTokenSvc,
    Json(req): Json<DeviceAuthPollRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let status = state
        .auth
        .device_flow_store
        .get_status(&req.device_code)
        .await?;

    match status {
        DeviceFlowStatus::Pending => Ok((
            StatusCode::OK,
            Json(DeviceAuthPollResponse {
                status: DeviceAuthStatus::Pending,
                token: None,
                token_name: None,
            }),
        )
            .into_response()),
        DeviceFlowStatus::Expired => Ok((
            StatusCode::OK,
            Json(DeviceAuthPollResponse {
                status: DeviceAuthStatus::Expired,
                token: None,
                token_name: None,
            }),
        )
            .into_response()),
        DeviceFlowStatus::Authorized { .. } => {
            // Consume the flow (one-time use)
            let (user_id, client_name) = state
                .auth
                .device_flow_store
                .consume(&req.device_code)
                .await?;

            let token_name = client_name.unwrap_or_else(|| "cli-device-auth".into());

            // Create an API token for the user
            match api_token_svc.create_token(user_id, &token_name).await {
                Ok(created) => Ok((
                    StatusCode::OK,
                    Json(DeviceAuthPollResponse {
                        status: DeviceAuthStatus::Authorized,
                        token: Some(SecretString::new(created.plaintext_token)),
                        token_name: Some(token_name),
                    }),
                )
                    .into_response()),
                Err(e) => {
                    tracing::error!("Failed to create API token for device flow: {e:?}");
                    Ok(error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    ))
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
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn device_auth_approve(
    State(state): State<Arc<AppState>>,
    CanViewServices(auth_user): CanViewServices,
    Json(req): Json<DeviceAuthApproveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let normalized = req.user_code.replace('-', "").to_uppercase();

    state
        .auth
        .device_flow_store
        .approve(&normalized, auth_user.user_id)
        .await?;

    // Notify SSE subscribers that the flow was approved.
    if let Ok(hash) = state
        .auth
        .device_flow_store
        .get_device_code_hash_by_user_code(&normalized)
        .await
    {
        state
            .broadcast
            .device_flow_broadcaster
            .notify_status_changed(&hash)
            .await;
    }

    Ok((
        StatusCode::OK,
        Json(DeviceAuthApproveResponse {
            message: "Device authorized".into(),
        }),
    )
        .into_response())
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
#[tracing::instrument(skip_all)]
pub async fn device_auth_stream(
    State(state): State<Arc<AppState>>,
    api_token_svc: ApiTokenSvc,
    Query(query): Query<DeviceAuthStreamQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let device_code = query.device_code;
    let device_code_hash = hash_token(&device_code);
    let shutdown_token = state.shutdown_token.clone();

    // Subscribe to the broadcaster for live notifications.
    let broadcast_rx = state
        .broadcast
        .device_flow_broadcaster
        .subscribe(&device_code_hash)
        .await;

    // Check current status in the DB.
    let current_status = state
        .auth
        .device_flow_store
        .get_status(&device_code)
        .await?;

    let stream = async_stream::stream! {
        match current_status {
            DeviceFlowStatus::Authorized { .. } => {
                // Already authorized — consume and yield token immediately.
                if let Some(event) = consume_and_yield(&state, &api_token_svc, &device_code).await {
                    yield Ok::<_, Infallible>(event);
                }
                state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                return;
            }
            DeviceFlowStatus::Expired => {
                let payload = DeviceAuthExpiredSse {
                    message: "Device flow expired".to_string(),
                };
                if let Ok(json) = serde_json::to_string(&payload) {
                    yield Ok::<_, Infallible>(Event::default().event("expired").data(json));
                }
                state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
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
                                if let Some(event) = consume_and_yield(&state, &api_token_svc, &device_code).await {
                                    yield Ok::<_, Infallible>(event);
                                }
                                state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                                return;
                            }
                            Ok(DeviceFlowEvent::Expired) => {
                                let payload = DeviceAuthExpiredSse {
                                    message: "Device flow expired".to_string(),
                                };
                                if let Ok(json) = serde_json::to_string(&payload) {
                                    yield Ok::<_, Infallible>(Event::default().event("expired").data(json));
                                }
                                state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
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
                        state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                        return;
                    }
                    _ = shutdown_token.cancelled() => {
                        state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                        return;
                    }
                }
            }
        }
    };

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default().interval(std::time::Duration::from_secs(15)))
        .into_response())
}

/// Consume the device flow and return an SSE `authorized` event with the token.
async fn consume_and_yield(
    state: &AppState,
    api_svc: &ApiTokenService,
    device_code: &str,
) -> Option<Event> {
    let (user_id, client_name) = match state.auth.device_flow_store.consume(device_code).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("Device flow consume failed during SSE: {e}");
            return None;
        }
    };

    let token_name = client_name.unwrap_or_else(|| "cli-device-auth".into());
    match api_svc.create_token(user_id, &token_name).await {
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
