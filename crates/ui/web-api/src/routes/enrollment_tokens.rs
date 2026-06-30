use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{password, token};
use crate::error_response::error_response;
use crate::middleware::permission::CanManageEnrollmentTokens;
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::enrollment_tokens as et_queries;
use crate::tenant_db::TenantDb;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_web_api_queries::queries::enrollment_tokens::{
    EnrollmentTokenView, create_enrollment_token_in_tx, revoke_enrollment_token_in_tx,
};
use uptrakit_web_api_types::validation::Validate;

pub use uptrakit_web_api_types::enrollment_tokens::{
    CreateEnrollmentTokenRequest, EnrollmentTokenCreatedResponse, EnrollmentTokenResponse,
    EnrollmentTokensSummary, ListEnrollmentTokensQuery,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;

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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageEnrollmentTokens(user): CanManageEnrollmentTokens,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<CreateEnrollmentTokenRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    if let Err(e) = body.validate() {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::ValidationFailed)
        .details(serde_json::json!({ "reason_code": "invalid_request" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let plaintext = match token::generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to generate enrollment token");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "token_generation_failed" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hash = match password::hash_password(&plaintext) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to hash enrollment token");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "token_hash_failed" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let id = Uuid::now_v7();
    let expires_at = body
        .expires_in_seconds
        .map(|secs| OffsetDateTime::now_utc() + time::Duration::seconds(secs as i64));

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "Failed to begin transaction for enrollment token create");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let model = match create_enrollment_token_in_tx(
        &tx,
        tenant_id,
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
            drop(tx);
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({
                "token_name": body.name,
                "reason_code": "enrollment_token_create_failed",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let after_view = EnrollmentTokenView::from(&model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::enrollment_token_create(
        &AbsentView(&after_view),
        &after_view,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "token_name": model.name,
        "has_capability_filter": model.allowed_capabilities.is_some(),
        "has_expiry": model.expires_at.is_some(),
        "max_uses": model.max_uses,
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for enrollment token create");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for enrollment token create");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit enrollment token create");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    let allowed_capabilities: Option<Vec<String>> = model
        .allowed_capabilities
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

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
    params(ListEnrollmentTokensQuery),
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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageEnrollmentTokens(user): CanManageEnrollmentTokens,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(token_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "Failed to begin transaction for enrollment token revoke");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let pair = match revoke_enrollment_token_in_tx(&tx, tenant_id, token_id).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!(error = %e, "Failed to revoke enrollment token");
            drop(tx);
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_REVOKE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("enrollment_token", token_id.to_string(), None)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "enrollment_token_revoke_failed" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let Some((before_model, after_model)) = pair else {
        drop(tx);
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_REVOKE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("enrollment_token", token_id.to_string(), None)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "reason_code": "enrollment_token_not_found_or_revoked",
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(
            StatusCode::NOT_FOUND,
            "Enrollment token not found or already revoked",
        );
    };

    let before_view = EnrollmentTokenView::from(&before_model);
    let after_view = EnrollmentTokenView::from(&after_model);

    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::enrollment_token_revoke(
        &before_view,
        &AbsentView(&after_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({ "token_name": before_model.name }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for enrollment token revoke");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for enrollment token revoke");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit enrollment token revoke");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
}
