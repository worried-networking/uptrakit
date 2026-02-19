use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// Response for a single scheduled task.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScheduledTaskResponse {
    pub id: Uuid,
    pub task_type: String,
    pub label: String,
    pub cron_expression: String,
    pub enabled: bool,
    pub task_config: Option<serde_json::Value>,
    pub last_run_at: Option<String>,
    pub next_run_at: String,
    pub is_running: bool,
    pub last_error: Option<String>,
    pub run_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Request to update a scheduled task.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateScheduledTaskRequest {
    /// New cron expression (5-field standard cron).
    pub cron_expression: Option<String>,
    /// Enable or disable the task.
    pub enabled: Option<bool>,
    /// Per-task configuration (JSON). Send null to clear.
    pub task_config: Option<serde_json::Value>,
}

/// Response when triggering immediate execution of a task.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerScheduledTaskResponse {
    /// Whether the trigger was applied.
    pub triggered: bool,
    /// Human-readable status message.
    pub message: String,
}

impl Validate for UpdateScheduledTaskRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref cron) = self.cron_expression {
            if cron.is_empty() {
                return Err(ValidationError {
                    field: "cron_expression",
                    message: "must not be empty".to_string(),
                });
            }

            let fields: Vec<&str> = cron.split_whitespace().collect();
            if fields.len() != 5 {
                return Err(ValidationError {
                    field: "cron_expression",
                    message: "must have exactly 5 fields".to_string(),
                });
            }
        }

        Ok(())
    }
}
