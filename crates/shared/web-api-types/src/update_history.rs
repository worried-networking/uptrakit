use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    pub host_id: Option<String>,
    pub software_item_id: Option<String>,
    pub status: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateHistoryResponse {
    pub id: String,
    pub host_id: String,
    pub host_name: String,
    pub software_item_id: String,
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
