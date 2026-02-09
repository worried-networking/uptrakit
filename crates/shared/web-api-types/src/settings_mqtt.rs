use serde::{Deserialize, Serialize};

use crate::mqtt_transport::MqttTransport;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum MqttClientConnectionStatus {
    Online,
    #[default]
    Offline,
    Connecting,
}

impl MqttClientConnectionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Connecting => "connecting",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "online" => Some(Self::Online),
            "offline" => Some(Self::Offline),
            "connecting" => Some(Self::Connecting),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MqttLimitResponse {
    pub max_clients_per_tenant: u16,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateMqttLimitRequest {
    pub max_clients_per_tenant: u16,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MqttClientResponse {
    pub id: String,
    pub enabled: bool,
    pub transport: MqttTransport,
    pub host: String,
    pub port: u16,
    pub url: String,
    pub client_id: String,
    pub username: Option<String>,
    pub has_password: bool,
    pub topic_prefix: String,
    pub connection_status: MqttClientConnectionStatus,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateMqttClientRequest {
    /// MQTT URL (e.g. `mqtt://broker:1883`, `mqtts://broker:8883`).
    /// If provided, `transport`, `host`, and `port` are extracted from it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<MqttTransport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_prefix: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateMqttClientRequest {
    /// MQTT URL — if provided, overrides `transport`, `host`, `port`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<MqttTransport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Set to `null` to clear, omit to keep existing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_prefix: Option<String>,
}
