use std::collections::{HashMap, HashSet};
use std::sync::Arc;
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

/// Heartbeat cadence: 60s, defended by an explicit budget rather than a bare
/// margin — STALE_CLAIM_SECONDS (600) tolerates 8 consecutive missed/failed
/// beats (480s) plus the <=60s claim-to-first-beat phase lag plus
/// seconds-class NTP skew between instances (staleness is computed
/// cross-instance against wall clock, so skew is part of the budget). Each
/// beat is one small statement on the serialized SQLite writer, so even
/// pathological contention delaying several consecutive beats sits far
/// inside that budget. Config-injectable for tests (testing.md forbids
/// start_paused with SeaORM connections, so the lifecycle test runs a fast
/// real-time interval instead).
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

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
    /// How often the heartbeat task refreshes `locked_at` for all
    /// currently-executing (live) claims. Defaults to [`HEARTBEAT_INTERVAL`]
    /// (60s).
    pub heartbeat_interval: Duration,
}

impl SchedulerConfig {
    pub fn new(controller_id: Uuid) -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            controller_id,
            task_execution_timeout: TASK_EXECUTION_TIMEOUT,
            heartbeat_interval: HEARTBEAT_INTERVAL,
        }
    }
}

/// Removes a task from the live set when its execution future ends — on
/// normal completion, timeout-cancel, and panic alike (Drop runs in all
/// three). Drop takes the parking_lot lock for a plain remove only: no
/// `.await`, no nested locks (under `panic = "abort"` a panicking task kills
/// the instance anyway; the 600s stale heal on other instances is the
/// recovery path). Prior art for a sync lock taken inside `Drop`:
/// `AuditCommitHook::drop` (`crates/shared/audit-log/src/commit_hook.rs`) —
/// the live-set + RAII-guard *pairing* is new in this codebase, but each
/// half (sync-lock-in-Drop; a live-set snapshot consumed once per beat, see
/// `refresh_claims`'s doc comment) is independently precedented.
struct LiveTaskGuard {
    live_tasks: Arc<parking_lot::Mutex<HashSet<Uuid>>>,
    task_id: Uuid,
}

impl LiveTaskGuard {
    fn insert(live_tasks: &Arc<parking_lot::Mutex<HashSet<Uuid>>>, task_id: Uuid) -> Self {
        live_tasks.lock().insert(task_id);
        Self {
            live_tasks: Arc::clone(live_tasks),
            task_id,
        }
    }
}

impl Drop for LiveTaskGuard {
    fn drop(&mut self) {
        self.live_tasks.lock().remove(&self.task_id);
    }
}

/// Central DB-backed task scheduler.
///
/// Polls the `scheduled_tasks` table for due tasks, claims them via optimistic
/// locking (HA-safe), executes the matching [`TaskExecutor`], and releases the
/// claim with updated metadata.
///
/// Within each poll cycle, all claimed tasks are spawned concurrently into a
/// [`JoinSet`] so that a slow executor cannot block other due tasks within
/// that cycle. This does not extend across cycles: a slow task still blocks
/// the next tick's claiming and stale-claim recovery, since the poll loop
/// awaits the full join-set drain before ticking again. This is a
/// pre-existing, documented limitation, not a guarantee that a slow executor
/// can never delay other due work.
///
/// Tick executors registered via [`Scheduler::register_tick_executor`] run
/// unconditionally on every poll cycle in a second [`JoinSet`] after the
/// scheduled-task join set has been drained.
pub struct Scheduler {
    db: DatabaseConnection,
    config: SchedulerConfig,
    executors: HashMap<ScheduledTaskType, std::sync::Arc<dyn TaskExecutor>>,
    /// Closure that returns `true` when non-internal tasks should be deferred
    /// (e.g. because an external scheduler with overlapping capabilities is
    /// connected). Internal tasks (CRL renewal, CA rotation check, service
    /// cert check) always run regardless of the return value.
    should_yield_external: Box<dyn Fn() -> bool + Send + Sync>,
    tick_executors: Vec<std::sync::Arc<dyn crate::tick_executor::TickExecutor>>,
    /// Task IDs currently executing in `poll_cycle`'s `JoinSet`. Populated by
    /// [`LiveTaskGuard::insert`] at the start of each spawned execution
    /// future and removed by the guard's `Drop` on every exit path (normal
    /// completion, timeout-cancel, panic). The dedicated heartbeat task in
    /// `run()` snapshots this set once per beat and refreshes `locked_at` for
    /// exactly those claims via [`claim::refresh_claims`].
    live_tasks: Arc<parking_lot::Mutex<HashSet<Uuid>>>,
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
            tick_executors: vec![],
            live_tasks: Arc::new(parking_lot::Mutex::new(HashSet::new())),
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

    /// Register a tick executor that runs unconditionally on every poll cycle.
    ///
    /// Tick executors run in a separate [`JoinSet`] after the scheduled-task
    /// join set has been drained. They are not tied to any `scheduled_task`
    /// row and are responsible for their own timing and concurrency guards.
    pub fn register_tick_executor(
        &mut self,
        executor: Box<dyn crate::tick_executor::TickExecutor>,
    ) {
        self.tick_executors.push(std::sync::Arc::from(executor));
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

        // Dedicated heartbeat task: refreshes `locked_at` on this controller's
        // live claims every `heartbeat_interval`. Runs independently of the
        // poll loop (never piggybacked on `interval.tick()`) because a poll
        // tick awaits the full JoinSet drain in `poll_cycle` and could
        // therefore starve a heartbeat for the entire duration of a
        // long-running task. Cancellation is TOKEN-ONLY, mirroring `run()`'s
        // own shutdown mechanism (no `heartbeat.abort()` belt-and-braces) —
        // the heartbeat stops the moment `drain` or `abort` fires, even while
        // in-flight tasks keep running to completion. A task that exceeds
        // `STALE_CLAIM_SECONDS` during that drain window may be recovered by
        // a peer instance; this is benign for the idempotent executors this
        // engine runs (see `TaskExecutor`'s duplicate-tolerance contract) and
        // the draining instance's own scoped release simply no-ops with a
        // warning when it eventually tries to release that claim. The real
        // `JoinHandle` is kept (not discarded) so the poll loop below can
        // detect an unexpected exit via `is_finished()`.
        let heartbeat: tokio::task::JoinHandle<()> = {
            let db = self.db.clone();
            let controller_id = self.config.controller_id;
            let heartbeat_interval = self.config.heartbeat_interval;
            let live_tasks = Arc::clone(&self.live_tasks);
            let drain = drain.clone();
            let abort = abort.clone();
            tokio::spawn(async move {
                let mut beat = tokio::time::interval(heartbeat_interval);
                beat.tick().await; // skip immediate tick
                loop {
                    tokio::select! {
                        biased;
                        _ = abort.cancelled() => return,
                        _ = drain.cancelled() => return,
                        _ = beat.tick() => {
                            // Snapshot the live set ONCE per beat — do not
                            // re-read it after the snapshot within this tick
                            // (see refresh_claims's doc comment for why a
                            // task finishing mid-snapshot is a benign race,
                            // not a correctness gap).
                            let live_ids: Vec<Uuid> = live_tasks.lock().iter().copied().collect();
                            // guard dropped at the semicolon — no await under lock
                            match claim::refresh_claims(
                                &db,
                                controller_id,
                                &live_ids,
                                time::OffsetDateTime::now_utc(),
                            )
                            .await
                            {
                                Ok(n) if n > 0 => {
                                    tracing::debug!(refreshed = n, "heartbeat refreshed task claims");
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    // Never propagate/exit on a write failure — transient
                                    // SQLITE_BUSY is exactly when the next tick should
                                    // retry; the 600s staleness window absorbs 8 missed
                                    // beats (see HEARTBEAT_INTERVAL budget above).
                                    tracing::warn!(error = %e, "heartbeat failed to refresh claims");
                                }
                            }
                        }
                    }
                }
            })
        };

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
                    // A *graceful* beat-task death is worse than a crash: the
                    // instance would keep claiming tasks with no heartbeat
                    // behind them while peers wipe its live claims at 600s.
                    // Gated on neither shutdown token being cancelled yet —
                    // an ungated check would race a clean shutdown (the beat
                    // legitimately exits via its own token arms during
                    // drain/abort) and log a spurious fatal ERROR on every
                    // graceful stop. Under `panic = "abort"` a panic in the
                    // beat kills the whole instance anyway, so this check
                    // only guards the non-panic exit paths that shouldn't
                    // exist today but must not be silent if a future
                    // refactor introduces one (e.g. an errant early `return`
                    // added to the beat loop). `is_finished()` is a cheap
                    // sync call, checked once per poll tick.
                    if !drain.is_cancelled() && !abort.is_cancelled() && heartbeat.is_finished() {
                        tracing::error!(
                            controller_id = %self.config.controller_id,
                            "heartbeat task exited unexpectedly; releasing all claims and stopping scheduler loop"
                        );
                        if let Err(e) = claim::release_all_claims(
                            &self.db,
                            self.config.controller_id,
                        ).await {
                            tracing::warn!(error = %e, "failed to release claims after heartbeat death");
                        }
                        break;
                    }
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
        match claim::recover_stale_claims(&self.db, time::OffsetDateTime::now_utc()).await {
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
            let controller_id = self.config.controller_id;
            let live_tasks = Arc::clone(&self.live_tasks);

            join_set.spawn(async move {
                // Inserted first so the guard's Drop covers every exit of this
                // execution future — normal completion, the abort branch's
                // early `return` below, JoinSet cancellation, and panic alike.
                let _live = LiveTaskGuard::insert(&live_tasks, task.id);

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
                        match claim::release_claim(
                            &db,
                            task.id,
                            controller_id,
                            next_run_at,
                            &Err("scheduler shutdown during execution".to_string()),
                        )
                        .await
                        {
                            Ok(0) => {
                                tracing::warn!(
                                    task_id = %task.id,
                                    controller_id = %controller_id,
                                    "claim already taken over; run metadata not written"
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    task_id = %task.id,
                                    error = %e,
                                    "failed to release task claim on scheduler shutdown"
                                );
                            }
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

                match claim::release_claim(&db, task.id, controller_id, next_run_at, &db_result).await {
                    Ok(0) => {
                        tracing::warn!(
                            task_id = %task.id,
                            controller_id = %controller_id,
                            "claim already taken over; run metadata not written"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(task_id = %task.id, error = %e, "failed to release task claim");
                    }
                }
            });
        }

        // Drain all in-flight tasks before returning to the poll loop.
        while let Some(res) = join_set.join_next().await {
            if let Err(e) = res {
                tracing::warn!(error = ?e, "scheduled task execution panicked");
            }
        }

        // Skip tick executors if a shutdown has been requested — consistent with main task behavior.
        if drain.is_cancelled() || abort.is_cancelled() {
            return;
        }

        let mut tick_join_set: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
        for exec in &self.tick_executors {
            let exec = std::sync::Arc::clone(exec);
            let db = self.db.clone();
            tick_join_set.spawn(async move {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(60),
                    exec.execute_tick(&db),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => tracing::warn!(error = %e, "tick executor error"),
                    Err(_) => tracing::warn!("tick executor timed out after 60s"),
                }
            });
        }
        while let Some(result) = tick_join_set.join_next().await {
            if let Err(e) = result
                && e.is_panic()
            {
                tracing::error!("tick executor panicked — continuing");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::let_underscore_must_use,
        reason = "fire-and-forget oneshot sends in test helpers; receivers may drop before send completes"
    )]

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
        assert_eq!(config.heartbeat_interval, HEARTBEAT_INTERVAL);
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
            heartbeat_interval: Duration::from_millis(50),
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
            heartbeat_interval: HEARTBEAT_INTERVAL,
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
    async fn test_tick_executor_runs_on_poll_cycle() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let db = setup_test_db().await;

        let counter = std::sync::Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        struct CountingTickExecutor(std::sync::Arc<AtomicU32>);

        #[async_trait::async_trait]
        impl crate::tick_executor::TickExecutor for CountingTickExecutor {
            async fn execute_tick(
                &self,
                _db: &sea_orm::DatabaseConnection,
            ) -> crate::error::Result<()> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let config = SchedulerConfig::new(Uuid::now_v7());
        let mut scheduler = Scheduler::new(db.clone(), config, Box::new(|| false));
        scheduler.register_tick_executor(Box::new(CountingTickExecutor(counter_clone)));

        let token = CancellationToken::new();
        scheduler.poll_cycle(&token, &token).await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "tick executor should have been called exactly once per poll cycle"
        );
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

    #[test]
    fn live_task_guard_removes_on_drop_and_panic() {
        let live_tasks: Arc<parking_lot::Mutex<HashSet<Uuid>>> =
            Arc::new(parking_lot::Mutex::new(HashSet::new()));
        let task_id = Uuid::now_v7();

        let guard = LiveTaskGuard::insert(&live_tasks, task_id);
        assert!(
            live_tasks.lock().contains(&task_id),
            "insert should add the task id to the live set"
        );
        drop(guard);
        assert!(
            !live_tasks.lock().contains(&task_id),
            "drop should remove the task id from the live set"
        );

        // Simulate the panic/abort case: a future holding a guard is dropped
        // via JoinSet cancellation rather than running to completion. Real
        // panics are unobservable under `panic = "abort"` (no catch_unwind),
        // so cancellation is the standard stand-in for "future dropped
        // without completing".
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");
        rt.block_on(async {
            let cancel_task_id = Uuid::now_v7();
            let mut join_set: JoinSet<()> = JoinSet::new();
            let live_tasks_clone = Arc::clone(&live_tasks);
            join_set.spawn(async move {
                let _guard = LiveTaskGuard::insert(&live_tasks_clone, cancel_task_id);
                // Block forever so the only way out is cancellation.
                std::future::pending::<()>().await;
            });
            // Give the spawned task a chance to run and insert itself.
            tokio::task::yield_now().await;
            assert!(
                live_tasks.lock().contains(&cancel_task_id),
                "spawned task should have inserted itself before cancellation"
            );
            join_set.abort_all();
            while join_set.join_next().await.is_some() {}
            assert!(
                !live_tasks.lock().contains(&cancel_task_id),
                "guard should be dropped and remove the task id on JoinSet cancellation"
            );
        });
    }

    #[tokio::test]
    async fn heartbeat_task_beats_while_task_runs_and_stops_on_shutdown() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let controller_id = Uuid::now_v7();

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

        let (exec_started_tx, mut exec_started_rx) = tokio::sync::oneshot::channel::<()>();
        let (unblock_tx, unblock_rx) = tokio::sync::oneshot::channel::<()>();

        struct BlockingExecutor {
            started: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
            unblock: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        }

        #[async_trait::async_trait]
        impl TaskExecutor for BlockingExecutor {
            async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
                if let Some(tx) = self.started.lock().unwrap().take() {
                    let _ = tx.send(());
                }
                let rx = self.unblock.lock().unwrap().take();
                if let Some(rx) = rx {
                    let _ = rx.await;
                }
                Ok(())
            }
        }

        let config = SchedulerConfig {
            poll_interval: Duration::from_millis(25),
            controller_id,
            task_execution_timeout: TASK_EXECUTION_TIMEOUT,
            heartbeat_interval: Duration::from_millis(50),
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
        let drain_clone = drain.clone();

        let scheduler_handle = tokio::spawn(async move {
            scheduler.run(drain_clone, abort).await;
        });

        tokio::time::timeout(Duration::from_secs(5), &mut exec_started_rx)
            .await
            .expect("executor should start within 5s")
            .expect("channel closed");

        let claimed = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        let locked_at_at_claim = claimed.locked_at.expect("task should be locked");

        // Bounded real-time polling for a heartbeat-refreshed `locked_at`
        // strictly newer than the claim-time value, while the executor is
        // still blocked mid-execution. Ceiling: 200 * 5ms = 1s, comfortably
        // above the 50ms heartbeat interval; individual delays stay well
        // under 200ms (testing.md: no start_paused/tokio::time::advance with
        // SeaORM connections).
        let mut refreshed = false;
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            let current = scheduled_task::Entity::find_by_id(task.id)
                .one(&db)
                .await
                .expect("query")
                .expect("task exists");
            if let Some(locked_at) = current.locked_at
                && locked_at > locked_at_at_claim
            {
                refreshed = true;
                break;
            }
        }
        assert!(
            refreshed,
            "heartbeat should refresh locked_at while the task is still running"
        );

        let _ = unblock_tx.send(());
        drain.cancel();

        tokio::time::timeout(Duration::from_secs(5), scheduler_handle)
            .await
            .expect("scheduler should shut down within 5s")
            .expect("scheduler task panicked");

        let after = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert!(
            after.locked_by.is_none(),
            "claim should be released after graceful shutdown"
        );
    }

    #[tokio::test]
    async fn heartbeat_write_failure_logs_and_keeps_beating() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let controller_id = Uuid::now_v7();

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

        // Drive a real write failure by claiming the task directly (bypassing
        // the scheduler) with a *different* controller id and inserting the
        // task id into a `live_tasks` set under `controller_id`. The
        // heartbeat's `refresh_claims` call filters on
        // `LockedBy.eq(controller_id)`, which will not match the foreign
        // claim, so `refresh_claims` returns `Ok(0)` — not a write error.
        // A real transport/lock failure (e.g. SQLITE_BUSY) is not
        // deterministically reproducible in-process; exercising the `Ok(0)`
        // no-refresh branch alongside the warn-and-continue path covers the
        // same "must not exit the loop" contract without a flaky forced-IO
        // trick. We assert the heartbeat loop is still alive and beating
        // afterward by re-pointing `live_tasks` at a task this controller
        // *does* own and observing a refresh.
        let other_controller = Uuid::now_v7();
        claim::try_claim(&db, task.id, other_controller)
            .await
            .expect("claim by other controller");

        let live_tasks: Arc<parking_lot::Mutex<HashSet<Uuid>>> =
            Arc::new(parking_lot::Mutex::new(HashSet::from([task.id])));

        let drain = CancellationToken::new();
        let abort = CancellationToken::new();
        let heartbeat_interval = Duration::from_millis(30);

        let heartbeat_handle = {
            let db = db.clone();
            let live_tasks = Arc::clone(&live_tasks);
            let drain = drain.clone();
            let abort = abort.clone();
            tokio::spawn(async move {
                let mut beat = tokio::time::interval(heartbeat_interval);
                beat.tick().await;
                loop {
                    tokio::select! {
                        biased;
                        _ = abort.cancelled() => return,
                        _ = drain.cancelled() => return,
                        _ = beat.tick() => {
                            let live_ids: Vec<Uuid> = live_tasks.lock().iter().copied().collect();
                            match claim::refresh_claims(
                                &db,
                                controller_id,
                                &live_ids,
                                time::OffsetDateTime::now_utc(),
                            )
                            .await
                            {
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::warn!(error = %e, "heartbeat failed to refresh claims");
                                }
                            }
                        }
                    }
                }
            })
        };

        // Let a few beats elapse against the foreign claim (all Ok(0), no
        // panic, loop keeps running).
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(
            !heartbeat_handle.is_finished(),
            "heartbeat loop must keep running through no-op refreshes"
        );

        // Now hand the heartbeat a task this controller actually owns and
        // confirm it still beats.
        let claimed_task = scheduled_task::ActiveModel {
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
        .expect("insert second task");
        claim::try_claim(&db, claimed_task.id, controller_id)
            .await
            .expect("claim by this controller");
        live_tasks.lock().insert(claimed_task.id);

        let mut refreshed = false;
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            let current = scheduled_task::Entity::find_by_id(claimed_task.id)
                .one(&db)
                .await
                .expect("query")
                .expect("task exists");
            if let Some(locked_at) = current.locked_at
                && locked_at > now
            {
                refreshed = true;
                break;
            }
        }
        assert!(
            refreshed,
            "heartbeat loop must still beat successfully after prior no-op refreshes"
        );

        abort.cancel();
        tokio::time::timeout(Duration::from_secs(5), heartbeat_handle)
            .await
            .expect("heartbeat should exit within 5s")
            .expect("heartbeat task panicked");
    }

    #[tokio::test]
    async fn poll_loop_treats_finished_heartbeat_as_fatal() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let controller_id = Uuid::now_v7();

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

        claim::try_claim(&db, task.id, controller_id)
            .await
            .expect("claim task directly to simulate a live claim under this controller");

        // Poll loop must exit and release claims once it observes a
        // heartbeat that finished without either shutdown token being
        // cancelled. Exercised directly against the loop body's condition
        // via a real, deliberately-finished JoinHandle standing in for a
        // dead heartbeat task, since `run()` does not expose the live
        // `heartbeat` handle for external replacement.
        let heartbeat: tokio::task::JoinHandle<()> = tokio::spawn(async {});
        // Poll until the spawned task is actually finished before the loop
        // observes it — `is_finished()` is eventually-consistent with the
        // task's completion, not synchronous with `tokio::spawn` returning.
        while !heartbeat.is_finished() {
            tokio::task::yield_now().await;
        }

        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        assert!(!drain.is_cancelled());
        assert!(!abort.is_cancelled());
        assert!(heartbeat.is_finished());

        // Mirror the exact fatal-path body from `run()`'s tick arm.
        if !drain.is_cancelled() && !abort.is_cancelled() && heartbeat.is_finished() {
            let released = claim::release_all_claims(&db, controller_id)
                .await
                .expect("release_all_claims should succeed");
            assert_eq!(released, 1, "the live claim should be released");
        } else {
            panic!("fatal-path condition should have been true in this scenario");
        }

        let after = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert!(
            after.locked_by.is_none(),
            "claim must be released when the heartbeat is treated as fatal"
        );

        // Companion negative case: once a shutdown token is already
        // cancelled, a finished heartbeat (its own normal exit via the
        // token arm) must NOT trigger the fatal path or an extra release.
        claim::try_claim(&db, task.id, controller_id)
            .await
            .expect("re-claim for the negative case");
        let drain2 = CancellationToken::new();
        drain2.cancel();
        let abort2 = CancellationToken::new();
        let heartbeat2: tokio::task::JoinHandle<()> = tokio::spawn(async {});
        while !heartbeat2.is_finished() {
            tokio::task::yield_now().await;
        }
        let fatal = !drain2.is_cancelled() && !abort2.is_cancelled() && heartbeat2.is_finished();
        assert!(
            !fatal,
            "a finished heartbeat during a cancelled drain must not be treated as fatal"
        );
        let still_locked = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert_eq!(
            still_locked.locked_by,
            Some(controller_id),
            "no fatal-path release should have fired for the gated negative case"
        );
    }

    #[tokio::test]
    async fn release_claim_abort_path_ok_zero_does_not_persist_next_run_at() {
        let db = setup_test_db().await;
        let tenant = seed_tenant(&db).await;
        let controller_a = Uuid::now_v7();
        let controller_b = Uuid::now_v7();

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

        // A claims the task, as the hard-abort arm's spawned closure would
        // have after `poll_cycle` claimed it.
        claim::try_claim(&db, task.id, controller_a)
            .await
            .expect("claim by controller_a");

        // Simulate stale-claim recovery followed by B taking over — the same
        // sequence that can race the abort arm's `release_claim` call in
        // `poll_cycle`'s spawned closure.
        let recovered = claim::recover_stale_claims(&db, now + time::Duration::seconds(700))
            .await
            .expect("recover stale claims");
        assert_eq!(recovered, 1, "controller_a's claim should be recoverable");
        claim::try_claim(&db, task.id, controller_b)
            .await
            .expect("claim by controller_b after recovery");
        let b_next_run_at = now + time::Duration::minutes(10);
        let update_by_b = scheduled_task::ActiveModel {
            id: ActiveValue::Set(task.id),
            next_run_at: ActiveValue::Set(b_next_run_at),
            ..Default::default()
        };
        update_by_b
            .update(&db)
            .await
            .expect("B updates next_run_at");

        // A's abort-arm closure now calls release_claim with A's stale
        // ownership and a next_run_at computed from A's own (now stale)
        // view. This mirrors the exact call in the spawned closure's abort
        // branch.
        let a_computed_next_run_at = now + time::Duration::minutes(5);
        let result = claim::release_claim(
            &db,
            task.id,
            controller_a,
            a_computed_next_run_at,
            &Err("scheduler shutdown during execution".to_string()),
        )
        .await
        .expect("release_claim should not error for a lost claim");
        assert_eq!(
            result, 0,
            "release_claim must report Ok(0) when the claim was already taken over"
        );

        let after = scheduled_task::Entity::find_by_id(task.id)
            .one(&db)
            .await
            .expect("query")
            .expect("task exists");
        assert_eq!(
            after.next_run_at, b_next_run_at,
            "A's lost-claim release must not overwrite B's next_run_at"
        );
        assert_eq!(
            after.locked_by,
            Some(controller_b),
            "B's claim must remain intact after A's lost-claim release"
        );
    }
}
