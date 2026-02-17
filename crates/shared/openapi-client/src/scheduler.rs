use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::scheduler::{
    ScheduledTaskResponse, TriggerScheduledTaskResponse, UpdateScheduledTaskRequest,
};

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

    /// Update a scheduled task (cron expression, enabled state, or config).
    pub async fn update_scheduled_task(
        &self,
        id: &str,
        req: &UpdateScheduledTaskRequest,
    ) -> Result<ScheduledTaskResponse> {
        let path = format!("/api/v1/scheduler/tasks/{id}");
        self.put_json(&path, req).await
    }

    /// Trigger immediate execution of a scheduled task.
    pub async fn trigger_scheduled_task(&self, id: &str) -> Result<TriggerScheduledTaskResponse> {
        let path = format!("/api/v1/scheduler/tasks/{id}/trigger");
        self.post_empty(&path).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::scheduler::UpdateScheduledTaskRequest;

    #[test]
    fn update_scheduled_task_request_serialization() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: Some("0 */6 * * *".to_string()),
            enabled: Some(true),
            task_config: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["cron_expression"], "0 */6 * * *");
        assert_eq!(json["enabled"], true);
    }

    #[test]
    fn update_scheduled_task_request_with_config() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: None,
            enabled: None,
            task_config: Some(serde_json::json!({"key": "value"})),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["task_config"]["key"], "value");
    }
}
