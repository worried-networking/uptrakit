use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(test, derive(strum::EnumIter))]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl UpdateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
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
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(ParseUpdateStatusError),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
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
    pub fn pagination(&self) -> crate::pagination::PaginationParams {
        crate::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub completed_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
    /// Classification of the update (security, bugfix, feature, unknown).
    pub update_category: String,
    /// Whether the update was dispatched in interactive mode (PTY allocated).
    ///
    /// `true` means the agent opened a PTY and kept stdin open. The UI uses
    /// this to show an "Input Required" badge on every in-progress interactive
    /// update in the history list, even when not actively watching the stream.
    pub interactive: bool,
}

// ---------------------------------------------------------------------------
// SSE event types for real-time update output streaming
// ---------------------------------------------------------------------------

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
