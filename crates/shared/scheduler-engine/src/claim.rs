use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::scheduled_task;
use uuid::Uuid;

use crate::error::{self, SchedulerError};

/// Duration after which a locked task is considered stale and can be reclaimed.
const STALE_CLAIM_SECONDS: i64 = 600; // 10 minutes

/// Attempt to claim a task for execution via optimistic locking.
///
/// Returns `true` if the claim was acquired (exactly one row updated).
pub async fn try_claim(
    db: &DatabaseConnection,
    task_id: Uuid,
    controller_id: Uuid,
) -> error::Result<bool> {
    let now = OffsetDateTime::now_utc();

    let result = scheduled_task::Entity::update_many()
        .col_expr(
            scheduled_task::Column::LockedBy,
            sea_orm::sea_query::Expr::value(controller_id),
        )
        .col_expr(
            scheduled_task::Column::LockedAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .filter(scheduled_task::Column::Id.eq(task_id))
        .filter(scheduled_task::Column::LockedBy.is_null())
        .exec(db)
        .await
        .context_to::<SchedulerError>()?;

    Ok(result.rows_affected == 1)
}

/// Release a task claim after execution, updating run metadata.
///
/// The `result` parameter is `Result<(), String>` because the `last_error` DB column
/// is `Option<String>`. The caller converts typed errors to strings before calling.
pub async fn release_claim(
    db: &DatabaseConnection,
    task_id: Uuid,
    next_run_at: OffsetDateTime,
    result: &Result<(), String>,
) -> error::Result<()> {
    let now = OffsetDateTime::now_utc();
    let last_error = match result {
        Ok(()) => None,
        Err(e) => Some(e.clone()),
    };

    let mut update = scheduled_task::Entity::update_many()
        .col_expr(
            scheduled_task::Column::LockedBy,
            sea_orm::sea_query::Expr::value(Option::<Uuid>::None),
        )
        .col_expr(
            scheduled_task::Column::LockedAt,
            sea_orm::sea_query::Expr::value(Option::<OffsetDateTime>::None),
        )
        .col_expr(
            scheduled_task::Column::NextRunAt,
            sea_orm::sea_query::Expr::value(next_run_at),
        )
        .col_expr(
            scheduled_task::Column::UpdatedAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .col_expr(
            scheduled_task::Column::LastError,
            sea_orm::sea_query::Expr::value(last_error),
        );

    if result.is_ok() {
        update = update
            .col_expr(
                scheduled_task::Column::LastRunAt,
                sea_orm::sea_query::Expr::value(now),
            )
            .col_expr(
                scheduled_task::Column::RunCount,
                sea_orm::sea_query::Expr::col(scheduled_task::Column::RunCount)
                    .add(sea_orm::sea_query::Expr::value(1i64)),
            );
    }

    update
        .filter(scheduled_task::Column::Id.eq(task_id))
        .exec(db)
        .await
        .context_to::<SchedulerError>()?;

    Ok(())
}

/// Find and release stale task claims (locked longer than the timeout).
pub async fn recover_stale_claims(db: &DatabaseConnection) -> error::Result<u64> {
    let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(STALE_CLAIM_SECONDS);

    let result = scheduled_task::Entity::update_many()
        .col_expr(
            scheduled_task::Column::LockedBy,
            sea_orm::sea_query::Expr::value(Option::<Uuid>::None),
        )
        .col_expr(
            scheduled_task::Column::LockedAt,
            sea_orm::sea_query::Expr::value(Option::<OffsetDateTime>::None),
        )
        .col_expr(
            scheduled_task::Column::LastError,
            sea_orm::sea_query::Expr::value(Some(
                "released: stale claim (controller may have crashed)".to_string(),
            )),
        )
        .filter(scheduled_task::Column::LockedAt.is_not_null())
        .filter(scheduled_task::Column::LockedAt.lt(cutoff))
        .exec(db)
        .await
        .context_to::<SchedulerError>()?;

    Ok(result.rows_affected)
}

/// Release all claims held by a specific controller (used during shutdown).
pub async fn release_all_claims(
    db: &DatabaseConnection,
    controller_id: Uuid,
) -> error::Result<u64> {
    let result = scheduled_task::Entity::update_many()
        .col_expr(
            scheduled_task::Column::LockedBy,
            sea_orm::sea_query::Expr::value(Option::<Uuid>::None),
        )
        .col_expr(
            scheduled_task::Column::LockedAt,
            sea_orm::sea_query::Expr::value(Option::<OffsetDateTime>::None),
        )
        .filter(scheduled_task::Column::LockedBy.eq(controller_id))
        .exec(db)
        .await
        .context_to::<SchedulerError>()?;

    Ok(result.rows_affected)
}

/// Find tasks that are due for execution (enabled, unlocked, next_run_at <= now).
pub async fn find_due_tasks(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> error::Result<Vec<scheduled_task::Model>> {
    let now = OffsetDateTime::now_utc();

    scheduled_task::Entity::find()
        .filter(scheduled_task::Column::TenantId.eq(tenant_id))
        .filter(scheduled_task::Column::Enabled.eq(true))
        .filter(scheduled_task::Column::LockedBy.is_null())
        .filter(scheduled_task::Column::NextRunAt.lte(now))
        .all(db)
        .await
        .context_to::<SchedulerError>()
}

/// Set `next_run_at` to now for immediate execution on next poll cycle.
#[cfg(test)]
pub async fn trigger_immediate(db: &DatabaseConnection, task_id: Uuid) -> error::Result<bool> {
    let now = OffsetDateTime::now_utc();

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
        .exec(db)
        .await
        .context_to::<SchedulerError>()?;

    Ok(result.rows_affected == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveModelTrait, ActiveValue, ConnectOptions, ConnectionTrait, Database, Schema,
    };
    use uptrakit_shared_db::entity::{scheduled_task, tenant};

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");

        let schema = Schema::new(db.get_database_backend());
        let stmt = schema.create_table_from_entity(tenant::Entity);
        db.execute(&stmt).await.expect("create tenants table");
        let stmt = schema.create_table_from_entity(scheduled_task::Entity);
        db.execute(&stmt)
            .await
            .expect("create scheduled_tasks table");
        db
    }

    async fn seed_tenant(db: &DatabaseConnection) -> tenant::Model {
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            name: ActiveValue::Set("Default".to_string()),
            slug: ActiveValue::Set("default".to_string()),
            is_default: ActiveValue::Set(true),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
            deactivated_at: ActiveValue::Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant")
    }

    async fn seed_task(db: &DatabaseConnection, tenant_id: Uuid) -> scheduled_task::Model {
        let now = OffsetDateTime::now_utc();
        scheduled_task::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            tenant_id: ActiveValue::Set(tenant_id),
            task_type: ActiveValue::Set(scheduled_task::ScheduledTaskType::AuthCleanup),
            cron_expression: ActiveValue::Set("*/5 * * * *".to_string()),
            enabled: ActiveValue::Set(true),
            task_config: ActiveValue::Set(None),
            last_run_at: ActiveValue::Set(None),
            next_run_at: ActiveValue::Set(now - time::Duration::minutes(1)),
            locked_by: ActiveValue::Set(None),
            locked_at: ActiveValue::Set(None),
            last_error: ActiveValue::Set(None),
            run_count: ActiveValue::Set(0),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
        }
        .insert(db)
        .await
        .expect("insert task")
    }

    #[tokio::test]
    async fn try_claim_succeeds_for_unclaimed_task() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let task = seed_task(&db, tenant.id).await;
        let controller_id = Uuid::now_v7();

        let claimed = try_claim(&db, task.id, controller_id).await.unwrap();
        assert!(claimed);

        let updated = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.locked_by, Some(controller_id));
        assert!(updated.locked_at.is_some());
    }

    #[tokio::test]
    async fn try_claim_fails_for_already_claimed_task() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let task = seed_task(&db, tenant.id).await;

        let first = Uuid::now_v7();
        let second = Uuid::now_v7();

        assert!(try_claim(&db, task.id, first).await.unwrap());
        assert!(!try_claim(&db, task.id, second).await.unwrap());
    }

    #[tokio::test]
    async fn release_claim_clears_lock_and_updates_metadata() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let task = seed_task(&db, tenant.id).await;
        let controller_id = Uuid::now_v7();

        try_claim(&db, task.id, controller_id).await.unwrap();

        let next = OffsetDateTime::now_utc() + time::Duration::minutes(5);
        release_claim(&db, task.id, next, &Ok(())).await.unwrap();

        let updated = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(updated.locked_by.is_none());
        assert!(updated.locked_at.is_none());
        assert!(updated.last_run_at.is_some());
        assert_eq!(updated.run_count, 1);
        assert!(updated.last_error.is_none());
    }

    #[tokio::test]
    async fn release_claim_records_error() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let task = seed_task(&db, tenant.id).await;
        let controller_id = Uuid::now_v7();

        try_claim(&db, task.id, controller_id).await.unwrap();

        let next = OffsetDateTime::now_utc() + time::Duration::minutes(5);
        let err = Err("test error".to_string());
        release_claim(&db, task.id, next, &err).await.unwrap();

        let updated = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(updated.locked_by.is_none());
        assert_eq!(updated.last_error.as_deref(), Some("test error"));
        assert_eq!(updated.run_count, 0); // Not incremented on failure
    }

    #[tokio::test]
    async fn find_due_tasks_returns_eligible() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let _task = seed_task(&db, tenant.id).await;

        let due = find_due_tasks(&db, tenant.id).await.unwrap();
        assert_eq!(due.len(), 1);
    }

    #[tokio::test]
    async fn find_due_tasks_excludes_locked() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let task = seed_task(&db, tenant.id).await;

        try_claim(&db, task.id, Uuid::now_v7()).await.unwrap();

        let due = find_due_tasks(&db, tenant.id).await.unwrap();
        assert!(due.is_empty());
    }

    #[tokio::test]
    async fn release_all_claims_for_controller() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let task = seed_task(&db, tenant.id).await;
        let controller_id = Uuid::now_v7();

        try_claim(&db, task.id, controller_id).await.unwrap();

        let released = release_all_claims(&db, controller_id).await.unwrap();
        assert_eq!(released, 1);

        let updated = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(updated.locked_by.is_none());
    }

    #[tokio::test]
    async fn trigger_immediate_sets_next_run_to_now() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;

        // Create a task with next_run_at in the future
        let now = OffsetDateTime::now_utc();
        let task = scheduled_task::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            tenant_id: ActiveValue::Set(tenant.id),
            task_type: ActiveValue::Set(scheduled_task::ScheduledTaskType::StaleLeaseCleanup),
            cron_expression: ActiveValue::Set("0 * * * *".to_string()),
            enabled: ActiveValue::Set(true),
            task_config: ActiveValue::Set(None),
            last_run_at: ActiveValue::Set(None),
            next_run_at: ActiveValue::Set(now + time::Duration::hours(1)),
            locked_by: ActiveValue::Set(None),
            locked_at: ActiveValue::Set(None),
            last_error: ActiveValue::Set(None),
            run_count: ActiveValue::Set(0),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
        }
        .insert(&db)
        .await
        .unwrap();

        let triggered = trigger_immediate(&db, task.id).await.unwrap();
        assert!(triggered);

        let updated = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        // next_run_at should now be close to `now`
        assert!(updated.next_run_at <= OffsetDateTime::now_utc());
    }
}
