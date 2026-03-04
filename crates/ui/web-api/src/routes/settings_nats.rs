//! HTTP handlers for `GET /api/v1/global-settings/nats` and `PUT /api/v1/global-settings/nats`.
//!
//! The NATS URL is stored as a global setting (encrypted at rest). It is used to
//! connect to NATS at startup; **hot-reload is not supported** — changes take
//! effect after the controller is restarted.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use uptrakit_web_api_types::MaskedUrl;
pub use uptrakit_web_api_types::settings_nats::{NatsSettingsResponse, UpdateNatsSettingsRequest};
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::settings_store::upsert_global_setting;

fn snapshot_to_response(nats_url: Option<MaskedUrl>) -> NatsSettingsResponse {
    let has_url = nats_url.is_some();
    NatsSettingsResponse {
        url: nats_url,
        has_url,
    }
}

/// Get NATS settings
///
/// Returns the current NATS URL configuration. The password component of the
/// URL is always redacted in the response. Changes take effect after the
/// controller is restarted.
#[utoipa::path(
    get,
    path = "/api/v1/global-settings/nats",
    responses(
        (status = 200, description = "NATS settings", body = NatsSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
pub async fn get_nats_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let nats_url = state.settings.nats_url();
    (StatusCode::OK, Json(snapshot_to_response(nats_url))).into_response()
}

/// Update NATS settings
///
/// Update the NATS server URL. The URL is encrypted at rest. Changes take
/// effect **after the controller is restarted** — the live NATS connection is
/// not replaced while the controller is running.
///
/// Send `"url": null` to clear the stored URL (NATS will be disabled after
/// the next restart).
#[utoipa::path(
    put,
    path = "/api/v1/global-settings/nats",
    request_body = UpdateNatsSettingsRequest,
    responses(
        (status = 200, description = "NATS settings updated", body = NatsSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
pub async fn update_nats_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
    Json(req): Json<UpdateNatsSettingsRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let mut nats_url = state.settings.nats_url();

    if let Some(ref val) = req.url {
        if val.is_null() {
            // Clear stored URL
            if let Err(e) =
                upsert_global_setting(state.db(), SettingKey::NatsUrl, serde_json::json!("")).await
            {
                tracing::error!("Failed to clear nats.url: {e:?}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            nats_url = None;
        } else if let Some(s) = val.as_str() {
            let stored_value = match uptrakit_crypto::encrypt_str(s, "uptrakit:settings:nats_url") {
                Ok(encrypted) => serde_json::json!(encrypted),
                Err(e) => {
                    tracing::error!("Failed to encrypt NATS URL: {e:?}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };
            if let Err(e) =
                upsert_global_setting(state.db(), SettingKey::NatsUrl, stored_value).await
            {
                tracing::error!("Failed to save nats.url: {e:?}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            nats_url = Some(MaskedUrl::new(s));
        }
    }

    state.settings.set_nats_url(nats_url.clone()).await;

    (StatusCode::OK, Json(snapshot_to_response(nats_url))).into_response()
}
