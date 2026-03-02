use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSettings, CanViewSettings};
use crate::settings_store::{delete_setting, upsert_setting};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub use uptrakit_web_api_types::settings_agent_certs::{
    AgentCertificateSettingsResponse, UpdateAgentCertificateSettingsRequest,
};

const MAX_AGENT_CERT_LIFETIME_DAYS: u16 = 730;

fn build_response(state: &AppState) -> AgentCertificateSettingsResponse {
    AgentCertificateSettingsResponse {
        lifetime_days: state.settings.agent_cert_lifetime_days(),
        renewal_window_hours_override: state.settings.renewal_window_hours_override(),
        effective_renewal_window_hours: state.settings.renewal_window_hours(),
    }
}

/// Get agent certificate settings
#[utoipa::path(
    get,
    path = "/api/v1/settings/agent-certificates",
    responses(
        (status = 200, description = "Agent certificate settings", body = AgentCertificateSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
pub async fn get_agent_certificate_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    (StatusCode::OK, Json(build_response(&state))).into_response()
}

/// Update agent certificate settings
#[utoipa::path(
    put,
    path = "/api/v1/settings/agent-certificates",
    request_body = UpdateAgentCertificateSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = AgentCertificateSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_settings"))),
    security(("bearer_token" = []))
)]
pub async fn update_agent_certificate_settings(
    State(state): State<Arc<AppState>>,
    CanManageSettings(_user): CanManageSettings,
    Json(req): Json<UpdateAgentCertificateSettingsRequest>,
) -> Response {
    if let Some(days) = req.lifetime_days {
        if days < 1 {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Certificate lifetime must be at least 1 day",
            );
        }
        if days > MAX_AGENT_CERT_LIFETIME_DAYS {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Certificate lifetime must not exceed 730 days",
            );
        }
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::AgentCertLifetimeDays,
            serde_json::json!(days),
        )
        .await
        {
            tracing::error!("Failed to save agent cert lifetime: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        state.settings.set_agent_cert_lifetime_days(days).await;
    }

    if let Some(hours) = req.renewal_window_hours {
        if hours == 0 {
            // Reset to automatic mode: remove the override from the DB.
            if let Err(e) = delete_setting(
                state.db(),
                state.default_tenant_id,
                SettingKey::AgentCertRenewalWindowHours,
            )
            .await
            {
                tracing::error!("Failed to delete renewal window setting: {e:?}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            state
                .settings
                .set_renewal_window_hours_override(None)
                .await;
        } else {
            if let Err(e) = upsert_setting(
                state.db(),
                state.default_tenant_id,
                SettingKey::AgentCertRenewalWindowHours,
                serde_json::json!(hours),
            )
            .await
            {
                tracing::error!("Failed to save renewal window: {e:?}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            state
                .settings
                .set_renewal_window_hours_override(Some(hours))
                .await;
        }
    }

    (StatusCode::OK, Json(build_response(&state))).into_response()
}
