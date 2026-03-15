use crate::AppState;
use crate::auth::api_token::ApiTokenService;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::require_auth::AuthenticatedUser;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uuid::Uuid;

use uptrakit_web_api_types::SecretString;
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
    extensions(("x-required-permission" = json!("self"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_api_token(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Validated(req): Validated<CreateApiTokenRequest>,
) -> Response {
    let service = ApiTokenService::new(state.db().clone());

    match service.create_token(auth_user.user_id, &req.name).await {
        Ok(created) => {
            let response = CreateApiTokenResponse {
                id: created.id,
                token: SecretString::new(created.plaintext_token),
                created_at: created.created_at,
            };
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create API token: {:?}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
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
    extensions(("x-required-permission" = json!("self"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_api_tokens(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
) -> Response {
    let service = ApiTokenService::new(state.db().clone());

    match service.list_tokens(auth_user.user_id).await {
        Ok(tokens) => {
            let response = ApiTokenListResponse {
                tokens: tokens
                    .into_iter()
                    .map(|t| ApiTokenResponse {
                        id: t.id,
                        name: t.name,
                        created_at: t.created_at,
                        last_used_at: t.last_used_at,
                        revoked_at: t.revoked_at,
                    })
                    .collect(),
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list API tokens: {:?}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Revoke an API token
#[utoipa::path(
    delete,
    path = "/api/v1/auth/api-tokens/{id}",
    params(
        ("id" = Uuid, Path, description = "API token ID")
    ),
    responses(
        (status = 204, description = "API token revoked"),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "API token not found")
    ),
    tag = "Authentication",
    extensions(("x-required-permission" = json!("self"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn revoke_api_token(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(token_id): Path<Uuid>,
) -> Response {
    let service = ApiTokenService::new(state.db().clone());

    match service.revoke_token(token_id, auth_user.user_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => error_response(StatusCode::NOT_FOUND, "API token not found"),
    }
}
