pub mod claim;
pub mod cron_utils;
pub mod error;
pub mod executor;
pub mod executors;

use std::collections::HashMap;
use std::time::Duration;

use sea_orm::DatabaseConnection;
use tokio_util::sync::CancellationToken;
use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
use uuid::Uuid;

use crate::scheduler::executor::TaskExecutor;

/// Default poll interval for the scheduler loop.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 15;

/// Configuration for the scheduler.
pub struct SchedulerConfig {
    /// How often to poll for due tasks.
    pub poll_interval: Duration,
    /// The controller ID used for claim ownership.
    pub controller_id: Uuid,
    /// The default tenant ID for task queries.
    pub tenant_id: Uuid,
}

impl SchedulerConfig {
    pub fn new(controller_id: Uuid, tenant_id: Uuid) -> Self {
        Self {
            poll_interval: Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS),
            controller_id,
            tenant_id,
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
                _ = interval.tick() => {
                    self.poll_cycle().await;
                }
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
            }
        }
    }

    /// Single poll cycle: recover stale claims, find due tasks, execute them.
    async fn poll_cycle(&self) {
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

            let result = executor.execute(&task).await;

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
