use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
    pub initiated_by: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub created_at: String,
}
