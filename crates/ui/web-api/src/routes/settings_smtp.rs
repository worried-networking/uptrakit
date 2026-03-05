//! HTTP handlers for `GET /api/v1/settings/smtp` and `PUT /api/v1/settings/smtp`.
//!
//! Per-tenant SMTP settings are stored as individual rows in the `settings`
//! key-value table. The SMTP password is encrypted at rest using
//! `uptrakit_crypto::encrypt_str`.

use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSettings, CanViewSettings};
use crate::settings::SmtpSettingsSnapshot;
use crate::settings_store::upsert_setting;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub use uptrakit_web_api_types::settings_smtp::{SmtpSettingsResponse, UpdateSmtpSettingsRequest};
use uptrakit_web_api_types::validation::Validate;

fn snapshot_to_response(smtp: &SmtpSettingsSnapshot) -> SmtpSettingsResponse {
    SmtpSettingsResponse {
        host: smtp.host.clone(),
        port: smtp.port,
        username: smtp.username.clone(),
        has_password: smtp.password.is_some(),
        from_address: smtp.from_address.clone(),
        from_name: smtp.from_name.clone(),
        tls_mode: smtp.tls_mode.clone(),
    }
}

/// Get SMTP settings
#[utoipa::path(
    get,
    path = "/api/v1/settings/smtp",
    responses(
        (status = 200, description = "SMTP settings", body = SmtpSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_smtp_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let smtp = state.settings.smtp();
    (StatusCode::OK, Json(snapshot_to_response(&smtp))).into_response()
}

/// Update SMTP settings
#[utoipa::path(
    put,
    path = "/api/v1/settings/smtp",
    request_body = UpdateSmtpSettingsRequest,
    responses(
        (status = 200, description = "SMTP settings updated", body = SmtpSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_smtp_settings(
    State(state): State<Arc<AppState>>,
    CanManageSettings(_user): CanManageSettings,
    Json(req): Json<UpdateSmtpSettingsRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let mut smtp = state.settings.smtp();

    if let Some(host) = req.host {
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpHost,
            serde_json::json!(host),
        )
        .await
        {
            tracing::error!("Failed to save smtp.host: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.host = Some(host).filter(|h| !h.is_empty());
    }

    if let Some(port) = req.port {
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpPort,
            serde_json::json!(port),
        )
        .await
        {
            tracing::error!("Failed to save smtp.port: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.port = Some(port);
    }

    if let Some(ref val) = req.username {
        let new_username: Option<String> = if val.is_null() {
            None
        } else {
            val.as_str().map(String::from)
        };
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpUsername,
            serde_json::json!(new_username.as_deref().unwrap_or("")),
        )
        .await
        {
            tracing::error!("Failed to save smtp.username: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.username = new_username;
    }

    if let Some(ref val) = req.password {
        let new_password: Option<String> = if val.is_null() {
            None
        } else {
            val.as_str().map(String::from)
        };
        let stored_value = if let Some(ref pw) = new_password {
            match uptrakit_crypto::encrypt_str(pw, "uptrakit:settings:smtp_password") {
                Ok(encrypted) => serde_json::json!(encrypted),
                Err(e) => {
                    tracing::error!("Failed to encrypt SMTP password: {e:?}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        } else {
            // null clears the stored value
            serde_json::json!("")
        };
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpPassword,
            stored_value,
        )
        .await
        {
            tracing::error!("Failed to save smtp.password: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.password = new_password;
    }

    if let Some(from_address) = req.from_address {
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpFromAddress,
            serde_json::json!(from_address),
        )
        .await
        {
            tracing::error!("Failed to save smtp.from_address: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.from_address = Some(from_address).filter(|a| !a.is_empty());
    }

    if let Some(ref val) = req.from_name {
        let new_name: Option<String> = if val.is_null() {
            None
        } else {
            val.as_str().map(String::from)
        };
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpFromName,
            serde_json::json!(new_name.as_deref().unwrap_or("")),
        )
        .await
        {
            tracing::error!("Failed to save smtp.from_name: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.from_name = new_name;
    }

    if let Some(tls_mode) = req.tls_mode {
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpTlsMode,
            serde_json::json!(tls_mode),
        )
        .await
        {
            tracing::error!("Failed to save smtp.tls_mode: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.tls_mode = tls_mode;
    }

    state.settings.set_smtp(smtp.clone()).await;

    (StatusCode::OK, Json(snapshot_to_response(&smtp))).into_response()
}
