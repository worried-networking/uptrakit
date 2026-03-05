use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{password, token};
use crate::error_response::error_response;
use crate::middleware::permission::CanManageSystemServices;
use crate::queries::system_enrollment_tokens as set_queries;
use uptrakit_web_api_types::validation::Validate;

pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::system_enrollment_tokens::{
    CreateSystemEnrollmentTokenRequest, ListSystemEnrollmentTokensQuery,
    SystemEnrollmentTokenCreatedResponse, SystemEnrollmentTokenResponse,
};

/// Create a new system enrollment token.
#[utoipa::path(
    post,
    path = "/api/v1/system-enrollment-tokens",
    request_body = CreateSystemEnrollmentTokenRequest,
    responses(
        (status = 201, description = "System enrollment token created", body = SystemEnrollmentTokenCreatedResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_system_enrollment_token(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(user): CanManageSystemServices,
    Json(body): Json<CreateSystemEnrollmentTokenRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let plaintext = match token::generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to generate system enrollment token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hash = match password::hash_password(&plaintext) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to hash system enrollment token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let id = Uuid::now_v7();
    let expires_at = body
        .expires_in_seconds
        .map(|secs| OffsetDateTime::now_utc() + time::Duration::seconds(secs as i64));

    let model = match set_queries::create_system_enrollment_token(
        state.db(),
        set_queries::CreateSystemTokenParams {
            id,
            name: &body.name,
            token_hash: hash.expose_secret(),
            max_uses: body.max_uses,
            expires_at,
            created_by_user_id: Some(user.user_id),
        },
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to create system enrollment token: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    (
        StatusCode::CREATED,
        Json(SystemEnrollmentTokenCreatedResponse {
            id: model.id,
            token: uptrakit_web_api_types::SecretString::new(plaintext),
            name: model.name,
            max_uses: model.max_uses.map(|v| v as u32),
            current_uses: model.current_uses as u32,
            expires_at: model.expires_at,
            created_at: model.created_at,
            created_by_user_id: model.created_by_user_id,
        }),
    )
        .into_response()
}

/// List system enrollment tokens.
#[utoipa::path(
    get,
    path = "/api/v1/system-enrollment-tokens",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of system enrollment tokens", body = PaginatedResponse<SystemEnrollmentTokenResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_system_enrollment_tokens(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(_user): CanManageSystemServices,
    Query(query): Query<ListSystemEnrollmentTokensQuery>,
) -> Response {
    match set_queries::list_system_enrollment_tokens(state.db(), &query.pagination()).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list system enrollment tokens: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single system enrollment token by ID.
#[utoipa::path(
    get,
    path = "/api/v1/system-enrollment-tokens/{id}",
    params(
        ("id" = Uuid, Path, description = "System enrollment token UUID")
    ),
    responses(
        (status = 200, description = "System enrollment token details", body = SystemEnrollmentTokenResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System enrollment token not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_system_enrollment_token(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(_user): CanManageSystemServices,
    Path(token_id): Path<Uuid>,
) -> Response {
    match set_queries::get_system_enrollment_token(state.db(), token_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "System enrollment token not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Revoke a system enrollment token (soft-delete).
#[utoipa::path(
    delete,
    path = "/api/v1/system-enrollment-tokens/{id}",
    params(
        ("id" = Uuid, Path, description = "System enrollment token UUID")
    ),
    responses(
        (status = 204, description = "System enrollment token revoked"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System enrollment token not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn revoke_system_enrollment_token(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(_user): CanManageSystemServices,
    Path(token_id): Path<Uuid>,
) -> Response {
    match set_queries::revoke_system_enrollment_token(state.db(), token_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(
            StatusCode::NOT_FOUND,
            "System enrollment token not found or already revoked",
        ),
        Err(e) => {
            tracing::error!("Failed to revoke system enrollment token: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
