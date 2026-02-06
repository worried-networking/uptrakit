use serde::{Deserialize, Serialize};

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

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
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
