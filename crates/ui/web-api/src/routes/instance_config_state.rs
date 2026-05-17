use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;

use uptrakit_config_reload::CoordinatorState;
use uptrakit_web_api_types::instance_config_state::{
    ConfigStateResponse, DegradedInfoView, FileStateView, LastReloadView,
};

use crate::AppState;
use crate::error_response::error_response_with_code;
use crate::middleware::permission::{CanManageInstanceConfigState, CanViewInstanceConfigState};

/// Get the current config reload coordinator state.
#[utoipa::path(
    get,
    path = "/api/v1/instance/config-state",
    tag = "System",
    responses(
        (status = 200, description = "Config state", body = ConfigStateResponse),
        (status = 403, description = "Not authorized")
    ),
    extensions(("x-required-permission" = json!("view_instance_config_state"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_config_state(
    State(state): State<Arc<AppState>>,
    _perm: CanViewInstanceConfigState,
) -> Response {
    let coordinator_state = state.coordinator_handle.state();
    let file_state = state.config_file_state.borrow().clone();
    let last_reload_info = state.last_reload.borrow().clone();
    let recent = state.recent_reload_events.borrow().clone();

    let file = FileStateView::new(
        file_state.path,
        file_state.digest,
        file_state.loaded_at,
        file_state.pending_digest,
        file_state.pending_detected_at,
    );

    let last_reload = last_reload_info
        .map(|r| LastReloadView::new(r.completed_at, r.sections, r.per_subsystem_ms));

    let sections = render_sections(&state);

    let (coordinator_state_str, degraded) = match coordinator_state {
        CoordinatorState::Idle => ("idle".to_string(), None),
        CoordinatorState::Reloading => ("reloading".to_string(), None),
        CoordinatorState::Degraded(info) => (
            "degraded".to_string(),
            Some(DegradedInfoView::new(
                info.since,
                info.failed_subsystems.clone(),
                info.reason.clone(),
            )),
        ),
        _ => {
            tracing::warn!("unrecognised CoordinatorState variant; treating as unknown");
            ("unknown".to_string(), None)
        }
    };

    let resp = ConfigStateResponse::new(
        coordinator_state_str,
        degraded,
        file,
        last_reload,
        sections,
        recent,
    );

    (StatusCode::OK, Json(resp)).into_response()
}

/// Clear the coordinator degraded state.
///
/// The operator asserts the underlying issue has been resolved. Returns the
/// updated config state after clearing.
#[utoipa::path(
    post,
    path = "/api/v1/instance/config-reload/clear-degraded",
    tag = "System",
    responses(
        (status = 200, description = "Cleared; returns updated config state", body = ConfigStateResponse),
        (status = 403, description = "Not authorized"),
        (status = 500, description = "Failed to clear")
    ),
    extensions(("x-required-permission" = json!("manage_instance_config_state"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn clear_coordinator_degraded(
    State(state): State<Arc<AppState>>,
    manage: CanManageInstanceConfigState,
) -> Response {
    if let Err(e) = state.coordinator_handle.clear_degraded().await {
        tracing::error!(error = %e, "clear_degraded failed");
        return error_response_with_code(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("clear_degraded failed: {e}"),
            "config_reload.clear_degraded_failed",
        );
    }
    // ManageInstanceConfigState implies ViewInstanceConfigState; reuse the inner
    // AuthenticatedUser to avoid repeating the auth/permission check.
    let view = CanViewInstanceConfigState::new(manage.0);
    get_config_state(State(state), view).await
}

fn render_sections(state: &AppState) -> serde_json::Value {
    serde_json::json!({
        "db": { "url": "<redacted>" },
        "network": render_network(state),
        "tls": render_tls(state),
    })
}

fn render_network(state: &AppState) -> serde_json::Value {
    let cfg = state.network_config_rx.borrow();
    serde_json::json!({
        "https_addr": cfg.https.addr,
        "pki_addr": cfg.pki_addr,
    })
}

fn render_tls(state: &AppState) -> serde_json::Value {
    let cfg = state.tls_config_rx.borrow();
    serde_json::json!({
        "trust_domain": cfg.trust_domain,
        "cert_path": cfg.cert_path,
    })
}
