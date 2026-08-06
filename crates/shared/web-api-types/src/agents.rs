use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeAgentRequest {
    pub source_id: Uuid,
}

impl Validate for MergeAgentRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        // No format/length invariants beyond field types; capability/existence checks are handler-side.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_agent_request_validate_is_ok() {
        let req = MergeAgentRequest {
            source_id: Uuid::nil(),
        };
        req.validate().expect("MergeAgentRequest should validate");
    }
}
