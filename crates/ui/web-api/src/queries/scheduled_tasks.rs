use rootcause::prelude::*;
use sea_orm::{
    ActiveEnum, ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter,
    sea_query::Expr,
};
use std::str::FromStr;
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::scheduled_task;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_types::scheduler::{ScheduledTaskResponse, UpdateScheduledTaskRequest};
use uuid::Uuid;

use crate::tenant_db::TenantDb;

/// Error returned by scheduled task query helpers.
#[derive(Debug, Error)]
pub enum ScheduledTaskError {
    /// The requested task does not exist for this tenant.
    #[error("scheduled task not found")]
    NotFound,
    /// The provided cron expression could not be parsed.
    #[error("invalid cron expression")]
    InvalidCronExpression,
    /// A database error occurred.
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<ScheduledTaskError>>;
impl_report_conversion!(sea_orm::DbErr => ScheduledTaskError::Db);

// --- Private helpers ---

fn model_to_response(m: &scheduled_task::Model) -> ScheduledTaskResponse {
    ScheduledTaskResponse {
        id: m.id,
        task_type: m.task_type.to_value().to_string(),
        label: m.task_type.label().to_string(),
        cron_expression: m.cron_expression.clone(),
        enabled: m.enabled,
        task_config: m.task_config.clone(),
        last_run_at: m.last_run_at,
        next_run_at: m.next_run_at,
        is_running: m.locked_by.is_some(),
        last_error: m.last_error.clone(),
        run_count: m.run_count,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

/// Normalize a 5-field standard cron expression to 6-field (the `cron` crate requires seconds).
fn normalize_cron(expr: &str) -> String {
    if expr.split_whitespace().count() == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    }
}

fn compute_next_run(cron_expr: &str, after: OffsetDateTime) -> Option<OffsetDateTime> {
    let normalized = normalize_cron(cron_expr);
    let schedule = cron::Schedule::from_str(&normalized).ok()?;
    let after_chrono = chrono::DateTime::from_timestamp(after.unix_timestamp(), 0)?;
    let next_chrono = schedule.after(&after_chrono).next()?;
    OffsetDateTime::from_unix_timestamp(next_chrono.timestamp()).ok()
}

// --- Public query functions ---

pub async fn list_scheduled_tasks(tenant_db: &TenantDb) -> Result<Vec<ScheduledTaskResponse>> {
    let tasks = tenant_db
        .find::<scheduled_task::Entity>()
        .all(tenant_db.db())
        .await
        .context_to()?;
    Ok(tasks.iter().map(model_to_response).collect())
}

/// Returns `None` if the task is not found.
pub async fn get_scheduled_task(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<ScheduledTaskResponse>> {
    let task = tenant_db
        .find_by_id::<scheduled_task::Entity, _>(id)
        .one(tenant_db.db())
        .await
        .context_to()?;
    Ok(task.as_ref().map(model_to_response))
}

/// Update a scheduled task.
pub async fn update_scheduled_task(
    tenant_db: &TenantDb,
    id: Uuid,
    req: UpdateScheduledTaskRequest,
) -> Result<ScheduledTaskResponse> {
    let task = tenant_db
        .find_by_id::<scheduled_task::Entity, _>(id)
        .one(tenant_db.db())
        .await
        .context_to()?;

    let task = task.ok_or_else(|| report!(ScheduledTaskError::NotFound))?;

    let mut active: scheduled_task::ActiveModel = task.into();
    let now = OffsetDateTime::now_utc();

    if let Some(ref cron_expr) = req.cron_expression {
        let normalized = normalize_cron(cron_expr);
        if cron::Schedule::from_str(&normalized).is_err() {
            bail!(ScheduledTaskError::InvalidCronExpression);
        }
        active.cron_expression = ActiveValue::Set(cron_expr.clone());
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

    let updated = active.update(tenant_db.db()).await.context_to()?;

    Ok(model_to_response(&updated))
}

/// Force immediate execution by setting `next_run_at` to now.
/// Returns `true` if the task was found and updated, `false` if not found.
pub async fn trigger_scheduled_task(tenant_db: &TenantDb, id: Uuid) -> Result<bool> {
    // Verify task exists for this tenant before issuing the bulk update.
    if tenant_db
        .find_by_id::<scheduled_task::Entity, _>(id)
        .one(tenant_db.db())
        .await
        .context_to()?
        .is_none()
    {
        return Ok(false);
    }

    let now = OffsetDateTime::now_utc();
    let result = scheduled_task::Entity::update_many()
        .col_expr(scheduled_task::Column::NextRunAt, Expr::value(now))
        .col_expr(scheduled_task::Column::UpdatedAt, Expr::value(now))
        .filter(scheduled_task::Column::Id.eq(id))
        .exec(tenant_db.db())
        .await
        .context_to()?;

    Ok(result.rows_affected == 1)
}
