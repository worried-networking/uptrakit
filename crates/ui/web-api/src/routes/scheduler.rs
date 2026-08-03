use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use http::StatusCode;
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use uptrakit_audit_log::{AuditEntry, AuditOutcome, Event, Stateful};
use uuid::Uuid;

use crate::AppState;
use crate::api_error::ApiError;
use crate::app_state::AuditEmitterState;
use crate::error_response::error_response;
use crate::middleware::action::CanManageScheduler;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::scheduled_tasks::{self as sched_queries, ScheduledTaskView};
use crate::tenant_db::TenantDb;
use uptrakit_web_api_types::validation::Validate;

pub use uptrakit_web_api_types::scheduler::{
    ScheduledTaskResponse, TriggerScheduledTaskResponse, UpdateScheduledTaskRequest,
};

struct AuditContext<'a> {
    audit_emitter: &'a uptrakit_audit_log::AuditEmitter,
    tenant_id: Uuid,
    user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
}

fn emit_scheduled_task_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_task_id: Uuid,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(action_type)
            .tenant_scope(ctx.tenant_id)
            .actor(actor_type, actor_id)
            .target("scheduled_task", target_task_id.to_string(), target_display)
            .outcome(outcome)
            .details(details)
            .build()
    {
        ctx.audit_emitter.emit_event(entry);
    }
}

/// List all scheduled tasks for the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/scheduler/tasks",
    tag = "Scheduler",
    responses(
        (status = 200, description = "Scheduled tasks", body = Vec<ScheduledTaskResponse>),
        (status = 403, description = "Not authorized")
    ),
    security(("oauth2" = ["scheduler:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_scheduled_tasks(
    tenant_db: TenantDb,
    CanManageScheduler(_user): CanManageScheduler,
) -> Response {
    match sched_queries::list_scheduled_tasks(&tenant_db).await {
        Ok(tasks) => Json(tasks).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to list scheduled tasks");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list tasks")
        }
    }
}

/// Get a single scheduled task by ID.
#[utoipa::path(
    get,
    path = "/api/v1/scheduler/tasks/{id}",
    tag = "Scheduler",
    params(
        ("id" = Uuid, Path, description = "Task UUID")
    ),
    responses(
        (status = 200, description = "Scheduled task", body = ScheduledTaskResponse),
        (status = 404, description = "Task not found"),
        (status = 403, description = "Not authorized")
    ),
    security(("oauth2" = ["scheduler:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_scheduled_task(
    tenant_db: TenantDb,
    CanManageScheduler(_user): CanManageScheduler,
    Path(task_id): Path<Uuid>,
) -> Response {
    match sched_queries::get_scheduled_task(&tenant_db, task_id).await {
        Ok(Some(task)) => Json(task).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Task not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to get scheduled task");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to get task")
        }
    }
}

/// Update a scheduled task (interval, jitter, enabled, config).
#[utoipa::path(
    put,
    path = "/api/v1/scheduler/tasks/{id}",
    tag = "Scheduler",
    params(
        ("id" = Uuid, Path, description = "Task UUID")
    ),
    request_body = UpdateScheduledTaskRequest,
    responses(
        (status = 200, description = "Updated task", body = ScheduledTaskResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Task not found"),
        (status = 403, description = "Not authorized")
    ),
    security(("oauth2" = ["scheduler:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_scheduled_task(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageScheduler(caller): CanManageScheduler,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(task_id): Path<Uuid>,
    Json(req): Json<UpdateScheduledTaskRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    if req.validate().is_err() {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::SCHEDULED_TASK_UPDATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("scheduled_task", task_id.to_string(), None)
        .outcome(AuditOutcome::ValidationFailed)
        .details(serde_json::json!({ "reason_code": "invalid_request" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return Err(ApiError::from(rootcause::report!(
            sched_queries::ScheduledTaskError::InvalidInterval
        )));
    }

    let requested_interval = req.interval_seconds;
    let requested_jitter = req.jitter_seconds;
    let requested_enabled = req.enabled;
    let requested_config = req.task_config.clone();

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
            tracing::error!(error = %e, "failed to begin transaction for scheduled task update");
            return Err(ApiError::from(rootcause::report!(
                sched_queries::ScheduledTaskError::Db(e)
            )));
        }
    };

    let pair = match sched_queries::update_scheduled_task_in_tx(&tx, tenant_id, task_id, req).await
    {
        Ok(Some(pair)) => pair,
        Ok(None) => {
            drop(tx);
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::SCHEDULED_TASK_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("scheduled_task", task_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "reason_code": sched_queries::ScheduledTaskError::NotFound.reason_code(),
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return Err(ApiError::from(rootcause::report!(
                sched_queries::ScheduledTaskError::NotFound
            )));
        }
        Err(e) => {
            drop(tx);
            let outcome = e.current_context().audit_outcome();
            let reason_code = e.current_context().reason_code();
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::SCHEDULED_TASK_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("scheduled_task", task_id.to_string(), None)
            .outcome(outcome)
            .details(serde_json::json!({ "reason_code": reason_code }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return Err(e.into());
        }
    };

    let (before_model, after_model) = pair;
    let before_view = ScheduledTaskView::from(&before_model);
    let after_view = ScheduledTaskView::from(&after_model);

    let mut changed_fields: Vec<&str> = Vec::new();
    if requested_interval.is_some() {
        changed_fields.push("interval_seconds");
    }
    if requested_jitter.is_some() {
        changed_fields.push("jitter_seconds");
    }
    if requested_enabled.is_some() {
        changed_fields.push("enabled");
    }
    if requested_config.is_some() {
        changed_fields.push("task_config");
    }

    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::scheduled_task_update(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "task_type": after_view.task_type,
            "interval_seconds": after_model.interval_seconds,
            "jitter_seconds": after_model.jitter_seconds,
            "enabled": after_model.enabled,
            "task_config_present": after_model.task_config.is_some(),
            "changed_fields": changed_fields,
        }))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "failed to build audit entry for scheduled task update");
            drop(tx);
            return Err(ApiError::from(rootcause::report!(
                sched_queries::ScheduledTaskError::Db(sea_orm::DbErr::Custom(e.to_string()))
            )));
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "failed to emit stateful audit entry for scheduled task update");
        drop(tx);
        return Err(ApiError::from(rootcause::report!(
            sched_queries::ScheduledTaskError::Db(sea_orm::DbErr::Custom(e.to_string()))
        )));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "failed to commit scheduled task update transaction");
        return Err(ApiError::from(rootcause::report!(
            sched_queries::ScheduledTaskError::Db(e)
        )));
    }
    hook.flush_after_commit().await;

    Ok(Json(sched_queries::model_to_response(&after_model)).into_response())
}

/// Trigger immediate execution of a scheduled task.
#[utoipa::path(
    post,
    path = "/api/v1/scheduler/tasks/{id}/trigger",
    tag = "Scheduler",
    params(
        ("id" = Uuid, Path, description = "Task UUID")
    ),
    responses(
        (status = 200, description = "Trigger result", body = TriggerScheduledTaskResponse),
        (status = 404, description = "Task not found"),
        (status = 403, description = "Not authorized")
    ),
    security(("oauth2" = ["scheduler:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn trigger_scheduled_task(
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanManageScheduler(caller): CanManageScheduler,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(task_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id(),
        user: &caller,
        api_token_id,
    };
    match sched_queries::trigger_scheduled_task(&tenant_db, task_id).await {
        Ok(true) => {
            emit_scheduled_task_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SCHEDULED_TASK_TRIGGER,
                task_id,
                None,
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "triggered": true,
                }),
            );
            Json(TriggerScheduledTaskResponse {
                triggered: true,
                message: "Task will execute on next scheduler poll".to_string(),
            })
            .into_response()
        }
        Ok(false) => {
            emit_scheduled_task_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SCHEDULED_TASK_TRIGGER,
                task_id,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "scheduled_task_not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "Task not found")
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to trigger scheduled task");
            emit_scheduled_task_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SCHEDULED_TASK_TRIGGER,
                task_id,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "scheduled_task_database_error",
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to trigger task")
        }
    }
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
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{audit_log, scheduled_task};

    async fn latest_scheduled_task_audit_row_for_target(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
        target_task_id: Uuid,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::TenantId.is_not_null())
                .filter(audit_log::Column::ActionType.eq(action_type))
                .filter(audit_log::Column::TargetType.eq("scheduled_task"))
                .filter(audit_log::Column::TargetId.eq(target_task_id.to_string()))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected scheduled task audit row");
    }

    async fn insert_scheduled_task(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
    ) -> scheduled_task::Model {
        let now = OffsetDateTime::now_utc();
        scheduled_task::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            task_type: Set(scheduled_task::ScheduledTaskType::FetchReleases),
            interval_seconds: Set(3600),
            jitter_seconds: Set(30),
            enabled: Set(true),
            task_config: Set(None),
            last_run_at: Set(None),
            next_run_at: Set(now + time::Duration::hours(1)),
            locked_by: Set(None),
            locked_at: Set(None),
            last_error: Set(None),
            run_count: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert scheduled task")
    }

    #[tokio::test]
    async fn update_scheduled_task_writes_update_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let task = insert_scheduled_task(&app.db, app.tenant_id).await;

        let status = client
            .put_json(
                &format!("/api/v1/scheduler/tasks/{}", task.id),
                &UpdateScheduledTaskRequest {
                    interval_seconds: Some(7200),
                    jitter_seconds: Some(60),
                    enabled: Some(false),
                    task_config: None,
                },
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::OK);

        let row = latest_scheduled_task_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::SCHEDULED_TASK_UPDATE,
            task.id,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["task_type"], serde_json::json!("fetch_releases"));
        assert_eq!(details["enabled"], serde_json::json!(false));
        assert_eq!(
            details["changed_fields"],
            serde_json::json!(["interval_seconds", "jitter_seconds", "enabled"])
        );
    }

    #[tokio::test]
    async fn update_scheduled_task_invalid_request_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let task = insert_scheduled_task(&app.db, app.tenant_id).await;

        let status = client
            .put_json(
                &format!("/api/v1/scheduler/tasks/{}", task.id),
                &serde_json::json!({
                    "interval_seconds": 0,
                }),
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = latest_scheduled_task_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::SCHEDULED_TASK_UPDATE,
            task.id,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
    }

    #[tokio::test]
    async fn trigger_scheduled_task_missing_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let missing_id = Uuid::now_v7();

        let status = client
            .post_empty(&format!("/api/v1/scheduler/tasks/{missing_id}/trigger"))
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let row = latest_scheduled_task_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::SCHEDULED_TASK_TRIGGER,
            missing_id,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("scheduled_task_not_found")
        );
    }
}
