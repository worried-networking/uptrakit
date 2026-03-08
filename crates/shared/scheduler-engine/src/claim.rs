use rootcause::prelude::*;
use sea_orm::{ActiveEnum, ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter};
use strum::IntoEnumIterator;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::scheduled_task;
use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
use uuid::Uuid;

use crate::error::{self, SchedulerError};

/// Duration after which a locked task is considered stale and can be reclaimed
/// (10 minutes).
///
/// This is intentionally much shorter than [`super::TASK_EXECUTION_TIMEOUT`]
/// (2 hours). If a controller crashes mid-execution, we want another instance
/// to pick up the abandoned task within minutes rather than waiting for the
/// full execution timeout. Running tasks check their own cancellation token
/// against `TASK_EXECUTION_TIMEOUT` independently.
const STALE_CLAIM_SECONDS: i64 = 600;

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
///
/// Returns due tasks across **all tenants**. Each returned row carries its own
/// `tenant_id`; executors that need it read it directly from the task model.
///
/// Only tasks with a known [`ScheduledTaskType`] variant are returned. Rows in the
/// database with an unrecognised `task_type` string (added during a rolling upgrade
/// where a newer controller instance created a task this instance doesn't know about)
/// are silently skipped rather than causing a deserialization failure.
pub async fn find_due_tasks(db: &DatabaseConnection) -> error::Result<Vec<scheduled_task::Model>> {
    let now = OffsetDateTime::now_utc();
    let known_types: Vec<String> = ScheduledTaskType::iter().map(|t| t.into_value()).collect();

    scheduled_task::Entity::find()
        .filter(scheduled_task::Column::TaskType.is_in(known_types))
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
    use sea_orm::{ActiveModelTrait, ActiveValue, ConnectOptions, Database};
    use uptrakit_shared_db::entity::{scheduled_task, tenant};

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        db
    }

    async fn seed_tenant(db: &DatabaseConnection) -> tenant::Model {
        use sea_orm::ActiveModelTrait;
        // Create a fresh non-default test tenant. Migrations seed scheduled tasks
        // only for tenants that exist at migration time, so this tenant starts with
        // no pre-seeded tasks — allowing seed_task() to insert freely.
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            name: ActiveValue::Set("Test Tenant".to_string()),
            slug: ActiveValue::Set(format!("test-{}", Uuid::now_v7())),
            is_default: ActiveValue::Set(false),
            deactivated_at: ActiveValue::Set(None),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
        }
        .insert(db)
        .await
        .expect("insert test tenant")
    }

    async fn seed_task(db: &DatabaseConnection, tenant_id: Uuid) -> scheduled_task::Model {
        let now = OffsetDateTime::now_utc();
        scheduled_task::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            tenant_id: ActiveValue::Set(tenant_id),
            task_type: ActiveValue::Set(scheduled_task::ScheduledTaskType::AuthCleanup),
            interval_seconds: ActiveValue::Set(300),
            jitter_seconds: ActiveValue::Set(30),
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
        let task = seed_task(&db, tenant.id).await;

        // find_due_tasks is tenant-agnostic; it may return tasks from other tenants
        // (seeded by migrations). Assert our specific task is present.
        let due = find_due_tasks(&db).await.unwrap();
        assert!(
            due.iter().any(|t| t.id == task.id),
            "seeded due task must be returned"
        );
    }

    #[tokio::test]
    async fn find_due_tasks_excludes_locked() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let task = seed_task(&db, tenant.id).await;

        try_claim(&db, task.id, Uuid::now_v7()).await.unwrap();

        // The locked task must not appear; other seeded tasks may still be present.
        let due = find_due_tasks(&db).await.unwrap();
        assert!(
            !due.iter().any(|t| t.id == task.id),
            "locked task must be excluded from due tasks"
        );
    }

    #[tokio::test]
    async fn find_due_tasks_spans_all_tenants() {
        let db = setup_test_db().await;

        // Seed two independent tenants, each with a due task.
        let tenant_a = seed_tenant(&db).await;
        let tenant_b = seed_tenant(&db).await;
        seed_task(&db, tenant_a.id).await;
        seed_task(&db, tenant_b.id).await;

        let due = find_due_tasks(&db).await.unwrap();
        let tenant_ids: std::collections::HashSet<Uuid> = due.iter().map(|t| t.tenant_id).collect();
        assert!(
            tenant_ids.contains(&tenant_a.id),
            "tenant A task should be returned"
        );
        assert!(
            tenant_ids.contains(&tenant_b.id),
            "tenant B task should be returned"
        );
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
            interval_seconds: ActiveValue::Set(3600),
            jitter_seconds: ActiveValue::Set(0),
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

    /// Rows with an unknown `task_type` string (from a newer controller version) must be
    /// silently excluded rather than causing a deserialization failure.
    #[tokio::test]
    async fn find_due_tasks_excludes_unknown_task_type() {
        use sea_orm::sea_query::{Expr as SqExpr, Query};

        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;

        // Insert a known task that is due.
        let known_task = seed_task(&db, tenant.id).await;

        // Insert an unknown task type — simulates a row created by a newer controller
        // version during a rolling upgrade. Hoisted outside the block so it's visible
        // in the assertions below.
        let unknown_id = Uuid::now_v7();
        {
            let now = OffsetDateTime::now_utc();
            let past = now - time::Duration::minutes(1);

            let insert = Query::insert()
                .into_table(scheduled_task::Entity)
                .columns([
                    scheduled_task::Column::Id,
                    scheduled_task::Column::TenantId,
                    scheduled_task::Column::TaskType,
                    scheduled_task::Column::IntervalSeconds,
                    scheduled_task::Column::JitterSeconds,
                    scheduled_task::Column::Enabled,
                    scheduled_task::Column::TaskConfig,
                    scheduled_task::Column::LastRunAt,
                    scheduled_task::Column::NextRunAt,
                    scheduled_task::Column::LockedBy,
                    scheduled_task::Column::LockedAt,
                    scheduled_task::Column::LastError,
                    scheduled_task::Column::RunCount,
                    scheduled_task::Column::CreatedAt,
                    scheduled_task::Column::UpdatedAt,
                ])
                .values_panic([
                    SqExpr::value(unknown_id),
                    SqExpr::value(tenant.id),
                    SqExpr::value("future_unknown_task_type"),
                    SqExpr::value(300i32),
                    SqExpr::value(30i32),
                    SqExpr::value(true),
                    SqExpr::value(sea_orm::Value::Json(None)),
                    SqExpr::value(sea_orm::Value::TimeDateTimeWithTimeZone(None)),
                    SqExpr::value(past),
                    SqExpr::value(sea_orm::Value::Uuid(None)),
                    SqExpr::value(sea_orm::Value::TimeDateTimeWithTimeZone(None)),
                    SqExpr::value(sea_orm::Value::String(None)),
                    SqExpr::value(0i32),
                    SqExpr::value(now),
                    SqExpr::value(now),
                ])
                .to_owned();

            use sea_orm::ConnectionTrait as _;
            db.execute(&insert).await.expect("insert unknown task type");
        }

        // find_due_tasks is tenant-agnostic; it may also return migration-seeded tasks.
        // Key invariants: the known task must be present, the unknown-type row must not.
        let due = find_due_tasks(&db).await.unwrap();
        assert!(
            due.iter().any(|t| t.id == known_task.id),
            "known due task must be returned"
        );
        assert!(
            !due.iter().any(|t| t.id == unknown_id),
            "row with unknown task type must be excluded"
        );
    }
}
