use crate::AppState;
use crate::auth::token::{generate_secure_token, generate_uuid, hash_token};
use crate::error_response::error_response;
use crate::extract::{ApiTokenSvc, Validated};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::middleware::tenant_context::TenantContext;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Stateful};
use uptrakit_web_api_queries::queries::api_tokens::{
    ApiTokenView, create_api_token_in_tx, revoke_api_token_in_tx,
};
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
    tenant: TenantContext,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<CreateApiTokenRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&auth_user, api_token_id);
    let tenant_id = tenant.tenant_id;

    let raw_token = match generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to generate secure token: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let plaintext = format!("upk_{raw_token}");
    let token_hash = hash_token(&plaintext);
    let id = generate_uuid();
    let created_at = OffsetDateTime::now_utc();

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
            tracing::error!("Failed to begin transaction for api token create: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let token_model = match create_api_token_in_tx(
        &tx,
        id,
        auth_user.user_id,
        &req.name,
        token_hash,
        created_at,
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to create API token: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let after_view = ApiTokenView::from(&token_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry =
        match AuditEntry::<Stateful>::api_token_create(&AbsentView(&after_view), &after_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({ "token_name": req.name }))
            .build()
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::error!("Failed to build audit entry for api token create: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for api token create: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit api token create: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    let response = CreateApiTokenResponse {
        id: token_model.id,
        token: SecretString::new(plaintext),
        created_at: token_model.created_at,
    };
    (StatusCode::CREATED, Json(response)).into_response()
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
    api_token_svc: ApiTokenSvc,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
) -> Response {
    match api_token_svc.list_tokens(auth_user.user_id).await {
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
    tenant: TenantContext,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(token_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&auth_user, api_token_id);
    let tenant_id = tenant.tenant_id;

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
            tracing::error!("Failed to begin transaction for api token revoke: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let pair = match revoke_api_token_in_tx(&tx, token_id, auth_user.user_id).await {
        Ok(p) => p,
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to revoke API token: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let Some((before_model, after_model)) = pair else {
        drop(tx);
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            uptrakit_audit_log::AuditActionType::API_TOKEN_REVOKE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("api_token", token_id.to_string(), None)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({ "reason_code": "api_token_not_found" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::NOT_FOUND, "API token not found");
    };

    let before_view = ApiTokenView::from(&before_model);
    let after_view = ApiTokenView::from(&after_model);

    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::api_token_revoke(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({}))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for api token revoke: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for api token revoke: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit api token revoke: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    use uptrakit_shared_db::entity::audit_log;

    async fn latest_tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::TenantId.is_not_null())
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row");
    }

    #[tokio::test]
    async fn create_api_token_writes_api_token_create_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let req = CreateApiTokenRequest {
            name: "device-cli".to_string(),
        };

        let status = client
            .post_json("/api/v1/auth/api-tokens", &req)
            .bearer(&access_token)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::CREATED);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::API_TOKEN_CREATE,
        )
        .await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::API_TOKEN_CREATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["token_name"], serde_json::json!("device-cli"));

        // before snapshot is absent ({}), after snapshot has token fields
        let before = row.before_snapshot.expect("before_snapshot");
        assert_eq!(before, serde_json::json!({}));
        let after = row.after_snapshot.expect("after_snapshot");
        // id is excluded from snapshot (used as target_id only)
        assert_eq!(after["name"], serde_json::json!("device-cli"));
        assert!(
            after.get("token_hash").is_none(),
            "token_hash must not appear in snapshot"
        );
    }

    #[tokio::test]
    async fn revoke_api_token_writes_api_token_revoke_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let create_req = CreateApiTokenRequest {
            name: "build-agent".to_string(),
        };
        let (create_status, created): (StatusCode, CreateApiTokenResponse) = client
            .post_json("/api/v1/auth/api-tokens", &create_req)
            .bearer(&access_token)
            .send_json()
            .await;
        assert_eq!(create_status, StatusCode::CREATED);

        let revoke_status = client
            .delete(&format!("/api/v1/auth/api-tokens/{}", created.id))
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(revoke_status, StatusCode::NO_CONTENT);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::API_TOKEN_REVOKE,
        )
        .await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::API_TOKEN_REVOKE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("api_token"));
        let expected_target_id = created.id.to_string();
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));

        // before snapshot has name (id excluded — used as target_id only)
        let before = row.before_snapshot.expect("before_snapshot");
        assert_eq!(before["name"], serde_json::json!("build-agent"));
        assert!(
            before.get("token_hash").is_none(),
            "token_hash must not appear in before snapshot"
        );
        // after snapshot has revoked_at set
        let after = row.after_snapshot.expect("after_snapshot");
        assert!(
            !after["revoked_at"].is_null(),
            "after snapshot must have revoked_at set"
        );
        assert!(
            after.get("token_hash").is_none(),
            "token_hash must not appear in after snapshot"
        );
    }
}
