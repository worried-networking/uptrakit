use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::app_state::AuditEmitterState;
use crate::auth::{password, token};
use crate::error_response::error_response;
use crate::middleware::permission::CanManageEnrollmentTokens;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::enrollment_tokens as et_queries;
use crate::tenant_db::TenantDb;
use uptrakit_web_api_types::validation::Validate;

pub use uptrakit_web_api_types::enrollment_tokens::{
    CreateEnrollmentTokenRequest, EnrollmentTokenCreatedResponse, EnrollmentTokenResponse,
    EnrollmentTokensSummary, ListEnrollmentTokensQuery,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;

fn emit_enrollment_token_audit(
    audit_emitter: &uptrakit_audit_log::AuditEmitter,
    tenant_id: Uuid,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_token_id: Option<Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);

    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(outcome)
        .details(details);

    if let Some(token_id) = target_token_id {
        builder = builder.target("enrollment_token", token_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        audit_emitter.emit_best_effort(entry);
    }
}

/// Create a new enrollment token
#[utoipa::path(
    post,
    path = "/api/v1/enrollment-tokens",
    request_body = CreateEnrollmentTokenRequest,
    responses(
        (status = 201, description = "Enrollment token created", body = EnrollmentTokenCreatedResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Enrollment Tokens",
    extensions(("x-required-permission" = json!("manage_enrollment_tokens"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_enrollment_token(
    State(audit): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanManageEnrollmentTokens(user): CanManageEnrollmentTokens,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<CreateEnrollmentTokenRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);

    if let Err(e) = body.validate() {
        emit_enrollment_token_audit(
            &audit.0,
            tenant_db.tenant_id,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let plaintext = match token::generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to generate enrollment token");
            emit_enrollment_token_audit(
                &audit.0,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "token_generation_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hash = match password::hash_password(&plaintext) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to hash enrollment token");
            emit_enrollment_token_audit(
                &audit.0,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "token_hash_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let id = Uuid::now_v7();
    let expires_at = body
        .expires_in_seconds
        .map(|secs| OffsetDateTime::now_utc() + time::Duration::seconds(secs as i64));

    let model = match et_queries::create_enrollment_token(
        &tenant_db,
        et_queries::CreateTokenParams {
            id,
            name: &body.name,
            token_hash: hash.expose_secret(),
            allowed_capabilities: body.allowed_capabilities.as_deref(),
            max_uses: body.max_uses,
            expires_at,
            created_by_user_id: Some(user.user_id),
        },
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create enrollment token");
            emit_enrollment_token_audit(
                &audit.0,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
                Some(id),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "token_name": body.name,
                    "reason_code": "enrollment_token_create_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let allowed_capabilities: Option<Vec<String>> = model
        .allowed_capabilities
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    emit_enrollment_token_audit(
        &audit.0,
        tenant_db.tenant_id,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
        Some(model.id),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "token_name": model.name,
            "has_capability_filter": model.allowed_capabilities.is_some(),
            "has_expiry": model.expires_at.is_some(),
            "max_uses": model.max_uses,
        }),
    );

    (
        StatusCode::CREATED,
        Json(EnrollmentTokenCreatedResponse {
            id: model.id,
            token: uptrakit_web_api_types::SecretString::new(plaintext),
            name: model.name,
            allowed_capabilities,
            max_uses: model.max_uses.map(|v| v as u32),
            current_uses: model.current_uses as u32,
            expires_at: model.expires_at,
            created_at: model.created_at,
            created_by_user_id: model.created_by_user_id,
        }),
    )
        .into_response()
}

/// List enrollment tokens
#[utoipa::path(
    get,
    path = "/api/v1/enrollment-tokens",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of enrollment tokens", body = PaginatedResponse<EnrollmentTokenResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Enrollment Tokens",
    extensions(("x-required-permission" = json!("manage_enrollment_tokens"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_enrollment_tokens(
    tenant_db: TenantDb,
    CanManageEnrollmentTokens(_user): CanManageEnrollmentTokens,
    Query(query): Query<ListEnrollmentTokensQuery>,
) -> Response {
    match et_queries::list_enrollment_tokens(&tenant_db, &query.pagination()).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list enrollment tokens");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single enrollment token by ID
#[utoipa::path(
    get,
    path = "/api/v1/enrollment-tokens/{id}",
    params(
        ("id" = Uuid, Path, description = "Enrollment token UUID")
    ),
    responses(
        (status = 200, description = "Enrollment token details", body = EnrollmentTokenResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Enrollment token not found")
    ),
    tag = "Enrollment Tokens",
    extensions(("x-required-permission" = json!("manage_enrollment_tokens"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_enrollment_token(
    tenant_db: TenantDb,
    CanManageEnrollmentTokens(_user): CanManageEnrollmentTokens,
    Path(token_id): Path<Uuid>,
) -> Response {
    match et_queries::get_enrollment_token(&tenant_db, token_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Enrollment token not found"),
        Err(e) => {
            tracing::error!(error = %e, "DB error");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Revoke an enrollment token (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/enrollment-tokens/{id}",
    params(
        ("id" = Uuid, Path, description = "Enrollment token UUID")
    ),
    responses(
        (status = 204, description = "Enrollment token revoked"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Enrollment token not found")
    ),
    tag = "Enrollment Tokens",
    extensions(("x-required-permission" = json!("manage_enrollment_tokens"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn revoke_enrollment_token(
    State(audit): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanManageEnrollmentTokens(user): CanManageEnrollmentTokens,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(token_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);

    match et_queries::revoke_enrollment_token(&tenant_db, token_id).await {
        Ok(true) => {
            emit_enrollment_token_audit(
                &audit.0,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_REVOKE,
                Some(token_id),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({}),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            emit_enrollment_token_audit(
                &audit.0,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_REVOKE,
                Some(token_id),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "enrollment_token_not_found_or_revoked",
                }),
            );
            error_response(
                StatusCode::NOT_FOUND,
                "Enrollment token not found or already revoked",
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to revoke enrollment token");
            emit_enrollment_token_audit(
                &audit.0,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_REVOKE,
                Some(token_id),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "enrollment_token_revoke_failed",
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
