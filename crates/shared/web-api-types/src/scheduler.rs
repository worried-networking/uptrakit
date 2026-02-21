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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    // ── ScheduledTaskResponse ────────────────────────────────────────

    #[test]
    fn scheduled_task_response_round_trip_all_fields() {
        let resp = ScheduledTaskResponse {
            id: sample_uuid(),
            task_type: "version_check".to_string(),
            label: "Nightly version check".to_string(),
            cron_expression: "0 0 * * *".to_string(),
            enabled: true,
            task_config: Some(serde_json::json!({"timeout": 30})),
            last_run_at: Some("2025-06-01T00:00:00Z".to_string()),
            next_run_at: "2025-06-02T00:00:00Z".to_string(),
            is_running: false,
            last_error: Some("timeout exceeded".to_string()),
            run_count: 42,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-06-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ScheduledTaskResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.task_type, "version_check");
        assert_eq!(deserialized.label, "Nightly version check");
        assert_eq!(deserialized.cron_expression, "0 0 * * *");
        assert!(deserialized.enabled);
        assert!(deserialized.task_config.is_some());
        assert_eq!(
            deserialized.last_run_at.as_deref(),
            Some("2025-06-01T00:00:00Z")
        );
        assert_eq!(deserialized.next_run_at, "2025-06-02T00:00:00Z");
        assert!(!deserialized.is_running);
        assert_eq!(deserialized.last_error.as_deref(), Some("timeout exceeded"));
        assert_eq!(deserialized.run_count, 42);
    }

    #[test]
    fn scheduled_task_response_round_trip_none_fields() {
        let resp = ScheduledTaskResponse {
            id: sample_uuid(),
            task_type: "cleanup".to_string(),
            label: "Weekly cleanup".to_string(),
            cron_expression: "0 0 * * 0".to_string(),
            enabled: false,
            task_config: None,
            last_run_at: None,
            next_run_at: "2025-06-08T00:00:00Z".to_string(),
            is_running: false,
            last_error: None,
            run_count: 0,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ScheduledTaskResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.task_config.is_none());
        assert!(deserialized.last_run_at.is_none());
        assert!(deserialized.last_error.is_none());
        assert!(!deserialized.enabled);
        assert_eq!(deserialized.run_count, 0);
    }

    #[test]
    fn scheduled_task_response_is_running_true() {
        let resp = ScheduledTaskResponse {
            id: sample_uuid(),
            task_type: "sync".to_string(),
            label: "Sync".to_string(),
            cron_expression: "*/5 * * * *".to_string(),
            enabled: true,
            task_config: None,
            last_run_at: None,
            next_run_at: "2025-06-01T00:05:00Z".to_string(),
            is_running: true,
            last_error: None,
            run_count: 10,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ScheduledTaskResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.is_running);
    }

    // ── UpdateScheduledTaskRequest ───────────────────────────────────

    #[test]
    fn update_scheduled_task_request_round_trip_all_fields() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: Some("0 */2 * * *".to_string()),
            enabled: Some(false),
            task_config: Some(serde_json::json!({"retries": 3})),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: UpdateScheduledTaskRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.cron_expression.as_deref(), Some("0 */2 * * *"));
        assert_eq!(deserialized.enabled, Some(false));
        assert!(deserialized.task_config.is_some());
    }

    #[test]
    fn update_scheduled_task_request_round_trip_none_fields() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: None,
            enabled: None,
            task_config: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: UpdateScheduledTaskRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.cron_expression.is_none());
        assert!(deserialized.enabled.is_none());
        assert!(deserialized.task_config.is_none());
    }

    #[test]
    fn update_scheduled_task_request_from_empty_json() {
        let json = r#"{}"#;
        let req: UpdateScheduledTaskRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert!(req.cron_expression.is_none());
        assert!(req.enabled.is_none());
        assert!(req.task_config.is_none());
    }

    // ── UpdateScheduledTaskRequest validation ────────────────────────

    #[test]
    fn validate_valid_cron_expression() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: Some("0 * * * *".to_string()),
            enabled: None,
            task_config: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_none_cron_passes() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: None,
            enabled: Some(true),
            task_config: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_empty_cron_fails() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: Some("".to_string()),
            enabled: None,
            task_config: None,
        };
        let err = req
            .validate()
            .expect_err("empty cron should fail validation");
        assert_eq!(err.field, "cron_expression");
        assert!(
            err.message.contains("not be empty"),
            "error message should mention emptiness"
        );
    }

    #[test]
    fn validate_cron_too_few_fields_fails() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: Some("0 * *".to_string()),
            enabled: None,
            task_config: None,
        };
        let err = req
            .validate()
            .expect_err("cron with 3 fields should fail validation");
        assert_eq!(err.field, "cron_expression");
        assert!(
            err.message.contains("5 fields"),
            "error message should mention 5 fields"
        );
    }

    #[test]
    fn validate_cron_too_many_fields_fails() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: Some("0 * * * * *".to_string()),
            enabled: None,
            task_config: None,
        };
        let err = req
            .validate()
            .expect_err("cron with 6 fields should fail validation");
        assert_eq!(err.field, "cron_expression");
        assert!(
            err.message.contains("5 fields"),
            "error message should mention 5 fields"
        );
    }

    #[test]
    fn validate_cron_single_field_fails() {
        let req = UpdateScheduledTaskRequest {
            cron_expression: Some("daily".to_string()),
            enabled: None,
            task_config: None,
        };
        let err = req
            .validate()
            .expect_err("single-word cron should fail validation");
        assert_eq!(err.field, "cron_expression");
    }

    // ── TriggerScheduledTaskResponse ─────────────────────────────────

    #[test]
    fn trigger_scheduled_task_response_round_trip() {
        let resp = TriggerScheduledTaskResponse {
            triggered: true,
            message: "Task started".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TriggerScheduledTaskResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.triggered);
        assert_eq!(deserialized.message, "Task started");
    }

    #[test]
    fn trigger_scheduled_task_response_not_triggered() {
        let resp = TriggerScheduledTaskResponse {
            triggered: false,
            message: "Task is already running".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TriggerScheduledTaskResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(!deserialized.triggered);
        assert_eq!(deserialized.message, "Task is already running");
    }
}
