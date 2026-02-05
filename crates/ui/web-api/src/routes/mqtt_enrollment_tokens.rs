use crate::AppState;
use crate::SettingKey;
use crate::auth::permissions::Permission;
use crate::auth::{password, token};
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use crate::settings_store::{delete_setting, load_setting, upsert_setting};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub use uptrakit_web_api_types::agents::MessageResponse;
pub use uptrakit_web_api_types::mqtt_services::{
    CreateMqttEnrollmentTokenRequest, MqttEnrollmentTokenListResponse, MqttEnrollmentTokenResponse,
};

/// Create a new MQTT enrollment token
#[utoipa::path(
    post,
    path = "/api/v1/mqtt-enrollment-tokens",
    request_body = CreateMqttEnrollmentTokenRequest,
    responses(
        (status = 201, description = "MQTT enrollment token created", body = MqttEnrollmentTokenResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "MQTT Enrollment Tokens",
    security(("bearer_token" = []))
)]
pub async fn create_mqtt_enrollment_token(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Json(request): Json<CreateMqttEnrollmentTokenRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    if request.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "Token name must not be empty").into_response();
    }

    // Generate token
    let plaintext = match token::generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to generate MQTT enrollment token: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let token_hash = match password::hash_password(&plaintext) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to hash MQTT enrollment token: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Store as a setting
    if let Err(e) = upsert_setting(
        &state.db,
        tenant.tenant_id,
        SettingKey::MqttEnrollmentTokenHash,
        serde_json::Value::String(token_hash),
    )
    .await
    {
        tracing::error!("Failed to store MQTT enrollment token hash: {:?}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let now = time::OffsetDateTime::now_utc();
    let now_str = now
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| now.to_string());

    (
        StatusCode::CREATED,
        Json(MqttEnrollmentTokenResponse {
            id: uuid::Uuid::now_v7().to_string(),
            name: request.name,
            token: plaintext,
            expires_at: None,
            uses_remaining: None,
            created_at: now_str,
        }),
    )
        .into_response()
}

/// List all MQTT enrollment tokens
#[utoipa::path(
    get,
    path = "/api/v1/mqtt-enrollment-tokens",
    responses(
        (status = 200, description = "List of MQTT enrollment tokens", body = Vec<MqttEnrollmentTokenListResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "MQTT Enrollment Tokens",
    security(("bearer_token" = []))
)]
pub async fn list_mqtt_enrollment_tokens(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    // Check if an MQTT enrollment token is configured
    let configured = matches!(
        load_setting(
            &state.db,
            tenant.tenant_id,
            SettingKey::MqttEnrollmentTokenHash,
        )
        .await,
        Ok(Some(_))
    );

    let response: Vec<MqttEnrollmentTokenListResponse> = if configured {
        vec![MqttEnrollmentTokenListResponse {
            id: "active".to_string(),
            name: "MQTT enrollment token".to_string(),
            expires_at: None,
            uses_remaining: None,
            created_at: String::new(),
        }]
    } else {
        vec![]
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Revoke an MQTT enrollment token
#[utoipa::path(
    delete,
    path = "/api/v1/mqtt-enrollment-tokens/{id}",
    params(
        ("id" = String, Path, description = "MQTT Enrollment Token UUID")
    ),
    responses(
        (status = 200, description = "MQTT enrollment token revoked", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Token not found")
    ),
    tag = "MQTT Enrollment Tokens",
    security(("bearer_token" = []))
)]
pub async fn revoke_mqtt_enrollment_token(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(_id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    if let Err(e) = delete_setting(
        &state.db,
        tenant.tenant_id,
        SettingKey::MqttEnrollmentTokenHash,
    )
    .await
    {
        tracing::error!("Failed to delete MQTT enrollment token: {:?}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: "MQTT enrollment token revoked".to_string(),
        }),
    )
        .into_response()
}
