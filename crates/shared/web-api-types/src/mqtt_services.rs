use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::SecretString;
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
    pub token: SecretString,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub expires_at: Option<OffsetDateTime>,
    pub uses_remaining: Option<u32>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
}

/// Response for listing MQTT enrollment tokens.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MqttEnrollmentTokenListResponse {
    pub id: Uuid,
    pub name: String,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub expires_at: Option<OffsetDateTime>,
    pub uses_remaining: Option<u32>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
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
    pub uses_remaining: Option<u32>,
}
