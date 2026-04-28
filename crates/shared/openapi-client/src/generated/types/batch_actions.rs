// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
/// Maximum number of IDs allowed in a single batch action request.
pub const MAX_BATCH_SIZE: usize = 100;
/// Request body for batch action endpoints.
///
/// Each resource's batch endpoint (`POST /api/v1/{resource}/batch`) accepts
/// this payload. The `action` string selects the operation (e.g. `"feature"`,
/// `"unfeature"`, `"delete"`) and `ids` lists the target entity UUIDs.
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchActionRequest {
    /// The action to perform (e.g. `"approve"`, `"reject"`, `"delete"`).
    pub action: String,
    /// The UUIDs of the entities to act upon.
    pub ids: Vec<Uuid>,
}
impl Validate for BatchActionRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.action.is_empty() {
            return Err(ValidationError {
                field: "action",
                message: "action must not be empty".to_string(),
            });
        }
        if self.ids.is_empty() {
            return Err(ValidationError {
                field: "ids",
                message: "ids must not be empty".to_string(),
            });
        }
        if self.ids.len() > MAX_BATCH_SIZE {
            return Err(ValidationError {
                field: "ids",
                message: format!("ids must contain at most {MAX_BATCH_SIZE} entries"),
            });
        }
        Ok(())
    }
}
/// Response body for batch action endpoints.
///
/// Reports per-item results, allowing partial success. Callers should inspect
/// both `succeeded` and `failed` to determine the outcome of each item.
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchActionResponse {
    /// Items that were successfully processed.
    pub succeeded: Vec<BatchActionSuccess>,
    /// Items that failed, with per-item error messages.
    pub failed: Vec<BatchActionFailure>,
}
/// A successfully processed item in a batch action.
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchActionSuccess {
    /// The UUID of the successfully processed entity.
    pub id: Uuid,
}
/// A failed item in a batch action.
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchActionFailure {
    /// The UUID of the entity that failed.
    pub id: Uuid,
    /// Human-readable error message describing why the action failed.
    pub error: String,
}
