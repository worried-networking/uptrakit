use crate::AppState;
use crate::auth::api_token::ApiTokenService;
use crate::middleware::require_auth::AuthenticatedUser;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub use uptrakit_web_api_types::api_tokens::{
    ApiTokenListResponse, ApiTokenResponse, CreateApiTokenRequest, CreateApiTokenResponse,
};

/// Create a new API token
#[utoipa::path(
    post,
    path = "/api/v1/auth/api-tokens",
    request_body = CreateApiTokenRequest,
    responses(
        (status = 201, description = "API token created", body = CreateApiTokenResponse),
        (status = 401, description = "Not authenticated")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
pub async fn create_api_token(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Json(req): Json<CreateApiTokenRequest>,
) -> Response {
    let service = ApiTokenService::new(state.db.clone());

    match service.create_token(auth_user.user_id, &req.name).await {
        Ok(created) => {
            let format = time::format_description::well_known::Rfc3339;
            let response = CreateApiTokenResponse {
                id: created.id.to_string(),
                token: created.plaintext_token,
                created_at: created.created_at.format(&format).unwrap_or_default(),
            };
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create API token: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// List user's API tokens
#[utoipa::path(
    get,
    path = "/api/v1/auth/api-tokens",
    responses(
        (status = 200, description = "List of API tokens", body = ApiTokenListResponse),
        (status = 401, description = "Not authenticated")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
pub async fn list_api_tokens(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
) -> Response {
    let service = ApiTokenService::new(state.db.clone());

    match service.list_tokens(auth_user.user_id).await {
        Ok(tokens) => {
            let format = time::format_description::well_known::Rfc3339;
            let response = ApiTokenListResponse {
                tokens: tokens
                    .into_iter()
                    .map(|t| ApiTokenResponse {
                        id: t.id.to_string(),
                        name: t.name,
                        created_at: t.created_at.format(&format).unwrap_or_default(),
                        last_used_at: t.last_used_at.and_then(|dt| dt.format(&format).ok()),
                        revoked_at: t.revoked_at.and_then(|dt| dt.format(&format).ok()),
                    })
                    .collect(),
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list API tokens: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Revoke an API token
#[utoipa::path(
    delete,
    path = "/api/v1/auth/api-tokens/{id}",
    params(
        ("id" = String, Path, description = "API token ID")
    ),
    responses(
        (status = 204, description = "API token revoked"),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "API token not found")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
pub async fn revoke_api_token(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    let token_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid token ID").into_response();
        }
    };

    let service = ApiTokenService::new(state.db.clone());

    match service.revoke_token(token_id, auth_user.user_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "API token not found").into_response(),
    }
}
