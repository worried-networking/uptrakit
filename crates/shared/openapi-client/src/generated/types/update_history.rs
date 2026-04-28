// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(strum::EnumIter))]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    Queued,
    Pending,
    InProgress,
    Completed,
    Failed,
}
impl UpdateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}
impl std::fmt::Display for UpdateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
#[derive(Debug, Error)]
#[error("invalid update status value")]
pub struct ParseUpdateStatusError;
impl std::str::FromStr for UpdateStatus {
    type Err = ParseUpdateStatusError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "queued" => Ok(Self::Queued),
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(ParseUpdateStatusError),
        }
    }
}
#[non_exhaustive]
#[derive(Default, Serialize, Deserialize)]
pub struct UpdateHistoryQuery {
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub status: Option<UpdateStatus>,
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}
impl UpdateHistoryQuery {
    /// Creates a new `UpdateHistoryQuery` with all filter fields set explicitly.
    pub fn new(
        host_id: Option<Uuid>,
        software_item_id: Option<Uuid>,
        status: Option<UpdateStatus>,
        page: Option<u64>,
        per_page: Option<u64>,
    ) -> Self {
        Self {
            host_id,
            software_item_id,
            status,
            page,
            per_page,
        }
    }
    pub fn pagination(&self) -> crate::generated::types::pagination::PaginationParams {
        crate::generated::types::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}
#[non_exhaustive]
#[derive(Serialize, Deserialize)]
pub struct UpdateHistoryResponse {
    pub id: Uuid,
    pub host_id: Uuid,
    pub host_name: String,
    pub software_item_id: Uuid,
    pub software_item_name: String,
    pub from_version: Option<String>,
    pub to_version: String,
    pub status: UpdateStatus,
    pub output: String,
    pub actor_type: String,
    pub actor_id: String,
    /// Human-readable display name of the actor, if resolvable.
    ///
    /// For `actor_type = "user"` this is `"First Last"`.
    /// For `actor_type = "service"` or `"system_service"` this is `friendly_name`.
    /// `None` when the actor record no longer exists or the ID is not a valid UUID.
    pub actor_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Classification of the update (security, bugfix, feature, unknown).
    pub update_category: String,
    /// Whether the update was dispatched in interactive mode (PTY allocated).
    ///
    /// `true` means the agent opened a PTY and kept stdin open. The UI uses
    /// this to show an "Input Required" badge on every in-progress interactive
    /// update in the history list, even when not actively watching the stream.
    pub interactive: bool,
    /// Whether any output was dropped because it exceeded the output size cap.
    ///
    /// When `true`, only the first 50 MB of output is stored. The truncation
    /// point is marked in the output stream with a system notice line. The
    /// detail view shows an amber warning banner when this field is `true`.
    pub output_truncated: bool,
    /// Optional generic pre-update protection status.
    pub pre_update_protection_status: Option<String>,
    /// Optional generic pre-update protection summary.
    pub pre_update_protection_summary: Option<String>,
    /// Optional hint for recovery actions.
    pub recovery_hint: Option<String>,
}
impl UpdateHistoryResponse {
    /// Creates a new `UpdateHistoryResponse` with all fields explicitly set.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Uuid,
        host_id: Uuid,
        host_name: String,
        software_item_id: Uuid,
        software_item_name: String,
        from_version: Option<String>,
        to_version: String,
        status: UpdateStatus,
        output: String,
        actor_type: String,
        actor_id: String,
        actor_name: Option<String>,
        started_at: OffsetDateTime,
        completed_at: Option<OffsetDateTime>,
        created_at: OffsetDateTime,
        update_category: String,
        interactive: bool,
        output_truncated: bool,
        pre_update_protection_status: Option<String>,
        pre_update_protection_summary: Option<String>,
        recovery_hint: Option<String>,
    ) -> Self {
        Self {
            id,
            host_id,
            host_name,
            software_item_id,
            software_item_name,
            from_version,
            to_version,
            status,
            output,
            actor_type,
            actor_id,
            actor_name,
            started_at,
            completed_at,
            created_at,
            update_category,
            interactive,
            output_truncated,
            pre_update_protection_status,
            pre_update_protection_summary,
            recovery_hint,
        }
    }
}
/// SSE `output` event payload: a single line of update output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutputLineSSE {
    pub id: Uuid,
    pub text: String,
    pub stream: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    pub seq: u64,
}
/// SSE `completed` event payload: the update has finished.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateCompletedSSE {
    pub status: String,
    pub error: Option<String>,
}
/// SSE `stdin_attention` event payload: the process is waiting for input.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StdinAttentionSSE {
    pub hint: Option<String>,
}
