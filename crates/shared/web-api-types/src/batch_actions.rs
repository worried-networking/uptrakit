use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// Maximum number of IDs allowed in a single batch action request.
pub const MAX_BATCH_SIZE: usize = 100;

/// Request body for batch action endpoints.
///
/// Each resource's batch endpoint (`POST /api/v1/{resource}/batch`) accepts
/// this payload. The `action` string selects the operation (e.g. `"feature"`,
/// `"unfeature"`, `"delete"`) and `ids` lists the target entity UUIDs.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BatchActionResponse {
    /// Items that were successfully processed.
    pub succeeded: Vec<BatchActionSuccess>,
    /// Items that failed, with per-item error messages.
    pub failed: Vec<BatchActionFailure>,
}

/// A successfully processed item in a batch action.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BatchActionSuccess {
    /// The UUID of the successfully processed entity.
    pub id: Uuid,
}

/// A failed item in a batch action.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BatchActionFailure {
    /// The UUID of the entity that failed.
    pub id: Uuid,
    /// Human-readable error message describing why the action failed.
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_empty_action() {
        let req = BatchActionRequest {
            action: String::new(),
            ids: vec![Uuid::nil()],
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "action");
    }

    #[test]
    fn validate_rejects_empty_ids() {
        let req = BatchActionRequest {
            action: "approve".to_string(),
            ids: vec![],
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "ids");
    }

    #[test]
    fn validate_rejects_over_max_ids() {
        let ids: Vec<Uuid> = (0..=MAX_BATCH_SIZE).map(|_| Uuid::new_v4()).collect();
        let req = BatchActionRequest {
            action: "approve".to_string(),
            ids,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "ids");
        assert!(err.message.contains(&MAX_BATCH_SIZE.to_string()));
    }

    #[test]
    fn validate_accepts_valid_request() {
        let req = BatchActionRequest {
            action: "approve".to_string(),
            ids: vec![Uuid::nil()],
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_max_ids() {
        let ids: Vec<Uuid> = (0..MAX_BATCH_SIZE).map(|_| Uuid::new_v4()).collect();
        let req = BatchActionRequest {
            action: "delete".to_string(),
            ids,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn batch_action_request_round_trip() {
        let req = BatchActionRequest {
            action: "approve".to_string(),
            ids: vec![Uuid::nil()],
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let parsed: BatchActionRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.action, "approve");
        assert_eq!(parsed.ids.len(), 1);
    }

    #[test]
    fn batch_action_response_round_trip() {
        let resp = BatchActionResponse {
            succeeded: vec![BatchActionSuccess { id: Uuid::nil() }],
            failed: vec![BatchActionFailure {
                id: Uuid::nil(),
                error: "not found".to_string(),
            }],
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let parsed: BatchActionResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.succeeded.len(), 1);
        assert_eq!(parsed.failed.len(), 1);
        assert_eq!(parsed.failed[0].error, "not found");
    }
}
