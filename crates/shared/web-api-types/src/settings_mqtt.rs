use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

use crate::mqtt_transport::MqttTransport;

// Re-export from shared-types for backward compatibility.
pub use uptrakit_shared_types::{MqttClientConnectionStatus, ParseMqttClientConnectionStatusError};

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
    pub id: Uuid,
    pub enabled: bool,
    pub transport: MqttTransport,
    pub host: String,
    pub port: u16,
    pub url: String,
    pub client_id: String,
    pub username: Option<String>,
    pub has_password: bool,
    pub has_ca_cert: bool,
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
    pub password: Option<SecretString>,
    /// Custom CA certificate in PEM format for TLS connections to private brokers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<SecretString>,
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
    /// Set to `null` to clear, omit to keep existing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<serde_json::Value>,
    /// Set to `null` to clear, omit to keep existing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<serde_json::Value>,
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
            let parsed: MqttClientConnectionStatus = s
                .parse()
                .expect("from_str should succeed for as_str output");
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
            let deserialized: MqttClientConnectionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status);
        }
    }
}
