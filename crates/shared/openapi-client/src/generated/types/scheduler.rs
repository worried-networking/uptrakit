use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
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
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
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
