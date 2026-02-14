use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

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

/// Error returned when parsing an invalid [`MqttClientConnectionStatus`] string.
#[derive(Debug, Error)]
#[error("invalid MQTT client connection status value")]
pub struct ParseMqttClientConnectionStatusError;

impl MqttClientConnectionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Connecting => "connecting",
        }
    }
}

impl FromStr for MqttClientConnectionStatus {
    type Err = ParseMqttClientConnectionStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "online" => Ok(Self::Online),
            "offline" => Ok(Self::Offline),
            "connecting" => Ok(Self::Connecting),
            _ => Err(ParseMqttClientConnectionStatusError),
        }
    }
}

impl std::fmt::Display for MqttClientConnectionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_status_from_str_valid() {
        assert_eq!(
            "online".parse::<MqttClientConnectionStatus>().ok(),
            Some(MqttClientConnectionStatus::Online)
        );
        assert_eq!(
            "offline".parse::<MqttClientConnectionStatus>().ok(),
            Some(MqttClientConnectionStatus::Offline)
        );
        assert_eq!(
            "connecting".parse::<MqttClientConnectionStatus>().ok(),
            Some(MqttClientConnectionStatus::Connecting)
        );
    }

    #[test]
    fn connection_status_from_str_invalid() {
        assert!("unknown".parse::<MqttClientConnectionStatus>().is_err());
        assert!("".parse::<MqttClientConnectionStatus>().is_err());
        assert!("ONLINE".parse::<MqttClientConnectionStatus>().is_err());
    }

    #[test]
    fn connection_status_as_str_round_trips_through_from_str() {
        for status in [
            MqttClientConnectionStatus::Online,
            MqttClientConnectionStatus::Offline,
            MqttClientConnectionStatus::Connecting,
        ] {
            let s = status.as_str();
            let parsed: MqttClientConnectionStatus =
                s.parse().expect("from_str should succeed for as_str output");
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn connection_status_display_matches_as_str() {
        for status in [
            MqttClientConnectionStatus::Online,
            MqttClientConnectionStatus::Offline,
            MqttClientConnectionStatus::Connecting,
        ] {
            assert_eq!(format!("{status}"), status.as_str());
        }
    }

    #[test]
    fn connection_status_default_is_offline() {
        assert_eq!(
            MqttClientConnectionStatus::default(),
            MqttClientConnectionStatus::Offline
        );
    }

    #[test]
    fn connection_status_serde_round_trip() {
        for status in [
            MqttClientConnectionStatus::Online,
            MqttClientConnectionStatus::Offline,
            MqttClientConnectionStatus::Connecting,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: MqttClientConnectionStatus =
                serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status);
        }
    }
}
