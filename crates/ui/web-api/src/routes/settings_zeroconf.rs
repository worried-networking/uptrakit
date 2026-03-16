//! HTTP handlers for `GET /api/v1/global-settings/zeroconf` and
//! `PUT /api/v1/global-settings/zeroconf`.
//!
//! Zeroconf settings control automatic service discovery and enrollment via
//! mDNS/DNS-SD. Changes take effect after the controller is restarted.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub use uptrakit_web_api_types::settings_zeroconf::{
    UpdateZeroconfSettingsRequest, ZeroconfSettingsResponse,
};

use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::settings::ZeroconfSnapshot;
use crate::settings_store::upsert_global_setting;

/// Get zeroconf settings
///
/// Returns the current zeroconf discovery configuration including the
/// read-only CA fingerprint used for trust-on-first-use verification.
#[utoipa::path(
    get,
    path = "/api/v1/global-settings/zeroconf",
    responses(
        (status = 200, description = "Zeroconf settings", body = ZeroconfSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_zeroconf_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let snap = state.settings.zeroconf();
    let ca_fingerprint = state.cert.ca_snapshot.borrow().active_fingerprint.clone();
    let resp = ZeroconfSettingsResponse {
        enabled: snap.enabled,
        url: snap.url,
        pki_addr: snap.pki_addr,
        ca_fingerprint: Some(ca_fingerprint),
    };
    (StatusCode::OK, Json(resp)).into_response()
}

/// Update zeroconf settings
///
/// Update the zeroconf discovery configuration. All fields are optional —
/// omitted fields keep their current value.
///
/// - `url`: empty string clears the value; non-empty must start with `https://`.
/// - `pki_addr`: empty string clears the value; non-empty must start with
///   `http://` or `https://`.
#[utoipa::path(
    put,
    path = "/api/v1/global-settings/zeroconf",
    request_body = UpdateZeroconfSettingsRequest,
    responses(
        (status = 200, description = "Zeroconf settings updated", body = ZeroconfSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_zeroconf_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
    Validated(req): Validated<UpdateZeroconfSettingsRequest>,
) -> Response {
    let mut snap = state.settings.zeroconf();

    // --- enabled ---
    if let Some(val) = req.enabled {
        if let Err(e) = upsert_global_setting(
            state.db(),
            SettingKey::ZeroconfEnabled,
            serde_json::json!(val),
        )
        .await
        {
            tracing::error!("Failed to save zeroconf.enabled: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        snap.enabled = val;
    }

    // --- url ---
    if let Some(ref raw) = req.url {
        let trimmed = raw.trim_end_matches('/');
        let (db_value, new_url) = if trimmed.is_empty() {
            (serde_json::Value::Null, None)
        } else {
            (serde_json::json!(trimmed), Some(trimmed.to_string()))
        };
        if let Err(e) = upsert_global_setting(state.db(), SettingKey::ZeroconfUrl, db_value).await {
            tracing::error!("Failed to save zeroconf.url: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        snap.url = new_url;
    }

    // --- pki_addr ---
    if let Some(ref raw) = req.pki_addr {
        let trimmed = raw.trim_end_matches('/');
        let (db_value, new_addr) = if trimmed.is_empty() {
            (serde_json::Value::Null, None)
        } else {
            (serde_json::json!(trimmed), Some(trimmed.to_string()))
        };
        if let Err(e) =
            upsert_global_setting(state.db(), SettingKey::ZeroconfPkiAddr, db_value).await
        {
            tracing::error!("Failed to save zeroconf.pki_addr: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        snap.pki_addr = new_addr;
    }

    // Update in-memory cache.
    let updated = ZeroconfSnapshot {
        enabled: snap.enabled,
        url: snap.url.clone(),
        pki_addr: snap.pki_addr.clone(),
    };
    state.settings.set_zeroconf(updated).await;

    let ca_fingerprint = state.cert.ca_snapshot.borrow().active_fingerprint.clone();
    let resp = ZeroconfSettingsResponse {
        enabled: snap.enabled,
        url: snap.url,
        pki_addr: snap.pki_addr,
        ca_fingerprint: Some(ca_fingerprint),
    };
    (StatusCode::OK, Json(resp)).into_response()
}
