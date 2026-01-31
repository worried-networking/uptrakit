use crate::AppState;
use crate::auth::api_token::ApiTokenService;
use crate::auth::device_flow::{DeviceFlowError, DeviceFlowStatus, MIN_POLL_INTERVAL_SECONDS};
use crate::middleware::require_auth::AuthenticatedUser;
use axum::http::HeaderMap;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct DeviceAuthStartRequest {
    pub client_name: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DeviceAuthStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Deserialize, ToSchema)]
pub struct DeviceAuthPollRequest {
    pub device_code: String,
}

#[derive(Serialize, ToSchema)]
pub struct DeviceAuthPollResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_name: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct DeviceAuthApproveRequest {
    pub user_code: String,
}

#[derive(Serialize, ToSchema)]
pub struct DeviceAuthApproveResponse {
    pub message: String,
}

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
    headers: HeaderMap,
    Json(req): Json<DeviceAuthStartRequest>,
) -> Response {
    let (device_code, user_code) = match state.device_flow_store.create(req.client_name).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("Failed to create device flow: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Derive verification URL from Host or Origin header
    let host = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
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
        interval: MIN_POLL_INTERVAL_SECONDS as u64,
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
    // Check rate limiting
    match state
        .device_flow_store
        .is_rate_limited(&req.device_code)
        .await
    {
        Ok(true) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(DeviceAuthPollResponse {
                    status: "slow_down".into(),
                    token: None,
                    token_name: None,
                }),
            )
                .into_response();
        }
        Ok(false) => {}
        Err(e) => match e.current_context() {
            DeviceFlowError::NotFound => {
                return (StatusCode::NOT_FOUND, "Device flow not found or expired\n")
                    .into_response();
            }
            _ => {
                tracing::error!("Device flow rate limit check failed: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        },
    }

    // Record poll timestamp
    if let Err(e) = state.device_flow_store.record_poll(&req.device_code).await {
        tracing::error!("Failed to record poll: {e}");
    }

    // Check status
    let status = match state.device_flow_store.get_status(&req.device_code).await {
        Ok(s) => s,
        Err(e) => match e.current_context() {
            DeviceFlowError::NotFound => {
                return (StatusCode::NOT_FOUND, "Device flow not found or expired\n")
                    .into_response();
            }
            _ => {
                tracing::error!("Device flow status check failed: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        },
    };

    match status {
        DeviceFlowStatus::Pending => (
            StatusCode::OK,
            Json(DeviceAuthPollResponse {
                status: "pending".into(),
                token: None,
                token_name: None,
            }),
        )
            .into_response(),
        DeviceFlowStatus::Expired => (
            StatusCode::OK,
            Json(DeviceAuthPollResponse {
                status: "expired".into(),
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
                            return (StatusCode::NOT_FOUND, "Device flow not found or expired\n")
                                .into_response();
                        }
                        _ => {
                            tracing::error!("Device flow consume failed: {e}");
                            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                        }
                    },
                };

            let token_name = client_name.unwrap_or_else(|| "cli-device-auth".into());

            // Create an API token for the user
            let service = ApiTokenService::new(state.db.clone());
            match service.create_token(user_id, &token_name).await {
                Ok(created) => (
                    StatusCode::OK,
                    Json(DeviceAuthPollResponse {
                        status: "authorized".into(),
                        token: Some(created.plaintext_token),
                        token_name: Some(token_name),
                    }),
                )
                    .into_response(),
                Err(e) => {
                    tracing::error!("Failed to create API token for device flow: {e:?}");
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
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
        (status = 404, description = "Device flow not found"),
        (status = 409, description = "Already authorized")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
pub async fn device_auth_approve(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Json(req): Json<DeviceAuthApproveRequest>,
) -> Response {
    let normalized = req.user_code.replace('-', "").to_uppercase();

    match state
        .device_flow_store
        .approve(&normalized, auth_user.user_id)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(DeviceAuthApproveResponse {
                message: "Device authorized".into(),
            }),
        )
            .into_response(),
        Err(e) => match e.current_context() {
            DeviceFlowError::NotFound => {
                (StatusCode::NOT_FOUND, "Device flow not found or expired\n").into_response()
            }
            DeviceFlowError::AlreadyAuthorized => {
                (StatusCode::CONFLICT, "Device flow already authorized\n").into_response()
            }
            _ => {
                tracing::error!("Device flow approve failed: {e}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
    }
}
