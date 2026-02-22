use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use http::StatusCode;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageGlobalSettings;

pub use uptrakit_web_api_types::settings_ca::RotateCaResponse;

/// Trigger an immediate CA rotation.
///
/// Signals the CA rotation background task to execute immediately.
/// After rotation, the controller broadcasts `CaBundleUpdated` and
/// `RequestCertRenewal` to all connected agents.
///
/// Requires authentication (handled by the `require_auth` layer).
#[utoipa::path(
    post,
    path = "/api/v1/settings/rotate-ca",
    responses(
        (status = 200, description = "CA rotation triggered", body = RotateCaResponse),
        (status = 400, description = "CA rotation not available"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_global_settings")))
)]
pub async fn rotate_ca(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> impl IntoResponse {
    let snapshot = state.ca_snapshot.borrow().clone();
    if !snapshot.managed {
        return error_response(
            StatusCode::BAD_REQUEST,
            "CA rotation is only available for managed (internally generated) CAs",
        );
    }

    // Signal the CA rotation background task to run immediately
    state.ca_rotation_trigger.notify_one();

    (
        StatusCode::OK,
        axum::Json(RotateCaResponse {
            message: "CA rotation triggered. Connected agents will be notified to renew their certificates.".to_string(),
        }),
    )
        .into_response()
}
