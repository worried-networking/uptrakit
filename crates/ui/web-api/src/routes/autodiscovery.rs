//! HTTP route handlers for autodiscovery ignore-list management.
//!
//! Endpoints:
//! - `GET  /api/v1/autodiscovery/ignores`    — list rules
//! - `POST /api/v1/autodiscovery/ignores`    — create rule
//! - `DELETE /api/v1/autodiscovery/ignores/{id}` — remove rule

use std::sync::Arc;

use crate::AppState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::action::{CanManageDiscoveryIgnores, CanReadSoftware};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uuid::Uuid;

pub use uptrakit_web_api_types::autodiscovery::{
    CreateSoftwareIgnoreRequest, SoftwareIgnoreResponse,
};
pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

/// List autodiscovery ignore rules.
#[utoipa::path(
    get,
    path = "/api/v1/autodiscovery/ignores",
    params(ListIgnoresParams),
    responses(
        (status = 200, description = "Paginated list of ignore rules", body = PaginatedResponse<SoftwareIgnoreResponse>),
    ),
    tag = "Autodiscovery",
    security(("oauth2" = ["software:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_autodiscovery_ignores(
    tenant_db: TenantDb,
    CanReadSoftware(_user): CanReadSoftware,
    Query(params): Query<ListIgnoresParams>,
) -> Response {
    let pagination = PaginationParams {
        page: params.page,
        per_page: params.per_page,
    };

    match autodiscovery_queries::list_ignore_rules(
        tenant_db.db(),
        tenant_db.tenant_id(),
        &pagination,
    )
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
    responses(
        (status = 201, description = "Ignore rule created", body = SoftwareIgnoreResponse),
        (status = 200, description = "Ignore rule already exists", body = SoftwareIgnoreResponse),
        (status = 400, description = "Invalid input"),
    ),
    tag = "Autodiscovery",
    security(("oauth2" = ["discovery.ignores:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_autodiscovery_ignore(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageDiscoveryIgnores(user): CanManageDiscoveryIgnores,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<CreateSoftwareIgnoreRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();
    let name = req.name.trim().to_string();

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
            tracing::error!(error = %e, "Failed to begin transaction for software ignore create");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let (was_created, rule) = match autodiscovery_queries::create_or_ignore_ignore_rule_in_tx(
        &tx,
        tenant_id,
        &name,
        req.host_id,
    )
    .await
    {
        Ok(result) => result,
        Err(e) => {
            drop(tx);
            tracing::error!(error = %e, "Failed to create autodiscovery ignore rule");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({
                "reason_code": "software_ignore_create_failed",
                "name": name,
                "host_id": req.host_id,
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if !was_created {
        // Rule already existed — no state change, emit an Event-class entry.
        drop(tx);
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_CREATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target(
            "software_ignore",
            rule.id.to_string(),
            Some(rule.name.clone()),
        )
        .outcome(AuditOutcome::Partial)
        .details(serde_json::json!({
            "name": rule.name,
            "host_id": rule.host_id,
            "was_created": false,
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return (
            StatusCode::OK,
            Json(SoftwareIgnoreResponse {
                id: rule.id,
                name: rule.name,
                host_id: rule.host_id,
                created_at: rule.created_at,
            }),
        )
            .into_response();
    }

    // New rule — emit_stateful so the snapshot is persisted atomically with the row.
    let view = autodiscovery_queries::SoftwareIgnoreView::from(&rule);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::software_ignore_create(
        &AbsentView(&view),
        &view,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "name": rule.name,
        "host_id": rule.host_id,
        "was_created": true,
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for software ignore create");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for software ignore create");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit software ignore create");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    (
        StatusCode::CREATED,
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
    responses(
        (status = 204, description = "Ignore rule deleted"),
        (status = 404, description = "Ignore rule not found")
    ),
    tag = "Autodiscovery",
    security(("oauth2" = ["discovery.ignores:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_autodiscovery_ignore(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageDiscoveryIgnores(user): CanManageDiscoveryIgnores,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(rule_id): Path<Uuid>,
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
            tracing::error!(error = %e, "Failed to begin transaction for software ignore delete");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_model =
        match autodiscovery_queries::delete_ignore_rule_in_tx(&tx, tenant_id, rule_id).await {
            Ok(Some(model)) => model,
            Ok(None) => {
                drop(tx);
                if let Ok(entry) = AuditEntry::<Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target("software_ignore", rule_id.to_string(), None)
                .outcome(AuditOutcome::Denied)
                .details(serde_json::json!({ "reason_code": "software_ignore_not_found" }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::NOT_FOUND, "Ignore rule not found");
            }
            Err(e) => {
                drop(tx);
                tracing::error!(error = %e, "Failed to delete autodiscovery ignore rule");
                if let Ok(entry) = AuditEntry::<Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target("software_ignore", rule_id.to_string(), None)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({ "reason_code": "software_ignore_delete_failed" }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let before_view = autodiscovery_queries::SoftwareIgnoreView::from(&before_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::software_ignore_delete(
        &before_view,
        &AbsentView(&before_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "name": before_model.name,
        "host_id": before_model.host_id,
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for software ignore delete");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for software ignore delete");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit software ignore delete");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
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
    security(("oauth2" = ["discovery.ignores:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_autodiscovery_ignores(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageDiscoveryIgnores(user): CanManageDiscoveryIgnores,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(body): Validated<BatchActionRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let (succeeded_ids, failed) = match body.action.as_str() {
        "delete" => {
            match autodiscovery_queries::batch_delete_ignore_rules(
                tenant_db.db(),
                tenant_id,
                &body.ids,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, "batch delete failed");
                    if let Ok(entry) = AuditEntry::<Event>::builder_event(
                        uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
                    )
                    .tenant_scope(tenant_id)
                    .actor(actor_type, actor_id)
                    .outcome(AuditOutcome::Failed)
                    .details(serde_json::json!({
                        "batch": true,
                        "reason_code": "batch_delete_failed",
                        "requested_count": body.ids.len(),
                    }))
                    .build()
                    {
                        state.audit_emitter.emit_event(entry);
                    }
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        }
        unknown => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({
                "batch": true,
                "reason_code": "unknown_action",
                "action": unknown,
                "requested_count": body.ids.len(),
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: delete"),
            );
        }
    };

    if let Ok(entry) = AuditEntry::<Event>::builder_event(
        uptrakit_audit_log::AuditActionType::SOFTWARE_IGNORE_DELETE,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(if failed.is_empty() {
        AuditOutcome::Success
    } else if succeeded_ids.is_empty() {
        AuditOutcome::Denied
    } else {
        AuditOutcome::Partial
    })
    .details(serde_json::json!({
        "batch": true,
        "requested_count": body.ids.len(),
        "succeeded_count": succeeded_ids.len(),
        "failed_count": failed.len(),
    }))
    .build()
    {
        state.audit_emitter.emit_event(entry);
    }

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

#[derive(serde::Deserialize, Default, utoipa::IntoParams)]
pub struct ListIgnoresParams {
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures::register_and_get_token;
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
    async fn delete_ignore_db_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
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
            serde_json::json!("software_ignore_delete_failed")
        );
    }

    #[tokio::test]
    async fn delete_ignore_delete_db_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
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
