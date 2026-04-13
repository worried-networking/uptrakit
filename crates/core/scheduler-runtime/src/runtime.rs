use std::sync::Arc;
use std::time::Duration;

use sea_orm::DatabaseConnection;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uptrakit_scheduler_engine::executors::{
    auth_cleanup::AuthCleanupExecutor, detect_version::DetectVersionExecutor,
    discover_software::DiscoverSoftwareExecutor, fetch_releases::FetchReleasesExecutor,
    stale_lease_cleanup::StaleLeaseCleanupExecutor,
};
use uptrakit_scheduler_engine::{
    Scheduler, SchedulerConfig, SchedulerNotifier, TASK_EXECUTION_TIMEOUT,
};
use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
use uuid::Uuid;

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(15);
const STOP_SCHEDULER_TIMEOUT: Duration = Duration::from_secs(30);

pub enum SchedulerStopMode {
    Drain,
    Abort,
}

pub struct SchedulerRunConfig {
    pub db: DatabaseConnection,
    pub controller_id: Uuid,
    pub notifier: Arc<dyn SchedulerNotifier>,
    pub should_yield: Box<dyn Fn() -> bool + Send + Sync>,
    pub poll_interval: Duration,
}

pub struct ManagedSchedulerRuntime {
    running: Option<RunningScheduler>,
}

struct RunningScheduler {
    drain: CancellationToken,
    abort: CancellationToken,
    handle: JoinHandle<()>,
}

impl SchedulerRunConfig {
    pub fn new(
        db: DatabaseConnection,
        controller_id: Uuid,
        notifier: Arc<dyn SchedulerNotifier>,
        should_yield: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        Self {
            db,
            controller_id,
            notifier,
            should_yield,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }
}

impl ManagedSchedulerRuntime {
    pub fn new() -> Self {
        Self { running: None }
    }

    pub async fn restart<F>(&mut self, config: SchedulerRunConfig, register_extras: F)
    where
        F: FnOnce(&mut Scheduler) + Send + 'static,
    {
        self.stop(SchedulerStopMode::Drain).await;

        let drain = CancellationToken::new();
        let abort = CancellationToken::new();
        let handle = tokio::spawn(run_scheduler(
            config,
            drain.clone(),
            abort.clone(),
            register_extras,
        ));

        self.running = Some(RunningScheduler {
            drain,
            abort,
            handle,
        });
    }

    pub async fn stop(&mut self, mode: SchedulerStopMode) {
        if let Some(running) = self.running.take() {
            running.stop(mode).await;
        }
    }
}

impl Default for ManagedSchedulerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn run_scheduler<F>(
    config: SchedulerRunConfig,
    drain: CancellationToken,
    abort: CancellationToken,
    register_extras: F,
) where
    F: FnOnce(&mut Scheduler),
{
    build_scheduler(config, register_extras)
        .run(drain, abort)
        .await;
}

fn build_scheduler<F>(config: SchedulerRunConfig, register_extras: F) -> Scheduler
where
    F: FnOnce(&mut Scheduler),
{
    let SchedulerRunConfig {
        db,
        controller_id,
        notifier,
        should_yield,
        poll_interval,
    } = config;

    let mut scheduler = Scheduler::new(
        db.clone(),
        SchedulerConfig {
            poll_interval,
            controller_id,
            task_execution_timeout: TASK_EXECUTION_TIMEOUT,
        },
        should_yield,
    );

    scheduler.register(
        ScheduledTaskType::AuthCleanup,
        Box::new(AuthCleanupExecutor::new(db.clone())),
    );
    scheduler.register(
        ScheduledTaskType::StaleLeaseCleanup,
        Box::new(StaleLeaseCleanupExecutor::new(db.clone())),
    );
    scheduler.register(
        ScheduledTaskType::FetchReleases,
        Box::new(FetchReleasesExecutor::new(
            db.clone(),
            Arc::clone(&notifier),
        )),
    );
    scheduler.register(
        ScheduledTaskType::DetectVersion,
        Box::new(DetectVersionExecutor::new(
            db.clone(),
            Arc::clone(&notifier),
        )),
    );
    scheduler.register(
        ScheduledTaskType::DiscoverSoftware,
        Box::new(DiscoverSoftwareExecutor::new(db, notifier)),
    );

    register_extras(&mut scheduler);
    scheduler
}

impl RunningScheduler {
    async fn stop(self, mode: SchedulerStopMode) {
        let mut handle = self.handle;

        match mode {
            SchedulerStopMode::Drain => {
                tracing::info!("stopping scheduler engine (graceful drain)");
                self.drain.cancel();
            }
            SchedulerStopMode::Abort => {
                tracing::info!("stopping scheduler engine (hard abort)");
                self.abort.cancel();
            }
        }

        match tokio::time::timeout(STOP_SCHEDULER_TIMEOUT, &mut handle).await {
            Ok(Ok(())) => {
                tracing::info!("scheduler engine stopped cleanly");
            }
            Ok(Err(join_err)) => {
                tracing::error!(error = %join_err, "scheduler task panicked during shutdown");
            }
            Err(_elapsed) => {
                tracing::warn!(
                    timeout_secs = STOP_SCHEDULER_TIMEOUT.as_secs(),
                    "scheduler task did not stop within timeout"
                );
                handle.abort();
                let _ = handle.await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn stop_drain_cancels_drain_token() {
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();
        let handle = tokio::spawn(async move {
            std::future::pending::<()>().await;
        });
        let running = RunningScheduler {
            drain: drain.clone(),
            abort,
            handle,
        };
        let mut runtime = ManagedSchedulerRuntime {
            running: Some(running),
        };

        runtime.stop(SchedulerStopMode::Drain).await;

        assert!(
            drain.is_cancelled(),
            "drain shutdown should cancel drain token"
        );
        assert!(
            runtime.running.is_none(),
            "runtime should clear stopped handle"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stop_abort_cancels_abort_token() {
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();
        let handle = tokio::spawn(async move {
            std::future::pending::<()>().await;
        });
        let running = RunningScheduler {
            drain,
            abort: abort.clone(),
            handle,
        };
        let mut runtime = ManagedSchedulerRuntime {
            running: Some(running),
        };

        runtime.stop(SchedulerStopMode::Abort).await;

        assert!(
            abort.is_cancelled(),
            "abort shutdown should cancel abort token"
        );
        assert!(
            runtime.running.is_none(),
            "runtime should clear stopped handle"
        );
    }
}
