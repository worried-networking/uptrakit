use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::app_state::{AuditEmitterState, DbState};
use crate::auth::{password, token};
use crate::error_response::error_response;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::system_enrollment_tokens as set_queries;
use uptrakit_web_api_types::validation::Validate;

pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::system_enrollment_tokens::{
    CreateSystemEnrollmentTokenRequest, ListSystemEnrollmentTokensQuery,
    SystemEnrollmentTokenCreatedResponse, SystemEnrollmentTokenResponse,
};

fn emit_system_enrollment_token_audit(
    audit_emitter: &uptrakit_audit_log::AuditEmitter,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_token_id: Option<Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);

    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .system_scope()
        .actor(actor_type, actor_id)
        .outcome(outcome)
        .details(details);

    if let Some(token_id) = target_token_id {
        builder = builder.target("system_enrollment_token", token_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        audit_emitter.emit_best_effort(entry);
    }
}

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
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_system_enrollment_token(
    State(db): State<DbState>,
    State(audit): State<AuditEmitterState>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<CreateSystemEnrollmentTokenRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    if let Err(e) = body.validate() {
        emit_system_enrollment_token_audit(
            &audit.0,
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
            tracing::error!(error = ?e, "Failed to generate system enrollment token");
            emit_system_enrollment_token_audit(
                &audit.0,
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
            tracing::error!(error = ?e, "Failed to hash system enrollment token");
            emit_system_enrollment_token_audit(
                &audit.0,
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

    let model = match set_queries::create_system_enrollment_token(
        db.db(),
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
            tracing::error!(error = %e, "Failed to create system enrollment token");
            emit_system_enrollment_token_audit(
                &audit.0,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
                Some(id),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "token_name": body.name,
                    "reason_code": "system_enrollment_token_create_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    emit_system_enrollment_token_audit(
        &audit.0,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
        Some(model.id),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "token_name": model.name,
            "has_expiry": model.expires_at.is_some(),
            "max_uses": model.max_uses,
        }),
    );

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
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_system_enrollment_tokens(
    State(db): State<DbState>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
    Query(query): Query<ListSystemEnrollmentTokensQuery>,
) -> Response {
    match set_queries::list_system_enrollment_tokens(db.db(), &query.pagination()).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list system enrollment tokens");
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
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_system_enrollment_token(
    State(db): State<DbState>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
    Path(token_id): Path<Uuid>,
) -> Response {
    match set_queries::get_system_enrollment_token(db.db(), token_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "System enrollment token not found"),
        Err(e) => {
            tracing::error!(error = %e, "DB error");
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
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn revoke_system_enrollment_token(
    State(db): State<DbState>,
    State(audit): State<AuditEmitterState>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(token_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    match set_queries::revoke_system_enrollment_token(db.db(), token_id).await {
        Ok(true) => {
            emit_system_enrollment_token_audit(
                &audit.0,
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
            emit_system_enrollment_token_audit(
                &audit.0,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_REVOKE,
                Some(token_id),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "system_enrollment_token_not_found_or_revoked",
                }),
            );
            error_response(
                StatusCode::NOT_FOUND,
                "System enrollment token not found or already revoked",
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to revoke system enrollment token");
            emit_system_enrollment_token_audit(
                &audit.0,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_REVOKE,
                Some(token_id),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "system_enrollment_token_revoke_failed",
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use axum::extract::FromRef;
    use sea_orm::{
        ActiveModelTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryOrder, Set,
    };
    use uptrakit_shared_db::entity::{system_audit_log, system_enrollment_token};

    async fn latest_system_audit_row(db: &DatabaseConnection) -> system_audit_log::Model {
        for _ in 0..40 {
            if let Some(row) = system_audit_log::Entity::find()
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected at least one system audit row");
    }

    async fn wait_for_system_audit_rows(db: &DatabaseConnection, expected: u64) {
        for _ in 0..40 {
            let count = system_audit_log::Entity::find()
                .count(db)
                .await
                .expect("count system audit rows");
            if count == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected {expected} system audit rows");
    }

    async fn insert_system_enrollment_token(
        db: &DatabaseConnection,
    ) -> system_enrollment_token::Model {
        let now = time::OffsetDateTime::now_utc();
        system_enrollment_token::ActiveModel {
            id: Set(Uuid::now_v7()),
            name: Set("System Token".to_string()),
            token_hash: Set("hashed-token".to_string()),
            max_uses: Set(Some(5)),
            current_uses: Set(0),
            expires_at: Set(None),
            created_at: Set(now),
            revoked_at: Set(None),
            created_by_user_id: Set(None),
        }
        .insert(db)
        .await
        .expect("insert system enrollment token")
    }

    #[tokio::test]
    async fn create_system_enrollment_token_writes_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ManageGlobalSettings],
            jti: None,
        };

        let response = create_system_enrollment_token(
            State(DbState::from_ref(&state)),
            State(AuditEmitterState::from_ref(&state)),
            CanManageGlobalSettings::new(auth_user),
            None,
            Json(CreateSystemEnrollmentTokenRequest {
                name: "System Token".to_string(),
                max_uses: Some(5),
                expires_in_seconds: Some(3600),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CREATED);
        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_CREATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("system_enrollment_token"));
    }

    #[tokio::test]
    async fn revoke_system_enrollment_token_missing_writes_denied_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let missing_id = Uuid::now_v7();

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ManageGlobalSettings],
            jti: None,
        };

        let response = revoke_system_enrollment_token(
            State(DbState::from_ref(&state)),
            State(AuditEmitterState::from_ref(&state)),
            CanManageGlobalSettings::new(auth_user),
            None,
            Path(missing_id),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_REVOKE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_id.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn revoke_system_enrollment_token_success_writes_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let token = insert_system_enrollment_token(&db).await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ManageGlobalSettings],
            jti: None,
        };

        let response = revoke_system_enrollment_token(
            State(DbState::from_ref(&state)),
            State(AuditEmitterState::from_ref(&state)),
            CanManageGlobalSettings::new(auth_user),
            None,
            Path(token.id),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::ENROLLMENT_TOKEN_REVOKE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(token.id.to_string().as_str())
        );
    }
}
