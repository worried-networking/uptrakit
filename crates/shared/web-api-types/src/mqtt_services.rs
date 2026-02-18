use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Query parameters for listing MQTT services.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListMqttServicesQuery {
    pub status: Option<String>,
}

/// Response for a created MQTT enrollment token.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MqttEnrollmentTokenResponse {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    pub expires_at: Option<String>,
    pub uses_remaining: Option<i32>,
    pub created_at: String,
}

/// Response for listing MQTT enrollment tokens.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MqttEnrollmentTokenListResponse {
    pub id: Uuid,
    pub name: String,
    pub expires_at: Option<String>,
    pub uses_remaining: Option<i32>,
    pub created_at: String,
}

/// Request to create a new MQTT enrollment token.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateMqttEnrollmentTokenRequest {
    pub name: String,
    /// Optional expiration time in RFC 3339 format.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// Optional maximum number of uses.
    #[serde(default)]
    pub uses_remaining: Option<i32>,
}
