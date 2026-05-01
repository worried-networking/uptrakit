use sea_orm::DatabaseConnection;

/// A lightweight executor invoked on every scheduler poll cycle.
///
/// Unlike [`crate::executor::TaskExecutor`], tick executors are not tied to a
/// specific `scheduled_task` row — they run unconditionally on each poll cycle
/// and are responsible for their own timing and concurrency guards.
///
/// Tick executors are registered via [`crate::scheduler::Scheduler::register_tick_executor`].
/// Each executor runs concurrently with the others in a separate [`tokio::task::JoinSet`]
/// after the scheduled-task join set has been drained.
#[async_trait::async_trait]
pub trait TickExecutor: Send + Sync {
    async fn execute_tick(&self, db: &DatabaseConnection) -> crate::error::Result<()>;
}
