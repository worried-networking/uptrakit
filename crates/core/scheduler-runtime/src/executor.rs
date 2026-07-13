use uptrakit_shared_db::entity::scheduled_task;

use crate::error;

/// Trait for executing a scheduled task.
///
/// Each task type has its own executor implementation.
///
/// # Contracts
///
/// - Implementations must not block outside `spawn_blocking`; a blocking
///   executor defeats both the execution timeout and the heartbeat lease and
///   wedges the instance until restart.
/// - Implementations must tolerate duplicate execution — the same task may
///   run concurrently on two instances during a DB partition; this is a
///   stated contract, not an accident, and currently holds for every
///   registered executor by design, not by enforcement.
#[async_trait::async_trait]
pub trait TaskExecutor: Send + Sync {
    /// Execute the task. Returns `Ok(())` on success, `Err(Report<SchedulerError>)` on failure.
    async fn execute(&self, task: &scheduled_task::Model) -> error::Result<()>;
}
