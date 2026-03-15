use axum::Json;
use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use uuid::Uuid;

use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanManageScheduler;
use crate::queries::scheduled_tasks::{self as sched_queries, ScheduledTaskError};
use crate::tenant_db::TenantDb;

pub use uptrakit_web_api_types::scheduler::{
    ScheduledTaskResponse, TriggerScheduledTaskResponse, UpdateScheduledTaskRequest,
};

/// List all scheduled tasks for the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/scheduler/tasks",
    tag = "Scheduler",
    responses(
        (status = 200, description = "Scheduled tasks", body = Vec<ScheduledTaskResponse>),
        (status = 403, description = "Not authorized")
    ),
    extensions(("x-required-permission" = json!("manage_scheduler"))),
    security(("bearer_token" = []))
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
    extensions(("x-required-permission" = json!("manage_scheduler"))),
    security(("bearer_token" = []))
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
    extensions(("x-required-permission" = json!("manage_scheduler"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_scheduled_task(
    tenant_db: TenantDb,
    CanManageScheduler(_user): CanManageScheduler,
    Path(task_id): Path<Uuid>,
    Validated(req): Validated<UpdateScheduledTaskRequest>,
) -> Response {
    match sched_queries::update_scheduled_task(&tenant_db, task_id, req).await {
        Ok(task) => Json(task).into_response(),
        Err(report) => match report.current_context() {
            ScheduledTaskError::NotFound => error_response(StatusCode::NOT_FOUND, "Task not found"),
            ScheduledTaskError::InvalidInterval => {
                error_response(StatusCode::BAD_REQUEST, "Invalid interval")
            }
            ScheduledTaskError::Db(_) => {
                tracing::error!(error = %report, "failed to update scheduled task");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to update task")
            }
        },
    }
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
    extensions(("x-required-permission" = json!("manage_scheduler"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn trigger_scheduled_task(
    tenant_db: TenantDb,
    CanManageScheduler(_user): CanManageScheduler,
    Path(task_id): Path<Uuid>,
) -> Response {
    match sched_queries::trigger_scheduled_task(&tenant_db, task_id).await {
        Ok(true) => Json(TriggerScheduledTaskResponse {
            triggered: true,
            message: "Task will execute on next scheduler poll".to_string(),
        })
        .into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Task not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to trigger scheduled task");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to trigger task")
        }
    }
}
