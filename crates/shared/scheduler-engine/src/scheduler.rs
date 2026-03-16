use std::collections::HashMap;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
use uuid::Uuid;

use crate::error::SchedulerError;
use crate::executor::TaskExecutor;
use crate::{claim, interval};

/// Default poll interval for the scheduler loop (15 seconds).
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(15);

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
    /// Maximum wall-clock time allowed for a single task execution before it
    /// is considered hung and its claim is released with a timeout error.
    /// Defaults to [`TASK_EXECUTION_TIMEOUT`] (2 hours).
    pub task_execution_timeout: Duration,
}

impl SchedulerConfig {
    pub fn new(controller_id: Uuid) -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            controller_id,
            task_execution_timeout: TASK_EXECUTION_TIMEOUT,
        }
    }
}

/// Central DB-backed task scheduler.
///
/// Polls the `scheduled_tasks` table for due tasks, claims them via optimistic
/// locking (HA-safe), executes the matching [`TaskExecutor`], and releases the
/// claim with updated metadata.
///
/// Within each poll cycle, all claimed tasks are spawned concurrently into a
/// [`JoinSet`] so that a slow executor cannot block other due tasks.
pub struct Scheduler {
    db: DatabaseConnection,
    config: SchedulerConfig,
    executors: HashMap<ScheduledTaskType, std::sync::Arc<dyn TaskExecutor>>,
    /// Closure that returns `true` when non-internal tasks should be deferred
    /// (e.g. because an external scheduler with overlapping capabilities is
    /// connected). Internal tasks (CRL renewal, CA rotation check, service
    /// cert check) always run regardless of the return value.
    should_yield_external: Box<dyn Fn() -> bool + Send + Sync>,
}

impl Scheduler {
    pub fn new(
        db: DatabaseConnection,
        config: SchedulerConfig,
        should_yield_external: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        Self {
            db,
            config,
            executors: HashMap::new(),
            should_yield_external,
        }
    }

    /// Register an executor for a task type.
    ///
    /// # Panics (debug builds only)
    ///
    /// Panics if an executor for `task_type` is already registered. This
    /// catches accidental double-registration during development without any
    /// runtime cost in release builds.
    pub fn register(&mut self, task_type: ScheduledTaskType, executor: Box<dyn TaskExecutor>) {
        debug_assert!(
            !self.executors.contains_key(&task_type),
            "BUG: executor for {task_type:?} is already registered; double-registration detected"
        );
        self.executors
            .insert(task_type, std::sync::Arc::from(executor));
    }

    /// Run the scheduler loop until a cancellation token is triggered.
    ///
    /// Two tokens control shutdown behaviour:
    ///
    /// - `drain` — soft stop: the loop exits after the current poll cycle
    ///   completes; in-flight tasks are allowed to finish naturally.
    /// - `abort` — hard stop: in-flight tasks receive the abort signal and
    ///   release their claims before terminating.
    ///
    /// `abort` is checked first (biased) so that a simultaneous hard-stop
    /// always wins over a pending soft-stop.
    pub async fn run(self, drain: CancellationToken, abort: CancellationToken) {
        let mut interval = tokio::time::interval(self.config.poll_interval);
        // Skip the first immediate tick
        interval.tick().await;

        tracing::info!(
            controller_id = %self.config.controller_id,
            poll_interval_secs = self.config.poll_interval.as_secs(),
            registered_executors = self.executors.len(),
            yielding_external = (self.should_yield_external)(),
            "scheduler started"
        );

        loop {
            tokio::select! {
                biased;
                _ = abort.cancelled() => {
                    tracing::debug!("scheduler hard-abort, releasing claims");
                    if let Err(e) = claim::release_all_claims(
                        &self.db,
                        self.config.controller_id,
                    ).await {
                        tracing::warn!(error = %e, "failed to release claims during hard-abort");
                    }
                    return;
                }
                _ = drain.cancelled() => {
                    tracing::debug!("scheduler draining, stopping new tasks");
                    if let Err(e) = claim::release_all_claims(
                        &self.db,
                        self.config.controller_id,
                    ).await {
                        tracing::warn!(error = %e, "failed to release claims during drain");
                    }
                    return;
                }
                _ = interval.tick() => {
                    self.poll_cycle(&drain, &abort).await;
                }
            }
        }
    }

    /// Single poll cycle: recover stale claims, find due tasks, execute them in parallel.
    ///
    /// Tasks are claimed sequentially (fast, avoids double-claim races), then each
    /// claimed task is spawned into a [`JoinSet`] for concurrent execution. The
    /// JoinSet is drained before returning so that all in-flight tasks complete
    /// (or release their claims) before the next poll tick.
    ///
    /// `drain` is checked before each claim attempt — if cancelled, no new tasks
    /// are claimed but already-running tasks continue to completion. `abort` is
    /// passed into each spawned task and triggers immediate claim release when fired.
    #[tracing::instrument(skip_all, fields(controller_id = %self.config.controller_id))]
    async fn poll_cycle(&self, drain: &CancellationToken, abort: &CancellationToken) {
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

        // Find tasks that are due for execution (across all tenants)
        let due_tasks = match claim::find_due_tasks(&self.db).await {
            Ok(tasks) => tasks,
            Err(e) => {
                tracing::warn!(error = %e, "failed to find due tasks");
                return;
            }
        };

        let mut join_set: JoinSet<()> = JoinSet::new();

        for task in due_tasks {
            // Stop claiming new tasks if a drain or abort was requested.
            if drain.is_cancelled() || abort.is_cancelled() {
                break;
            }

            // Defer non-internal tasks when a yield condition is active (e.g.
            // an external scheduler with overlapping capabilities is connected).
            if (self.should_yield_external)() && !task.task_type.is_internal() {
                tracing::debug!(
                    task_type = ?task.task_type,
                    "skipping external task (yield condition active)"
                );
                continue;
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
                "spawning scheduled task"
            );

            let db = self.db.clone();
            let executor = executor.clone();
            let abort = abort.clone();
            let timeout = self.config.task_execution_timeout;

            join_set.spawn(async move {
                // Execute with per-task timeout and hard-abort awareness.
                // `biased` gives the abort branch higher priority so that a
                // hard-stop is honoured before the timeout branch fires.
                // Drain (soft stop) does not interrupt running tasks — they
                // complete naturally and release their claims normally.
                let result: crate::error::Result<()> = tokio::select! {
                    biased;
                    _ = abort.cancelled() => {
                        tracing::debug!(
                            task_id = %task.id,
                            task_type = ?task.task_type,
                            "scheduler hard-abort during task execution; releasing claim"
                        );
                        // Release the claim so other scheduler instances can pick
                        // up the task immediately rather than waiting for the
                        // stale-claim recovery window (up to 10 minutes).
                        let now = time::OffsetDateTime::now_utc();
                        let next_run_at = interval::compute_next_run_at(now, task.interval_seconds, task.jitter_seconds);
                        if let Err(e) = claim::release_claim(
                            &db,
                            task.id,
                            next_run_at,
                            &Err("scheduler shutdown during execution".to_string()),
                        )
                        .await
                        {
                            tracing::warn!(
                                task_id = %task.id,
                                error = %e,
                                "failed to release task claim on scheduler shutdown"
                            );
                        }
                        return;
                    }
                    res = tokio::time::timeout(timeout, executor.execute(&task)) => {
                        res.unwrap_or_else(|_| {
                            tracing::warn!(
                                task_id = %task.id,
                                task_type = ?task.task_type,
                                timeout_secs = timeout.as_secs(),
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

                // Compute the next run time from the interval + jitter
                let now = time::OffsetDateTime::now_utc();
                let next_run_at = interval::compute_next_run_at(now, task.interval_seconds, task.jitter_seconds);

                // Convert typed error to string for DB storage.
                let db_result = result.as_ref().map(|_| ()).map_err(|e| e.to_string());

                if let Err(e) = claim::release_claim(&db, task.id, next_run_at, &db_result).await {
                    tracing::warn!(
                        task_id = %task.id,
                        error = %e,
                        "failed to release task claim"
                    );
                }
            });
        }

        // Drain all in-flight tasks before returning to the poll loop.
        while let Some(res) = join_set.join_next().await {
            if let Err(e) = res {
                tracing::warn!(error = ?e, "scheduled task execution panicked");
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
        let config = SchedulerConfig::new(controller_id);
        assert_eq!(config.poll_interval, Duration::from_secs(15));
        assert_eq!(config.controller_id, controller_id);
        assert_eq!(config.task_execution_timeout, TASK_EXECUTION_TIMEOUT);
    }

    #[tokio::test]
    async fn scheduler_poll_cycle_empty_db_leaves_no_locked_tasks() {
        let db = setup_test_db().await;
        let config = SchedulerConfig::new(Uuid::now_v7());
        let scheduler = Scheduler::new(db.clone(), config, Box::new(|| false));
        let token = CancellationToken::new();

        scheduler.poll_cycle(&token, &token).await;

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
        let controller_id = Uuid::now_v7();
        let config = SchedulerConfig {
            poll_interval: Duration::from_millis(50),
            controller_id,
            task_execution_timeout: TASK_EXECUTION_TIMEOUT,
        };
        let scheduler = Scheduler::new(db.clone(), config, Box::new(|| false));

        let token = CancellationToken::new();

        // Cancel before run() — CancellationToken::cancel() is synchronous and idempotent;
        // if the token is already cancelled when scheduler.run() enters its select! loop, the
        // cancelled branch fires immediately on the first poll. No real sleep needed.
        token.cancel();

        // Must complete within a reasonable timeout after cancellation.
        // Pass the same token as both drain and abort — the abort branch fires
        // first (biased), which is correct for this test.
        let result =
            tokio::time::timeout(Duration::from_secs(5), scheduler.run(token.clone(), token)).await;
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
            interval_seconds: ActiveValue::Set(300),
            jitter_seconds: ActiveValue::Set(30),
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

        let config = SchedulerConfig::new(Uuid::now_v7());
        let scheduler = Scheduler::new(db.clone(), config, Box::new(|| false));

        let token = CancellationToken::new();
        scheduler.poll_cycle(&token, &token).await;

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
        .insert(&db)
        .await
        .expect("insert task");

        let config = SchedulerConfig::new(Uuid::now_v7());
        let scheduler = Scheduler::new(db.clone(), config, Box::new(|| false));
        let token = CancellationToken::new();

        scheduler.poll_cycle(&token, &token).await;

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
    async fn cancellation_releases_claim() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let controller_id = Uuid::now_v7();

        // Insert a task that is due now.
        let now = time::OffsetDateTime::now_utc();
        let task = scheduled_task::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            tenant_id: ActiveValue::Set(tenant.id),
            task_type: ActiveValue::Set(ScheduledTaskType::StaleLeaseCleanup),
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
        .insert(&db)
        .await
        .expect("insert task");

        // Register an executor that blocks until a signal, so we can cancel mid-execution.
        let (exec_started_tx, mut exec_started_rx) = tokio::sync::oneshot::channel::<()>();
        let (unblock_tx, unblock_rx) = tokio::sync::oneshot::channel::<()>();

        struct BlockingExecutor {
            started: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
            unblock: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        }

        #[async_trait::async_trait]
        impl TaskExecutor for BlockingExecutor {
            async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
                // Signal that execution has started.
                if let Some(tx) = self.started.lock().unwrap().take() {
                    let _ = tx.send(());
                }
                // Block until unblocked (simulating long-running work).
                // Extract the receiver *before* awaiting so the MutexGuard is
                // dropped — holding it across an await makes the future !Send.
                let rx = self.unblock.lock().unwrap().take();
                if let Some(rx) = rx {
                    let _ = rx.await;
                }
                Ok(())
            }
        }

        let config = SchedulerConfig {
            poll_interval: Duration::from_millis(50),
            controller_id,
            task_execution_timeout: TASK_EXECUTION_TIMEOUT,
        };
        let mut scheduler = Scheduler::new(db.clone(), config, Box::new(|| false));
        scheduler.register(
            ScheduledTaskType::StaleLeaseCleanup,
            Box::new(BlockingExecutor {
                started: std::sync::Mutex::new(Some(exec_started_tx)),
                unblock: std::sync::Mutex::new(Some(unblock_rx)),
            }),
        );

        let drain = CancellationToken::new();
        let abort = CancellationToken::new();
        let abort_clone = abort.clone();

        // Run the scheduler in a background task.
        let scheduler_handle = tokio::spawn(async move {
            scheduler.run(drain, abort_clone).await;
        });

        // Wait until the executor has started (i.e. the task has been claimed).
        tokio::time::timeout(Duration::from_secs(5), &mut exec_started_rx)
            .await
            .expect("executor should start within 5s")
            .expect("channel closed");

        // Verify the task is now locked by our controller.
        let locked = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert_eq!(
            locked.locked_by,
            Some(controller_id),
            "task should be claimed"
        );

        // Hard-abort the scheduler while the executor is still running.
        abort.cancel();
        // Unblock the executor so its task can observe the abort signal.
        let _ = unblock_tx.send(());

        // Wait for the scheduler to finish.
        tokio::time::timeout(Duration::from_secs(5), scheduler_handle)
            .await
            .expect("scheduler should shut down within 5s")
            .expect("scheduler task panicked");

        // After shutdown the claim MUST be released (locked_by IS NULL).
        let after = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert!(
            after.locked_by.is_none(),
            "claim must be released on cancellation; got locked_by = {:?}",
            after.locked_by
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "double-registration detected")]
    fn register_debug_panics_on_double_registration() {
        // Use a throw-away in-memory connection handle (no real DB needed for
        // the register path, which only modifies the HashMap).
        let db = {
            // Safety: we only test the synchronous HashMap guard, the DB is
            // never actually used before the panic.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async { sea_orm::Database::connect("sqlite::memory:").await.unwrap() })
        };
        let config = SchedulerConfig::new(Uuid::now_v7());
        let mut scheduler = Scheduler::new(db, config, Box::new(|| false));

        struct NoopExecutor;
        #[async_trait::async_trait]
        impl TaskExecutor for NoopExecutor {
            async fn execute(
                &self,
                _task: &uptrakit_shared_db::entity::scheduled_task::Model,
            ) -> crate::error::Result<()> {
                Ok(())
            }
        }

        scheduler.register(ScheduledTaskType::AuthCleanup, Box::new(NoopExecutor));
        // Second registration for the same type must panic in debug builds.
        scheduler.register(ScheduledTaskType::AuthCleanup, Box::new(NoopExecutor));
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
        .insert(&db)
        .await
        .expect("insert task");

        let executed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let executed_clone = executed.clone();

        struct TrackingExecutor(std::sync::Arc<std::sync::atomic::AtomicBool>);
        #[async_trait::async_trait]
        impl TaskExecutor for TrackingExecutor {
            async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }

        let config = SchedulerConfig::new(Uuid::now_v7());
        let mut scheduler = Scheduler::new(db.clone(), config, Box::new(|| false));
        scheduler.register(
            ScheduledTaskType::StaleLeaseCleanup,
            Box::new(TrackingExecutor(executed_clone)),
        );
        let token = CancellationToken::new();

        scheduler.poll_cycle(&token, &token).await;

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

    #[tokio::test]
    async fn poll_cycle_skips_external_tasks_when_flag_set() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;

        // Seed a due external task (AuthCleanup).
        let now = time::OffsetDateTime::now_utc();
        let task_id = Uuid::now_v7();
        scheduled_task::ActiveModel {
            id: ActiveValue::Set(task_id),
            tenant_id: ActiveValue::Set(tenant.id),
            task_type: ActiveValue::Set(ScheduledTaskType::AuthCleanup),
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
        .insert(&db)
        .await
        .expect("insert task");

        let executed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let executed_clone = executed.clone();

        struct TrackingExecutor(std::sync::Arc<std::sync::atomic::AtomicBool>);
        #[async_trait::async_trait]
        impl TaskExecutor for TrackingExecutor {
            async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }

        let config = SchedulerConfig::new(Uuid::now_v7());
        let mut scheduler = Scheduler::new(db.clone(), config, Box::new(|| true));
        scheduler.register(
            ScheduledTaskType::AuthCleanup,
            Box::new(TrackingExecutor(executed_clone)),
        );
        let token = CancellationToken::new();

        scheduler.poll_cycle(&token, &token).await;

        assert!(
            !executed.load(std::sync::atomic::Ordering::SeqCst),
            "external task should be skipped when external scheduler is connected"
        );

        // Task should remain unclaimed.
        let task = scheduled_task::Entity::find_by_id(task_id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert!(task.locked_by.is_none());
        assert_eq!(task.run_count, 0);
    }

    #[tokio::test]
    async fn poll_cycle_runs_internal_tasks_when_flag_set() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;

        // Seed a due internal task (CrlRenewal).
        let now = time::OffsetDateTime::now_utc();
        let task_id = Uuid::now_v7();
        scheduled_task::ActiveModel {
            id: ActiveValue::Set(task_id),
            tenant_id: ActiveValue::Set(tenant.id),
            task_type: ActiveValue::Set(ScheduledTaskType::CrlRenewal),
            interval_seconds: ActiveValue::Set(14400),
            jitter_seconds: ActiveValue::Set(120),
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
            async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }

        let config = SchedulerConfig::new(Uuid::now_v7());
        let mut scheduler = Scheduler::new(db.clone(), config, Box::new(|| true));
        scheduler.register(
            ScheduledTaskType::CrlRenewal,
            Box::new(TrackingExecutor(executed_clone)),
        );
        let token = CancellationToken::new();

        scheduler.poll_cycle(&token, &token).await;

        assert!(
            executed.load(std::sync::atomic::Ordering::SeqCst),
            "internal task should still execute when external scheduler is connected"
        );

        // Task should be released and run_count incremented.
        let task = scheduled_task::Entity::find_by_id(task_id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert!(task.locked_by.is_none());
        assert_eq!(task.run_count, 1);
    }
}
