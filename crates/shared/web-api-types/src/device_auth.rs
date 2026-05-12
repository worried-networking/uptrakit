use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthApproveRequest {
    pub user_code: String,
}

impl Validate for DeviceAuthApproveRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.user_code.trim().is_empty() {
            return Err(ValidationError {
                field: "user_code",
                message: "user_code is required".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthApproveResponse {
    pub message: String,
}
