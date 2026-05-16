//! Test-utilities endpoints — only compiled with the `test-utils` Cargo feature.
//!
//! All handlers check `UPTRAKIT_TEST_UTILS_ENABLED=true` at runtime before acting.
//! If the env var is absent or not `"true"`, every handler returns 404, making
//! the endpoints invisible to clients not in a test context.
#![cfg(feature = "test-utils")]

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uptrakit_wire::{ControllerMessage, RequestCertRenewalPayload};
use uuid::Uuid;

use crate::AppState;

fn test_utils_allowed() -> bool {
    std::env::var("UPTRAKIT_TEST_UTILS_ENABLED").as_deref() == Ok("true")
}

/// Send `RequestCertRenewal` to a specific connected service.
///
/// Returns 200 if the message was sent, 404 if the service is not connected
/// or if `UPTRAKIT_TEST_UTILS_ENABLED` is not `"true"`.
pub(crate) async fn request_service_renewal(
    State(state): State<Arc<AppState>>,
    Path(service_id): Path<Uuid>,
) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let msg = ControllerMessage::RequestCertRenewal(RequestCertRenewalPayload {
        reason: "test-utils: forced renewal".to_string(),
    });
    if state.service_connections.send(&service_id, msg).await {
        StatusCode::OK.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Close the WebSocket for a specific service, triggering its reconnect loop.
///
/// Returns 200 unconditionally (disconnect is a no-op if already disconnected).
/// Returns 404 if `UPTRAKIT_TEST_UTILS_ENABLED` is not `"true"`.
pub(crate) async fn disconnect_service(
    State(state): State<Arc<AppState>>,
    Path(service_id): Path<Uuid>,
) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    state
        .service_connections
        .force_disconnect(&service_id)
        .await;
    StatusCode::OK.into_response()
}
