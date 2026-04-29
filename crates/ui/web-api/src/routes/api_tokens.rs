use crate::app_state::AuditEmitterState;
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
use uuid::Uuid;

use uptrakit_web_api_types::SecretString;
pub use uptrakit_web_api_types::api_tokens::{
    ApiTokenListResponse, ApiTokenResponse, CreateApiTokenRequest, CreateApiTokenResponse,
};

struct AuditContext<'a> {
    emitter: &'a uptrakit_audit_log::AuditEmitter,
    tenant_id: Uuid,
    user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
}

fn emit_api_token_mutation_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_token_id: Option<Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id, actor_display) =
        authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .actor_display_opt(actor_display)
        .outcome(outcome)
        .details(details);

    if let Some(token_id) = target_token_id {
        builder = builder.target("api_token", token_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        ctx.emitter.emit_best_effort(entry);
    }
}

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
    State(audit): State<AuditEmitterState>,
    tenant: TenantContext,
    api_token_svc: ApiTokenSvc,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<CreateApiTokenRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let ctx = AuditContext {
        emitter: &audit.0,
        tenant_id: tenant.tenant_id,
        user: &auth_user,
        api_token_id,
    };

    match api_token_svc
        .create_token(auth_user.user_id, &req.name)
        .await
    {
        Ok(created) => {
            emit_api_token_mutation_audit(
                &ctx,
                uptrakit_audit_log::AuditActionType::API_TOKEN_CREATE,
                Some(created.id),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "token_name": req.name,
                }),
            );
            let response = CreateApiTokenResponse {
                id: created.id,
                token: SecretString::new(created.plaintext_token),
                created_at: created.created_at,
            };
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create API token: {:?}", e);
            emit_api_token_mutation_audit(
                &ctx,
                uptrakit_audit_log::AuditActionType::API_TOKEN_CREATE,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "token_name": req.name,
                    "reason_code": "api_token_create_failed",
                }),
            );
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
    State(audit): State<AuditEmitterState>,
    tenant: TenantContext,
    api_token_svc: ApiTokenSvc,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(token_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let ctx = AuditContext {
        emitter: &audit.0,
        tenant_id: tenant.tenant_id,
        user: &auth_user,
        api_token_id,
    };

    match api_token_svc
        .revoke_token(token_id, auth_user.user_id)
        .await
    {
        Ok(()) => {
            emit_api_token_mutation_audit(
                &ctx,
                uptrakit_audit_log::AuditActionType::API_TOKEN_REVOKE,
                Some(token_id),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({}),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => {
            emit_api_token_mutation_audit(
                &ctx,
                uptrakit_audit_log::AuditActionType::API_TOKEN_REVOKE,
                Some(token_id),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "api_token_not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "API token not found")
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
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
    }
}
