use crate::AppState;
use crate::auth::permissions::Permission;
use crate::auth::{password, token};
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, Set,
};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_shared_db::entity::{mqtt_enrollment_token, prelude::*};

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

    // Parse optional expiry
    let expires_at = if let Some(ref exp) = request.expires_at {
        match time::OffsetDateTime::parse(exp, &Rfc3339) {
            Ok(dt) => Some(dt),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    "Invalid expires_at format (expected RFC 3339)",
                )
                    .into_response();
            }
        }
    } else {
        None
    };

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

    let now = OffsetDateTime::now_utc();
    let token_id = uuid::Uuid::now_v7();

    let model = mqtt_enrollment_token::ActiveModel {
        id: Set(token_id),
        tenant_id: Set(tenant.tenant_id),
        name: Set(request.name.clone()),
        token_hash: Set(token_hash),
        expires_at: Set(expires_at),
        uses_remaining: Set(request.uses_remaining),
        created_by: Set(user.user_id),
        created_at: Set(now),
    };

    let inserted = match model.insert(&state.db).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to insert MQTT enrollment token: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    (
        StatusCode::CREATED,
        Json(MqttEnrollmentTokenResponse {
            id: inserted.id.to_string(),
            name: inserted.name,
            token: plaintext,
            expires_at: inserted.expires_at.map(format_rfc3339),
            uses_remaining: inserted.uses_remaining,
            created_at: format_rfc3339(inserted.created_at),
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

    let now = OffsetDateTime::now_utc();

    let tokens = match MqttEnrollmentToken::find()
        .filter(mqtt_enrollment_token::Column::TenantId.eq(tenant.tenant_id))
        // Exclude expired tokens
        .filter(
            Condition::any()
                .add(mqtt_enrollment_token::Column::ExpiresAt.is_null())
                .add(mqtt_enrollment_token::Column::ExpiresAt.gt(now)),
        )
        // Exclude exhausted tokens
        .filter(
            Condition::any()
                .add(mqtt_enrollment_token::Column::UsesRemaining.is_null())
                .add(mqtt_enrollment_token::Column::UsesRemaining.gt(0)),
        )
        .order_by_desc(mqtt_enrollment_token::Column::CreatedAt)
        .all(&state.db)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to list MQTT enrollment tokens: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let response: Vec<MqttEnrollmentTokenListResponse> = tokens
        .into_iter()
        .map(|t| MqttEnrollmentTokenListResponse {
            id: t.id.to_string(),
            name: t.name,
            expires_at: t.expires_at.map(format_rfc3339),
            uses_remaining: t.uses_remaining,
            created_at: format_rfc3339(t.created_at),
        })
        .collect();

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
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let token_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid token ID").into_response(),
    };

    let result = match MqttEnrollmentToken::delete_by_id(token_id)
        .filter(mqtt_enrollment_token::Column::TenantId.eq(tenant.tenant_id))
        .exec(&state.db)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to delete MQTT enrollment token: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if result.rows_affected == 0 {
        return (StatusCode::NOT_FOUND, "Token not found").into_response();
    }

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: "MQTT enrollment token revoked".to_string(),
        }),
    )
        .into_response()
}

// --- Helper functions ---

fn format_rfc3339(dt: OffsetDateTime) -> String {
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}
