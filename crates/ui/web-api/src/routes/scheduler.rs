use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use http::StatusCode;
use sea_orm::{ActiveEnum, ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use uptrakit_shared_db::entity::scheduled_task;

use crate::AppState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;

pub use uptrakit_web_api_types::scheduler::{
    ScheduledTaskResponse, TriggerScheduledTaskResponse, UpdateScheduledTaskRequest,
};

fn model_to_response(m: &scheduled_task::Model) -> ScheduledTaskResponse {
    ScheduledTaskResponse {
        id: m.id,
        task_type: m.task_type.to_value().to_string(),
        label: m.task_type.label().to_string(),
        cron_expression: m.cron_expression.clone(),
        enabled: m.enabled,
        task_config: m.task_config.clone(),
        last_run_at: m.last_run_at.and_then(|t| t.format(&Rfc3339).ok()),
        next_run_at: m.next_run_at.format(&Rfc3339).unwrap_or_default(),
        is_running: m.locked_by.is_some(),
        last_error: m.last_error.clone(),
        run_count: m.run_count,
        created_at: m.created_at.format(&Rfc3339).unwrap_or_default(),
        updated_at: m.updated_at.format(&Rfc3339).unwrap_or_default(),
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
    security(("bearer_token" = []))
)]
pub async fn list_scheduled_tasks(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let tasks = match scheduled_task::Entity::find()
        .filter(scheduled_task::Column::TenantId.eq(state.default_tenant_id))
        .all(&state.db)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to list scheduled tasks");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list tasks");
        }
    };

    let response: Vec<ScheduledTaskResponse> = tasks.iter().map(model_to_response).collect();
    Json(response).into_response()
}

/// Get a single scheduled task by ID.
#[utoipa::path(
    get,
    path = "/api/v1/scheduler/tasks/{id}",
    tag = "Scheduler",
    params(
        ("id" = String, Path, description = "Task UUID")
    ),
    responses(
        (status = 200, description = "Scheduled task", body = ScheduledTaskResponse),
        (status = 404, description = "Task not found"),
        (status = 403, description = "Not authorized")
    ),
    security(("bearer_token" = []))
)]
pub async fn get_scheduled_task(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let task_id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid task ID"),
    };

    let task = match scheduled_task::Entity::find_by_id(task_id)
        .filter(scheduled_task::Column::TenantId.eq(state.default_tenant_id))
        .one(&state.db)
        .await
    {
        Ok(Some(t)) => t,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Task not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to get scheduled task");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to get task");
        }
    };

    Json(model_to_response(&task)).into_response()
}

/// Update a scheduled task (cron expression, enabled, config).
#[utoipa::path(
    put,
    path = "/api/v1/scheduler/tasks/{id}",
    tag = "Scheduler",
    params(
        ("id" = String, Path, description = "Task UUID")
    ),
    request_body = UpdateScheduledTaskRequest,
    responses(
        (status = 200, description = "Updated task", body = ScheduledTaskResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Task not found"),
        (status = 403, description = "Not authorized")
    ),
    security(("bearer_token" = []))
)]
pub async fn update_scheduled_task(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    Json(req): Json<UpdateScheduledTaskRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let task_id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid task ID"),
    };

    let task = match scheduled_task::Entity::find_by_id(task_id)
        .filter(scheduled_task::Column::TenantId.eq(state.default_tenant_id))
        .one(&state.db)
        .await
    {
        Ok(Some(t)) => t,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Task not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to find scheduled task for update");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to find task");
        }
    };

    let mut active: scheduled_task::ActiveModel = task.into();
    let now = time::OffsetDateTime::now_utc();

    if let Some(ref cron_expr) = req.cron_expression {
        // Validate cron expression by parsing (normalize 5-field to 6-field)
        let normalized = normalize_cron(cron_expr);
        if cron::Schedule::from_str(&normalized).is_err() {
            return error_response(StatusCode::BAD_REQUEST, "Invalid cron expression");
        }
        active.cron_expression = ActiveValue::Set(cron_expr.clone());

        // Recompute next_run_at from the new expression
        if let Some(next) = compute_next_run(cron_expr, now) {
            active.next_run_at = ActiveValue::Set(next);
        }
    }

    if let Some(enabled) = req.enabled {
        active.enabled = ActiveValue::Set(enabled);
    }

    if let Some(ref config) = req.task_config {
        if config.is_null() {
            active.task_config = ActiveValue::Set(None);
        } else {
            active.task_config = ActiveValue::Set(Some(config.clone()));
        }
    }

    active.updated_at = ActiveValue::Set(now);

    let updated = match active.update(&state.db).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "failed to update scheduled task");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to update task");
        }
    };

    Json(model_to_response(&updated)).into_response()
}

/// Trigger immediate execution of a scheduled task.
#[utoipa::path(
    post,
    path = "/api/v1/scheduler/tasks/{id}/trigger",
    tag = "Scheduler",
    params(
        ("id" = String, Path, description = "Task UUID")
    ),
    responses(
        (status = 200, description = "Trigger result", body = TriggerScheduledTaskResponse),
        (status = 404, description = "Task not found"),
        (status = 403, description = "Not authorized")
    ),
    security(("bearer_token" = []))
)]
pub async fn trigger_scheduled_task(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let task_id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid task ID"),
    };

    // Verify task exists for this tenant
    match scheduled_task::Entity::find_by_id(task_id)
        .filter(scheduled_task::Column::TenantId.eq(state.default_tenant_id))
        .one(&state.db)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Task not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to find task for trigger");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to find task");
        }
    }

    let now = time::OffsetDateTime::now_utc();
    let result = scheduled_task::Entity::update_many()
        .col_expr(
            scheduled_task::Column::NextRunAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .col_expr(
            scheduled_task::Column::UpdatedAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .filter(scheduled_task::Column::Id.eq(task_id))
        .exec(&state.db)
        .await;

    match result {
        Ok(r) => {
            let triggered = r.rows_affected == 1;
            Json(TriggerScheduledTaskResponse {
                triggered,
                message: if triggered {
                    "Task will execute on next scheduler poll".to_string()
                } else {
                    "Task not found or already running".to_string()
                },
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to trigger scheduled task");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to trigger task")
        }
    }
}

/// Normalize 5-field standard cron to 6-field (the `cron` crate requires seconds).
fn normalize_cron(expr: &str) -> String {
    if expr.split_whitespace().count() == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    }
}

fn compute_next_run(cron_expr: &str, after: time::OffsetDateTime) -> Option<time::OffsetDateTime> {
    let normalized = normalize_cron(cron_expr);
    let schedule = cron::Schedule::from_str(&normalized).ok()?;
    let after_chrono = chrono::DateTime::from_timestamp(after.unix_timestamp(), 0)?;
    let next_chrono = schedule.after(&after_chrono).next()?;
    time::OffsetDateTime::from_unix_timestamp(next_chrono.timestamp()).ok()
}

use std::str::FromStr;
