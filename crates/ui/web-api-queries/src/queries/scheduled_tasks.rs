use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr,
};
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
    /// The provided interval is invalid.
    #[error("invalid interval")]
    InvalidInterval,
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
        task_type: sea_orm::ActiveEnum::to_value(&m.task_type).to_string(),
        label: m.task_type.label().to_string(),
        interval_seconds: m.interval_seconds,
        jitter_seconds: m.jitter_seconds,
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

/// Compute the next run time inline (same logic as `uptrakit_scheduler_engine::interval`
/// but avoids a crate dependency for a single addition).
fn compute_next_run_at(
    now: OffsetDateTime,
    interval_seconds: i32,
    jitter_seconds: i32,
) -> OffsetDateTime {
    let jitter = if jitter_seconds > 0 {
        rand::Rng::random_range(&mut rand::rng(), 0..=jitter_seconds)
    } else {
        0
    };
    now + time::Duration::seconds(i64::from(interval_seconds) + i64::from(jitter))
}

// --- Public query functions ---

#[tracing::instrument(skip_all)]
pub async fn list_scheduled_tasks(tenant_db: &TenantDb) -> Result<Vec<ScheduledTaskResponse>> {
    let tasks = tenant_db
        .find::<scheduled_task::Entity>()
        .all(tenant_db.db())
        .await
        .context_to()?;
    Ok(tasks.iter().map(model_to_response).collect())
}

/// Returns `None` if the task is not found.
#[tracing::instrument(skip_all)]
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
#[tracing::instrument(skip_all)]
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

    let mut interval = task.interval_seconds;
    let mut jitter = task.jitter_seconds;

    let mut active: scheduled_task::ActiveModel = task.into();
    let now = OffsetDateTime::now_utc();

    if let Some(new_interval) = req.interval_seconds {
        if new_interval <= 0 {
            bail!(ScheduledTaskError::InvalidInterval);
        }
        interval = new_interval;
        active.interval_seconds = ActiveValue::Set(new_interval);
    }

    if let Some(new_jitter) = req.jitter_seconds {
        if new_jitter < 0 {
            bail!(ScheduledTaskError::InvalidInterval);
        }
        jitter = new_jitter;
        active.jitter_seconds = ActiveValue::Set(new_jitter);
    }

    // Recompute next_run_at if interval or jitter changed.
    if req.interval_seconds.is_some() || req.jitter_seconds.is_some() {
        active.next_run_at = ActiveValue::Set(compute_next_run_at(now, interval, jitter));
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
#[tracing::instrument(skip_all)]
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
        .filter(scheduled_task::Column::TenantId.eq(tenant_db.tenant_id))
        .exec(tenant_db.db())
        .await
        .context_to()?;

    Ok(result.rows_affected == 1)
}
