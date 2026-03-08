use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// Task type string for the fetch-releases scheduler task.
///
/// Use this constant instead of a raw string literal wherever the `task_type`
/// field of [`ScheduledTaskResponse`] is compared or displayed, to avoid a
/// silent mismatch if the DB-side string value is ever changed.
///
/// This task fetches the latest available versions from external APIs and
/// dispatches agent-side package-index queries. It replaces the old
/// `version_check` task, which was renamed in migration
/// `m20260307_000001_split_version_check`.
pub const TASK_TYPE_FETCH_RELEASES: &str = "fetch_releases";

/// Task type string for the detect-version scheduler task.
///
/// This task detects the currently installed versions on all agent hosts.
/// It is the counterpart to [`TASK_TYPE_FETCH_RELEASES`] and was introduced
/// alongside it when the old `version_check` task was split in two.
pub const TASK_TYPE_DETECT_VERSION: &str = "detect_version";

/// Response for a single scheduled task.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScheduledTaskResponse {
    pub id: Uuid,
    pub task_type: String,
    pub label: String,
    pub interval_seconds: i32,
    pub jitter_seconds: i32,
    pub enabled: bool,
    pub task_config: Option<serde_json::Value>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub last_run_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub next_run_at: OffsetDateTime,
    pub is_running: bool,
    pub last_error: Option<String>,
    pub run_count: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}

/// Request to update a scheduled task.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateScheduledTaskRequest {
    /// Base repeat interval in seconds. Must be > 0.
    pub interval_seconds: Option<i32>,
    /// Maximum random jitter added to each interval in seconds. Must be >= 0.
    pub jitter_seconds: Option<i32>,
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
        if let Some(interval) = self.interval_seconds
            && interval <= 0
        {
            return Err(ValidationError {
                field: "interval_seconds",
                message: "must be greater than 0".to_string(),
            });
        }

        if let Some(jitter) = self.jitter_seconds
            && jitter < 0
        {
            return Err(ValidationError {
                field: "jitter_seconds",
                message: "must be >= 0".to_string(),
            });
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
        use time::macros::datetime;
        let resp = ScheduledTaskResponse {
            id: sample_uuid(),
            task_type: "fetch_releases".to_string(),
            label: "Fetch Latest Releases".to_string(),
            interval_seconds: 21600,
            jitter_seconds: 300,
            enabled: true,
            task_config: Some(serde_json::json!({"timeout": 30})),
            last_run_at: Some(datetime!(2025-06-01 00:00:00 UTC)),
            next_run_at: datetime!(2025-06-02 00:00:00 UTC),
            is_running: false,
            last_error: Some("timeout exceeded".to_string()),
            run_count: 42,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 00:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ScheduledTaskResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.task_type, "fetch_releases");
        assert_eq!(deserialized.label, "Fetch Latest Releases");
        assert_eq!(deserialized.interval_seconds, 21600);
        assert_eq!(deserialized.jitter_seconds, 300);
        assert!(deserialized.enabled);
        assert!(deserialized.task_config.is_some());
        assert_eq!(
            deserialized.last_run_at,
            Some(datetime!(2025-06-01 00:00:00 UTC))
        );
        assert_eq!(deserialized.next_run_at, datetime!(2025-06-02 00:00:00 UTC));
        assert!(!deserialized.is_running);
        assert_eq!(deserialized.last_error.as_deref(), Some("timeout exceeded"));
        assert_eq!(deserialized.run_count, 42);
    }

    #[test]
    fn scheduled_task_response_round_trip_none_fields() {
        use time::macros::datetime;
        let resp = ScheduledTaskResponse {
            id: sample_uuid(),
            task_type: "cleanup".to_string(),
            label: "Weekly cleanup".to_string(),
            interval_seconds: 86400,
            jitter_seconds: 300,
            enabled: false,
            task_config: None,
            last_run_at: None,
            next_run_at: datetime!(2025-06-08 00:00:00 UTC),
            is_running: false,
            last_error: None,
            run_count: 0,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
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
        use time::macros::datetime;
        let resp = ScheduledTaskResponse {
            id: sample_uuid(),
            task_type: "sync".to_string(),
            label: "Sync".to_string(),
            interval_seconds: 300,
            jitter_seconds: 30,
            enabled: true,
            task_config: None,
            last_run_at: None,
            next_run_at: datetime!(2025-06-01 00:05:00 UTC),
            is_running: true,
            last_error: None,
            run_count: 10,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
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
            interval_seconds: Some(7200),
            jitter_seconds: Some(60),
            enabled: Some(false),
            task_config: Some(serde_json::json!({"retries": 3})),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: UpdateScheduledTaskRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.interval_seconds, Some(7200));
        assert_eq!(deserialized.jitter_seconds, Some(60));
        assert_eq!(deserialized.enabled, Some(false));
        assert!(deserialized.task_config.is_some());
    }

    #[test]
    fn update_scheduled_task_request_round_trip_none_fields() {
        let req = UpdateScheduledTaskRequest {
            interval_seconds: None,
            jitter_seconds: None,
            enabled: None,
            task_config: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: UpdateScheduledTaskRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.interval_seconds.is_none());
        assert!(deserialized.jitter_seconds.is_none());
        assert!(deserialized.enabled.is_none());
        assert!(deserialized.task_config.is_none());
    }

    #[test]
    fn update_scheduled_task_request_from_empty_json() {
        let json = r#"{}"#;
        let req: UpdateScheduledTaskRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert!(req.interval_seconds.is_none());
        assert!(req.jitter_seconds.is_none());
        assert!(req.enabled.is_none());
        assert!(req.task_config.is_none());
    }

    // ── UpdateScheduledTaskRequest validation ────────────────────────

    #[test]
    fn validate_valid_interval() {
        let req = UpdateScheduledTaskRequest {
            interval_seconds: Some(300),
            jitter_seconds: Some(30),
            enabled: None,
            task_config: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_none_interval_passes() {
        let req = UpdateScheduledTaskRequest {
            interval_seconds: None,
            jitter_seconds: None,
            enabled: Some(true),
            task_config: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_zero_interval_fails() {
        let req = UpdateScheduledTaskRequest {
            interval_seconds: Some(0),
            jitter_seconds: None,
            enabled: None,
            task_config: None,
        };
        let err = req
            .validate()
            .expect_err("zero interval should fail validation");
        assert_eq!(err.field, "interval_seconds");
        assert!(err.message.contains("greater than 0"));
    }

    #[test]
    fn validate_negative_interval_fails() {
        let req = UpdateScheduledTaskRequest {
            interval_seconds: Some(-1),
            jitter_seconds: None,
            enabled: None,
            task_config: None,
        };
        let err = req
            .validate()
            .expect_err("negative interval should fail validation");
        assert_eq!(err.field, "interval_seconds");
    }

    #[test]
    fn validate_negative_jitter_fails() {
        let req = UpdateScheduledTaskRequest {
            interval_seconds: None,
            jitter_seconds: Some(-1),
            enabled: None,
            task_config: None,
        };
        let err = req
            .validate()
            .expect_err("negative jitter should fail validation");
        assert_eq!(err.field, "jitter_seconds");
        assert!(err.message.contains(">= 0"));
    }

    #[test]
    fn validate_zero_jitter_passes() {
        let req = UpdateScheduledTaskRequest {
            interval_seconds: Some(300),
            jitter_seconds: Some(0),
            enabled: None,
            task_config: None,
        };
        assert!(req.validate().is_ok());
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
