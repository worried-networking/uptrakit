use uptrakit_shared_db::entity::scheduled_task;

/// Trait for executing a scheduled task.
///
/// Each task type has its own executor implementation.
#[async_trait::async_trait]
pub trait TaskExecutor: Send + Sync {
    /// Execute the task. Returns `Ok(())` on success, `Err(message)` on failure.
    async fn execute(&self, task: &scheduled_task::Model) -> Result<(), String>;
}
