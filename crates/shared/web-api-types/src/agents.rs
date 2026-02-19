use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollmentTokenResponse {
    pub token: SecretString,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollmentTokenStatusResponse {
    pub configured: bool,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeAgentRequest {
    pub source_id: Uuid,
}
