use crate::AppState;
use crate::auth::api_token::ApiTokenService;
use crate::auth::device_flow::{DeviceFlowError, DeviceFlowStatus};
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use axum::http::HeaderMap;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_shared_db::entity::pending_device_flow::DeviceAuthStatus;

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
            let service = ApiTokenService::new(state.db.clone());
            match service.create_token(user_id, &token_name).await {
                Ok(created) => (
                    StatusCode::OK,
                    Json(DeviceAuthPollResponse {
                        status: DeviceAuthStatus::Authorized,
                        token: Some(created.plaintext_token),
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
    security(("bearer_token" = []))
)]
pub async fn device_auth_approve(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Json(req): Json<DeviceAuthApproveRequest>,
) -> Response {
    if !auth_user.has_permission(Permission::ViewAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

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
