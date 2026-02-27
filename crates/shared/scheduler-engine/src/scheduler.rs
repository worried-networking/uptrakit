use std::collections::HashMap;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use tokio_util::sync::CancellationToken;
use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
use uuid::Uuid;

use crate::error::SchedulerError;
use crate::executor::TaskExecutor;
use crate::{claim, cron_utils};

/// Default poll interval for the scheduler loop.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 15;

/// Maximum wall-clock time allowed for a single scheduled task execution.
///
/// Matches the wire-protocol update execution timeout (2 hours). Tasks that
/// exceed this limit receive a `SchedulerError::TaskTimedOut` error and their
/// claim is released with the timeout recorded as the last error.
pub const TASK_EXECUTION_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

/// Configuration for the scheduler.
pub struct SchedulerConfig {
    /// How often to poll for due tasks.
    pub poll_interval: Duration,
    /// The controller ID used for claim ownership.
    pub controller_id: Uuid,
    /// The default tenant ID for task queries.
    pub tenant_id: Uuid,
    /// Maximum wall-clock time allowed for a single task execution before it
    /// is considered hung and its claim is released with a timeout error.
    /// Defaults to [`TASK_EXECUTION_TIMEOUT`] (2 hours).
    pub task_execution_timeout: Duration,
}

impl SchedulerConfig {
    pub fn new(controller_id: Uuid, tenant_id: Uuid) -> Self {
        Self {
            poll_interval: Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS),
            controller_id,
            tenant_id,
            task_execution_timeout: TASK_EXECUTION_TIMEOUT,
        }
    }
}

/// Central DB-backed task scheduler.
///
/// Polls the `scheduled_tasks` table for due tasks, claims them via optimistic
/// locking (HA-safe), executes the matching [`TaskExecutor`], and releases the
/// claim with updated metadata.
pub struct Scheduler {
    db: DatabaseConnection,
    config: SchedulerConfig,
    executors: HashMap<ScheduledTaskType, Box<dyn TaskExecutor>>,
}

impl Scheduler {
    pub fn new(db: DatabaseConnection, config: SchedulerConfig) -> Self {
        Self {
            db,
            config,
            executors: HashMap::new(),
        }
    }

    /// Register an executor for a task type.
    pub fn register(&mut self, task_type: ScheduledTaskType, executor: Box<dyn TaskExecutor>) {
        self.executors.insert(task_type, executor);
    }

    /// Run the scheduler loop until the cancellation token is triggered.
    pub async fn run(self, token: CancellationToken) {
        let mut interval = tokio::time::interval(self.config.poll_interval);
        // Skip the first immediate tick
        interval.tick().await;

        tracing::info!(
            controller_id = %self.config.controller_id,
            poll_interval_secs = self.config.poll_interval.as_secs(),
            registered_executors = self.executors.len(),
            "scheduler started"
        );

        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => {
                    tracing::debug!("scheduler shutting down, releasing claims");
                    if let Err(e) = claim::release_all_claims(
                        &self.db,
                        self.config.controller_id,
                    ).await {
                        tracing::warn!(error = %e, "failed to release claims during shutdown");
                    }
                    return;
                }
                _ = interval.tick() => {
                    self.poll_cycle(&token).await;
                }
            }
        }
    }

    /// Single poll cycle: recover stale claims, find due tasks, execute them.
    ///
    /// `token` is checked before and during each task execution. If the token
    /// is cancelled mid-cycle the loop exits early — the per-task timeout
    /// ensures no single executor can block the cancellation check indefinitely.
    async fn poll_cycle(&self, token: &CancellationToken) {
        // Recover stale claims from crashed controllers
        match claim::recover_stale_claims(&self.db).await {
            Ok(recovered) if recovered > 0 => {
                tracing::info!(recovered, "recovered stale task claims");
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to recover stale claims");
            }
            _ => {}
        }

        // Find tasks that are due for execution
        let due_tasks = match claim::find_due_tasks(&self.db, self.config.tenant_id).await {
            Ok(tasks) => tasks,
            Err(e) => {
                tracing::warn!(error = %e, "failed to find due tasks");
                return;
            }
        };

        for task in due_tasks {
            // Exit early if a shutdown was requested while iterating
            if token.is_cancelled() {
                break;
            }

            let Some(executor) = self.executors.get(&task.task_type) else {
                tracing::warn!(
                    task_type = ?task.task_type,
                    "no executor registered for task type"
                );
                continue;
            };

            // Try to claim the task (optimistic lock)
            let claimed = match claim::try_claim(&self.db, task.id, self.config.controller_id).await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        task_id = %task.id,
                        error = %e,
                        "failed to claim task"
                    );
                    continue;
                }
            };

            if !claimed {
                // Another controller claimed it first — skip
                continue;
            }

            tracing::debug!(
                task_id = %task.id,
                task_type = ?task.task_type,
                "executing scheduled task"
            );

            // Execute with per-task timeout and cancellation awareness.
            // `biased` gives the cancellation branch higher priority so that a
            // pending shutdown is honoured before the timeout branch fires.
            let result: crate::error::Result<()> = tokio::select! {
                biased;
                _ = token.cancelled() => {
                    tracing::debug!(
                        task_id = %task.id,
                        task_type = ?task.task_type,
                        "scheduler shutdown requested during task execution"
                    );
                    break;
                }
                res = tokio::time::timeout(
                    self.config.task_execution_timeout,
                    executor.execute(&task),
                ) => {
                    res.unwrap_or_else(|_| {
                        tracing::warn!(
                            task_id = %task.id,
                            task_type = ?task.task_type,
                            timeout_secs = self.config.task_execution_timeout.as_secs(),
                            "scheduled task timed out"
                        );
                        bail!(SchedulerError::TaskTimedOut(task.task_type))
                    })
                }
            };

            if let Err(ref e) = result {
                tracing::warn!(
                    task_id = %task.id,
                    task_type = ?task.task_type,
                    error = %e,
                    "scheduled task failed"
                );
            }

            // Compute the next run time from the cron expression
            let now = time::OffsetDateTime::now_utc();
            let next_run_at = cron_utils::next_run_after(&task.cron_expression, now)
                .unwrap_or_else(|| now + time::Duration::hours(1));

            // Convert typed error to string for DB storage (last_error column is Option<String>).
            let db_result = result.as_ref().map(|_| ()).map_err(|e| e.to_string());

            if let Err(e) = claim::release_claim(&self.db, task.id, next_run_at, &db_result).await {
                tracing::warn!(
                    task_id = %task.id,
                    error = %e,
                    "failed to release task claim"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveModelTrait, ActiveValue, ConnectOptions, ConnectionTrait, Database, EntityTrait,
        Schema,
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
        let now = time::OffsetDateTime::now_utc();
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

    #[test]
    fn scheduler_config_new_sets_default_poll_interval() {
        let controller_id = Uuid::now_v7();
        let tenant_id = Uuid::now_v7();
        let config = SchedulerConfig::new(controller_id, tenant_id);
        assert_eq!(config.poll_interval, Duration::from_secs(15));
        assert_eq!(config.controller_id, controller_id);
        assert_eq!(config.tenant_id, tenant_id);
        assert_eq!(config.task_execution_timeout, TASK_EXECUTION_TIMEOUT);
    }

    #[tokio::test]
    async fn scheduler_poll_cycle_empty_db_leaves_no_locked_tasks() {
        let db = setup_test_db().await;
        let config = SchedulerConfig::new(Uuid::now_v7(), Uuid::now_v7());
        let scheduler = Scheduler::new(db.clone(), config);
        let token = CancellationToken::new();

        scheduler.poll_cycle(&token).await;

        // No tasks should exist at all in the empty DB
        let all_tasks = scheduled_task::Entity::find()
            .all(&db)
            .await
            .expect("query");
        assert!(all_tasks.is_empty(), "empty DB should have no tasks");
    }

    #[tokio::test]
    async fn scheduler_run_exits_on_cancellation() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let controller_id = Uuid::now_v7();
        let config = SchedulerConfig {
            poll_interval: Duration::from_millis(50),
            controller_id,
            tenant_id: tenant.id,
            task_execution_timeout: TASK_EXECUTION_TIMEOUT,
        };
        let scheduler = Scheduler::new(db.clone(), config);

        let token = CancellationToken::new();
        let token_clone = token.clone();

        // Cancel after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            token_clone.cancel();
        });

        // Must complete within a reasonable timeout after cancellation
        let result = tokio::time::timeout(Duration::from_secs(5), scheduler.run(token)).await;
        assert!(
            result.is_ok(),
            "scheduler.run should exit promptly after cancellation"
        );

        // After shutdown, no tasks should be locked by this controller
        let all_tasks = scheduled_task::Entity::find()
            .all(&db)
            .await
            .expect("query");
        for task in &all_tasks {
            assert_ne!(
                task.locked_by,
                Some(controller_id),
                "all claims should be released on shutdown"
            );
        }
    }

    #[tokio::test]
    async fn scheduler_poll_cycle_with_no_due_tasks() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;

        // Seed a task whose next_run_at is in the future (not yet due)
        let now = time::OffsetDateTime::now_utc();
        let task_id = Uuid::now_v7();
        scheduled_task::ActiveModel {
            id: ActiveValue::Set(task_id),
            tenant_id: ActiveValue::Set(tenant.id),
            task_type: ActiveValue::Set(ScheduledTaskType::AuthCleanup),
            cron_expression: ActiveValue::Set("*/5 * * * *".to_string()),
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
        .expect("insert task");

        let config = SchedulerConfig::new(Uuid::now_v7(), tenant.id);
        let scheduler = Scheduler::new(db.clone(), config);

        let token = CancellationToken::new();
        scheduler.poll_cycle(&token).await;

        // Task should remain untouched: not locked, run_count still 0
        let task = scheduled_task::Entity::find_by_id(task_id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert!(
            task.locked_by.is_none(),
            "future task should not be claimed"
        );
        assert_eq!(
            task.run_count, 0,
            "future task should not have been executed"
        );
    }

    #[tokio::test]
    async fn scheduler_poll_cycle_skips_unregistered_task_type() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;

        // Seed a due task but don't register its executor
        let now = time::OffsetDateTime::now_utc();
        let task_id = Uuid::now_v7();
        scheduled_task::ActiveModel {
            id: ActiveValue::Set(task_id),
            tenant_id: ActiveValue::Set(tenant.id),
            task_type: ActiveValue::Set(ScheduledTaskType::AuthCleanup),
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
        .insert(&db)
        .await
        .expect("insert task");

        let config = SchedulerConfig::new(Uuid::now_v7(), tenant.id);
        let scheduler = Scheduler::new(db.clone(), config);
        let token = CancellationToken::new();

        scheduler.poll_cycle(&token).await;

        // Task should NOT be claimed or executed when no executor is registered
        let task = scheduled_task::Entity::find_by_id(task_id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert!(
            task.locked_by.is_none(),
            "task with no executor should not be claimed"
        );
        assert_eq!(
            task.run_count, 0,
            "task with no executor should not be executed"
        );
    }

    #[tokio::test]
    async fn scheduler_poll_cycle_executes_registered_task() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;

        let now = time::OffsetDateTime::now_utc();
        let task = scheduled_task::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            tenant_id: ActiveValue::Set(tenant.id),
            task_type: ActiveValue::Set(ScheduledTaskType::StaleLeaseCleanup),
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
        .insert(&db)
        .await
        .expect("insert task");

        let executed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let executed_clone = executed.clone();

        struct TrackingExecutor(std::sync::Arc<std::sync::atomic::AtomicBool>);
        #[async_trait::async_trait]
        impl TaskExecutor for TrackingExecutor {
            async fn execute(
                &self,
                _task: &scheduled_task::Model,
            ) -> crate::error::Result<()> {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }

        let config = SchedulerConfig::new(Uuid::now_v7(), tenant.id);
        let mut scheduler = Scheduler::new(db.clone(), config);
        scheduler.register(
            ScheduledTaskType::StaleLeaseCleanup,
            Box::new(TrackingExecutor(executed_clone)),
        );
        let token = CancellationToken::new();

        scheduler.poll_cycle(&token).await;

        assert!(executed.load(std::sync::atomic::Ordering::SeqCst));

        // Task should be released (unlocked) and run_count incremented
        let updated = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert!(updated.locked_by.is_none());
        assert_eq!(updated.run_count, 1);
    }
}
