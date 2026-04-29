//! HTTP route handlers for autodiscovery ignore-list management.
//!
//! Endpoints:
//! - `GET  /api/v1/autodiscovery/ignores`    — list rules
//! - `POST /api/v1/autodiscovery/ignores`    — create rule
//! - `DELETE /api/v1/autodiscovery/ignores/{id}` — remove rule

use crate::app_state::AuditEmitterState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{CanManageIgnores, CanViewSoftware};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

pub use uptrakit_web_api_types::autodiscovery::{
    CreateSoftwareIgnoreRequest, SoftwareIgnoreResponse,
};
pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

struct AuditContext<'a> {
    tenant_id: Uuid,
    user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
}

fn emit_software_ignore_audit(
    audit_emitter: &uptrakit_audit_log::AuditEmitter,
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_rule_id: Uuid,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id, actor_display) =
        authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .actor_display_opt(actor_display)
        .target(
            "software_ignore",
            target_rule_id.to_string(),
            target_display,
        )
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        audit_emitter.emit_best_effort(entry);
    }
}

fn emit_software_ignore_batch_audit(
    audit_emitter: &uptrakit_audit_log::AuditEmitter,
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id, actor_display) =
        authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .actor_display_opt(actor_display)
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        audit_emitter.emit_best_effort(entry);
    }
}

/// List autodiscovery ignore rules.
#[utoipa::path(
    get,
    path = "/api/v1/autodiscovery/ignores",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
    ),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Paginated list of ignore rules", body = PaginatedResponse<SoftwareIgnoreResponse>),
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_autodiscovery_ignores(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(params): Query<ListIgnoresParams>,
) -> Response {
    let pagination = PaginationParams {
        page: params.page,
        per_page: params.per_page,
    };

    match autodiscovery_queries::list_ignore_rules(tenant_db.db(), tenant_db.tenant_id, &pagination)
        .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list autodiscovery ignore rules");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Create an autodiscovery ignore rule.
///
/// Idempotent — if the rule already exists, returns the existing rule.
#[utoipa::path(
    post,
    path = "/api/v1/autodiscovery/ignores",
    request_body = CreateSoftwareIgnoreRequest,
    extensions(("x-required-permission" = json!("manage_ignores"))),
    responses(
        (status = 201, description = "Ignore rule created", body = SoftwareIgnoreResponse),
        (status = 200, description = "Ignore rule already exists", body = SoftwareIgnoreResponse),
        (status = 400, description = "Invalid input"),
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_autodiscovery_ignore(
    State(audit): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanManageIgnores(user): CanManageIgnores,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<CreateSoftwareIgnoreRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use uptrakit_shared_db::entity::{prelude::*, software_ignore};

    let name = req.name.trim().to_string();

    // Create the rule (idempotent). Returns true if newly inserted, false if already existed.
    let was_created = match autodiscovery_queries::create_or_ignore_ignore_rule(
        tenant_db.db(),
        tenant_db.tenant_id,
        &name,
        req.host_id,
    )
    .await
    {
        Ok(created) => created,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create autodiscovery ignore rule");
            emit_software_ignore_audit(
                &audit.0,
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_CREATE,
                Uuid::nil(),
                Some(name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_ignore_create_failed",
                    "name": name,
                    "host_id": req.host_id,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Fetch the current rule to get the correct ID and created_at.
    let mut query = SoftwareIgnore::find()
        .filter(software_ignore::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_ignore::Column::Name.eq(&name));
    if let Some(host_id) = req.host_id {
        query = query.filter(software_ignore::Column::HostId.eq(host_id));
    } else {
        query = query.filter(software_ignore::Column::HostId.is_null());
    }
    let rule = match query.one(tenant_db.db()).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            emit_software_ignore_audit(
                &audit.0,
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_CREATE,
                Uuid::nil(),
                Some(name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_ignore_lookup_failed",
                    "name": name,
                    "host_id": req.host_id,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to load autodiscovery ignore rule after create");
            emit_software_ignore_audit(
                &audit.0,
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_CREATE,
                Uuid::nil(),
                Some(name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_ignore_lookup_failed",
                    "name": name,
                    "host_id": req.host_id,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let status = if was_created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    let outcome = if was_created {
        uptrakit_audit_log::AuditOutcome::Success
    } else {
        uptrakit_audit_log::AuditOutcome::Partial
    };
    emit_software_ignore_audit(
        &audit.0,
        &audit_ctx,
        uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_CREATE,
        rule.id,
        Some(rule.name.clone()),
        outcome,
        serde_json::json!({
            "name": rule.name,
            "host_id": rule.host_id,
            "was_created": was_created,
        }),
    );

    (
        status,
        Json(SoftwareIgnoreResponse {
            id: rule.id,
            name: rule.name,
            host_id: rule.host_id,
            created_at: rule.created_at,
        }),
    )
        .into_response()
}

/// Delete an autodiscovery ignore rule.
#[utoipa::path(
    delete,
    path = "/api/v1/autodiscovery/ignores/{id}",
    params(("id" = Uuid, Path, description = "Ignore rule UUID")),
    extensions(("x-required-permission" = json!("manage_ignores"))),
    responses(
        (status = 204, description = "Ignore rule deleted"),
        (status = 404, description = "Ignore rule not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_autodiscovery_ignore(
    State(audit): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanManageIgnores(user): CanManageIgnores,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(rule_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use uptrakit_shared_db::entity::{prelude::*, software_ignore};

    let existing_rule = match SoftwareIgnore::find_by_id(rule_id)
        .filter(software_ignore::Column::TenantId.eq(tenant_db.tenant_id))
        .one(tenant_db.db())
        .await
    {
        Ok(rule) => rule,
        Err(e) => {
            tracing::error!(error = %e, "Failed to load autodiscovery ignore rule before delete");
            emit_software_ignore_audit(
                &audit.0,
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
                rule_id,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_ignore_lookup_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match autodiscovery_queries::delete_ignore_rule(tenant_db.db(), tenant_db.tenant_id, rule_id)
        .await
    {
        Ok(true) => {
            emit_software_ignore_audit(
                &audit.0,
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
                rule_id,
                existing_rule.as_ref().map(|rule| rule.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "name": existing_rule.as_ref().map(|rule| rule.name.clone()),
                    "host_id": existing_rule.as_ref().and_then(|rule| rule.host_id),
                }),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            emit_software_ignore_audit(
                &audit.0,
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
                rule_id,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "software_ignore_not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "Ignore rule not found")
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to delete autodiscovery ignore rule");
            emit_software_ignore_audit(
                &audit.0,
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
                rule_id,
                existing_rule.as_ref().map(|rule| rule.name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_ignore_delete_failed",
                    "name": existing_rule.as_ref().map(|rule| rule.name.clone()),
                    "host_id": existing_rule.as_ref().and_then(|rule| rule.host_id),
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Perform a batch action on multiple autodiscovery ignore rules.
///
/// Supported actions: `delete`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/autodiscovery/ignores/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Autodiscovery",
    extensions(("x-required-permission" = json!("manage_ignores"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_autodiscovery_ignores(
    State(audit): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanManageIgnores(user): CanManageIgnores,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(body): Validated<BatchActionRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let (succeeded_ids, failed) = match body.action.as_str() {
        "delete" => {
            match autodiscovery_queries::batch_delete_ignore_rules(
                tenant_db.db(),
                tenant_db.tenant_id,
                &body.ids,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, "batch delete failed");
                    emit_software_ignore_batch_audit(
                        &audit.0,
                        &audit_ctx,
                        uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
                        uptrakit_audit_log::AuditOutcome::Failed,
                        serde_json::json!({
                            "batch": true,
                            "reason_code": "batch_delete_failed",
                            "requested_count": body.ids.len(),
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        }
        unknown => {
            emit_software_ignore_batch_audit(
                &audit.0,
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "batch": true,
                    "reason_code": "unknown_action",
                    "action": unknown,
                    "requested_count": body.ids.len(),
                }),
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: delete"),
            );
        }
    };

    emit_software_ignore_batch_audit(
        &audit.0,
        &audit_ctx,
        uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        if failed.is_empty() {
            uptrakit_audit_log::AuditOutcome::Success
        } else if succeeded_ids.is_empty() {
            uptrakit_audit_log::AuditOutcome::Denied
        } else {
            uptrakit_audit_log::AuditOutcome::Partial
        },
        serde_json::json!({
            "batch": true,
            "requested_count": body.ids.len(),
            "succeeded_count": succeeded_ids.len(),
            "failed_count": failed.len(),
        }),
    );

    let response = BatchActionResponse {
        succeeded: succeeded_ids
            .into_iter()
            .map(|id| BatchActionSuccess { id })
            .collect(),
        failed: failed
            .into_iter()
            .map(|(id, error)| BatchActionFailure { id, error })
            .collect(),
    };

    (StatusCode::OK, Json(response)).into_response()
}

#[derive(serde::Deserialize, Default)]
pub struct ListIgnoresParams {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures::{register_and_get_token, seed_permissions_for_owner};
    use http::StatusCode;
    use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
    use serde_json::Value;
    use uptrakit_shared_db::entity::audit_log;

    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
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
    async fn create_ignore_writes_software_ignore_create_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        let (status, body): (StatusCode, Value) = client
            .post_json(
                "/api/v1/autodiscovery/ignores",
                &serde_json::json!({
                    "name": "Ignored App",
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, StatusCode::CREATED);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_ignore"));
        assert_eq!(
            row.target_id.as_deref(),
            body["id"].as_str(),
            "audit target_id should match created rule id",
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["name"], serde_json::json!("Ignored App"));
        assert_eq!(details["host_id"], serde_json::Value::Null);
        assert_eq!(details["was_created"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn delete_ignore_writes_software_ignore_delete_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        let (create_status, create_body): (StatusCode, Value) = client
            .post_json(
                "/api/v1/autodiscovery/ignores",
                &serde_json::json!({
                    "name": "To Delete",
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(create_status, StatusCode::CREATED);
        let rule_id = create_body["id"]
            .as_str()
            .expect("create response should contain id");

        let delete_status = client
            .delete(&format!("/api/v1/autodiscovery/ignores/{rule_id}"))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(delete_status, StatusCode::NO_CONTENT);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_ignore"));
        assert_eq!(row.target_id.as_deref(), Some(rule_id));
        let details = row.details_json.expect("details");
        assert_eq!(details["name"], serde_json::json!("To Delete"));
        assert_eq!(details["host_id"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn create_ignore_db_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        app.db
            .execute_unprepared("DROP TABLE software_ignores")
            .await
            .expect("drop software_ignores table");

        let status = client
            .post_json(
                "/api/v1/autodiscovery/ignores",
                &serde_json::json!({
                    "name": "Ignored App",
                }),
            )
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_ignore_create_failed")
        );
    }

    #[tokio::test]
    async fn delete_missing_ignore_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        let status = client
            .delete(&format!(
                "/api/v1/autodiscovery/ignores/{}",
                uuid::Uuid::new_v4()
            ))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_ignore_not_found")
        );
    }

    #[tokio::test]
    async fn delete_ignore_lookup_db_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        app.db
            .execute_unprepared("DROP TABLE software_ignores")
            .await
            .expect("drop software_ignores table");

        let status = client
            .delete(&format!(
                "/api/v1/autodiscovery/ignores/{}",
                uuid::Uuid::new_v4()
            ))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_ignore_lookup_failed")
        );
    }

    #[tokio::test]
    async fn delete_ignore_delete_db_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        let (create_status, create_body): (StatusCode, Value) = client
            .post_json(
                "/api/v1/autodiscovery/ignores",
                &serde_json::json!({
                    "name": "Can Not Delete",
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(create_status, StatusCode::CREATED);
        let rule_id = create_body["id"]
            .as_str()
            .expect("create response should contain id");

        app.db
            .execute_unprepared(
                "CREATE TRIGGER software_ignore_delete_block BEFORE DELETE ON software_ignores BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
            )
            .await
            .expect("install delete-block trigger");

        let status = client
            .delete(&format!("/api/v1/autodiscovery/ignores/{rule_id}"))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_ignore_delete_failed")
        );
    }

    #[tokio::test]
    async fn batch_ignores_invalid_action_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        let status = client
            .post_json(
                "/api/v1/autodiscovery/ignores/batch",
                &serde_json::json!({
                    "action": "noop",
                    "ids": [uuid::Uuid::new_v4()],
                }),
            )
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["batch"], serde_json::json!(true));
        assert_eq!(details["action"], serde_json::json!("noop"));
        assert_eq!(details["reason_code"], serde_json::json!("unknown_action"));
    }

    #[tokio::test]
    async fn batch_ignores_backend_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        app.db
            .execute_unprepared("DROP TABLE software_ignores")
            .await
            .expect("drop software_ignores table");

        let status = client
            .post_json(
                "/api/v1/autodiscovery/ignores/batch",
                &serde_json::json!({
                    "action": "delete",
                    "ids": [uuid::Uuid::new_v4()],
                }),
            )
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["batch"], serde_json::json!(true));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("batch_delete_failed")
        );
    }

    #[tokio::test]
    async fn batch_ignores_success_writes_success_summary_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        let (_, body_a): (StatusCode, Value) = client
            .post_json(
                "/api/v1/autodiscovery/ignores",
                &serde_json::json!({ "name": "Batch A" }),
            )
            .bearer(&token)
            .send_json()
            .await;
        let id_a = body_a["id"].as_str().expect("rule id A");

        let (_, body_b): (StatusCode, Value) = client
            .post_json(
                "/api/v1/autodiscovery/ignores",
                &serde_json::json!({ "name": "Batch B" }),
            )
            .bearer(&token)
            .send_json()
            .await;
        let id_b = body_b["id"].as_str().expect("rule id B");

        let (status, response): (StatusCode, Value) = client
            .post_json(
                "/api/v1/autodiscovery/ignores/batch",
                &serde_json::json!({
                    "action": "delete",
                    "ids": [id_a, id_b],
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["succeeded"].as_array().map(Vec::len), Some(2));
        assert_eq!(response["failed"].as_array().map(Vec::len), Some(0));

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["batch"], serde_json::json!(true));
        assert_eq!(details["requested_count"], serde_json::json!(2));
        assert_eq!(details["succeeded_count"], serde_json::json!(2));
        assert_eq!(details["failed_count"], serde_json::json!(0));
    }

    #[tokio::test]
    async fn batch_ignores_partial_writes_partial_summary_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        let (_, body): (StatusCode, Value) = client
            .post_json(
                "/api/v1/autodiscovery/ignores",
                &serde_json::json!({ "name": "Batch Partial" }),
            )
            .bearer(&token)
            .send_json()
            .await;
        let id = body["id"].as_str().expect("rule id");

        let (status, response): (StatusCode, Value) = client
            .post_json(
                "/api/v1/autodiscovery/ignores/batch",
                &serde_json::json!({
                    "action": "delete",
                    "ids": [id, uuid::Uuid::new_v4()],
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["succeeded"].as_array().map(Vec::len), Some(1));
        assert_eq!(response["failed"].as_array().map(Vec::len), Some(1));

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Partial.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["batch"], serde_json::json!(true));
        assert_eq!(details["requested_count"], serde_json::json!(2));
        assert_eq!(details["succeeded_count"], serde_json::json!(1));
        assert_eq!(details["failed_count"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn batch_ignores_all_failures_write_denied_summary_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["manage_ignores"]).await;
        let token = register_and_get_token(&client).await;

        let (status, response): (StatusCode, Value) = client
            .post_json(
                "/api/v1/autodiscovery/ignores/batch",
                &serde_json::json!({
                    "action": "delete",
                    "ids": [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()],
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["succeeded"].as_array().map(Vec::len), Some(0));
        assert_eq!(response["failed"].as_array().map(Vec::len), Some(2));

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["batch"], serde_json::json!(true));
        assert_eq!(details["requested_count"], serde_json::json!(2));
        assert_eq!(details["succeeded_count"], serde_json::json!(0));
        assert_eq!(details["failed_count"], serde_json::json!(2));
    }
}
