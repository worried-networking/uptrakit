use crate::generated::shared_types::UpdateCategory;
use crate::generated::types::software_items::TriggerUpdateStatus;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
/// Request body for `POST /api/v1/hosts/{host_id}/batch-update`.
///
/// Triggers updates for all outdated software items on the given host, with
/// optional filtering by update category.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostBatchUpdateRequest {
    /// Only include items with this update category (e.g. `"security"`).
    /// `None` means all outdated items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_filter: Option<String>,
    /// Exclude specific software items from the batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_item_ids: Option<Vec<Uuid>>,
}
impl Validate for HostBatchUpdateRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self
            .category_filter
            .as_deref()
            .is_some_and(|cat| cat.parse::<UpdateCategory>().is_err())
        {
            return Err(ValidationError {
                field: "category_filter",
                message: "must be a valid update category".to_string(),
            });
        }
        Ok(())
    }
}
/// Request body for `POST /api/v1/software-items/{id}/batch-update`.
///
/// Rolls out a software item to all (or selected) assigned hosts.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ItemBatchUpdateRequest {
    /// Target version to roll out.
    pub to_version: String,
    /// Limit the rollout to specific hosts. `None` means all assigned hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_ids: Option<Vec<Uuid>>,
}
impl Validate for ItemBatchUpdateRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.to_version.trim().is_empty() {
            return Err(ValidationError {
                field: "to_version",
                message: "must not be empty".to_string(),
            });
        }
        Ok(())
    }
}
/// Response returned by batch-update endpoints.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BatchUpdateResponse {
    /// Batch ID, if a batch was created (omitted when `total_created` is 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<Uuid>,
    /// Number of update records created.
    pub total_created: usize,
    /// Per-item update details.
    pub updates: Vec<BatchUpdateItem>,
    /// Items that were skipped (existing active update, missing plugin, etc.).
    pub skipped: Vec<BatchSkippedItem>,
}
/// A single item within a batch update response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BatchUpdateItem {
    pub update_history_id: Uuid,
    pub software_item_id: Uuid,
    pub software_item_name: String,
    pub host_id: Uuid,
    pub host_name: String,
    pub to_version: String,
    pub trigger_status: TriggerUpdateStatus,
}
/// An item that was skipped during batch creation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BatchSkippedItem {
    pub software_item_id: Uuid,
    pub software_item_name: String,
    pub host_id: Uuid,
    pub host_name: String,
    pub reason: String,
}
/// Summary response for listing batches.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateBatchSummaryResponse {
    pub id: Uuid,
    pub batch_type: String,
    pub status: String,
    pub total_count: i32,
    pub completed_count: i64,
    pub failed_count: i64,
    pub pending_count: i64,
    pub actor_type: String,
    pub actor_id: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub completed_at: Option<OffsetDateTime>,
}
/// Detailed batch response including per-item update summaries.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateBatchDetailResponse {
    pub id: Uuid,
    pub batch_type: String,
    pub status: String,
    pub total_count: i32,
    pub completed_count: i64,
    pub failed_count: i64,
    pub pending_count: i64,
    pub actor_type: String,
    pub actor_id: String,
    pub updates: Vec<UpdateBatchItemSummary>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub completed_at: Option<OffsetDateTime>,
}
/// Summary of a single update within a batch.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateBatchItemSummary {
    pub update_history_id: Uuid,
    pub host_id: Uuid,
    pub host_name: String,
    pub software_item_id: Uuid,
    pub software_item_name: String,
    pub to_version: String,
    pub status: String,
    pub update_category: String,
}
/// Query parameters for listing batches.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
pub struct UpdateBatchListQuery {
    /// Filter by batch status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}
impl UpdateBatchListQuery {
    pub fn pagination(&self) -> crate::generated::types::pagination::PaginationParams {
        crate::generated::types::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}
