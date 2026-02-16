use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::scheduler::{ScheduledTaskResponse, TriggerScheduledTaskResponse};

impl UptrakitClient {
    /// List all scheduled tasks.
    pub async fn list_scheduled_tasks(&self) -> Result<Vec<ScheduledTaskResponse>> {
        self.get("/api/v1/scheduler/tasks").await
    }

    /// Get a single scheduled task by ID.
    pub async fn get_scheduled_task(&self, id: &str) -> Result<ScheduledTaskResponse> {
        let path = format!("/api/v1/scheduler/tasks/{id}");
        self.get(&path).await
    }

    /// Trigger immediate execution of a scheduled task.
    pub async fn trigger_scheduled_task(
        &self,
        id: &str,
    ) -> Result<TriggerScheduledTaskResponse> {
        let path = format!("/api/v1/scheduler/tasks/{id}/trigger");
        self.post_empty(&path).await
    }
}
