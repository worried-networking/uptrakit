use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::response::IntoResponse;
use http::StatusCode;

use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;

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
    tag = "Settings"
)]
pub async fn rotate_ca(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> impl IntoResponse {
    if !user.has_permission(Permission::ManageGlobalSettings) {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"error": "Insufficient permissions"})),
        )
            .into_response();
    }

    let snapshot = state.ca_snapshot.borrow().clone();
    if !snapshot.managed {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": "CA rotation is only available for managed (internally generated) CAs"
            })),
        )
            .into_response();
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
