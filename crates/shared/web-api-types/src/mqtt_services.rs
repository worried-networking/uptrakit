use serde::{Deserialize, Serialize};

/// Status of an MQTT service instance in the enrollment/approval workflow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MqttServiceStatus {
    Pending,
    Approved,
    Rejected,
    Deactivated,
}

impl MqttServiceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Deactivated => "deactivated",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            "deactivated" => Some(Self::Deactivated),
            _ => None,
        }
    }
}

/// Response for an MQTT service instance.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MqttServiceResponse {
    pub id: String,
    pub hostname: String,
    pub friendly_name: String,
    pub status: MqttServiceStatus,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

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
    pub id: String,
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
    pub id: String,
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
