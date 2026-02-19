use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateApiTokenRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateApiTokenResponse {
    pub id: Uuid,
    pub token: SecretString,
    pub created_at: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApiTokenResponse {
    pub id: Uuid,
    pub name: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApiTokenListResponse {
    pub tokens: Vec<ApiTokenResponse>,
}
