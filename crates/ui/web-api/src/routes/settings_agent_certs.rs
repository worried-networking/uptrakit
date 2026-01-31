use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::settings_store::upsert_setting;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct AgentCertificateSettingsResponse {
    pub lifetime_days: u16,
    pub renewal_window_hours: u16,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateAgentCertificateSettingsRequest {
    pub lifetime_days: Option<u16>,
    pub renewal_window_hours: Option<u16>,
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
    security(("bearer_token" = []))
)]
pub async fn get_agent_certificate_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let response = AgentCertificateSettingsResponse {
        lifetime_days: state.settings.agent_cert_lifetime_days().await,
        renewal_window_hours: state.settings.renewal_window_hours().await,
    };
    (StatusCode::OK, Json(response)).into_response()
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
    security(("bearer_token" = []))
)]
pub async fn update_agent_certificate_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(req): Json<UpdateAgentCertificateSettingsRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    if let Some(days) = req.lifetime_days {
        if days < 1 {
            return (
                StatusCode::BAD_REQUEST,
                "Certificate lifetime must be at least 1 day",
            )
                .into_response();
        }
        if let Err(e) = upsert_setting(
            &state.db,
            "agent_certificate.lifetime_days",
            serde_json::json!(days),
        )
        .await
        {
            tracing::error!("Failed to save agent cert lifetime: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        state.settings.set_agent_cert_lifetime_days(days).await;
    }

    if let Some(hours) = req.renewal_window_hours {
        if hours < 1 {
            return (
                StatusCode::BAD_REQUEST,
                "Renewal window must be at least 1 hour",
            )
                .into_response();
        }
        if let Err(e) = upsert_setting(
            &state.db,
            "agent_certificate.renewal_window_hours",
            serde_json::json!(hours),
        )
        .await
        {
            tracing::error!("Failed to save renewal window: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        state.settings.set_renewal_window_hours(hours).await;
    }

    let response = AgentCertificateSettingsResponse {
        lifetime_days: state.settings.agent_cert_lifetime_days().await,
        renewal_window_hours: state.settings.renewal_window_hours().await,
    };
    (StatusCode::OK, Json(response)).into_response()
}
