use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Connection status of an MQTT client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(
    feature = "sea-orm",
    sea_orm(rs_type = "String", db_type = "Text")
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MqttClientConnectionStatus {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "online"))]
    Online,
    #[default]
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "offline"))]
    Offline,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "connecting"))]
    Connecting,
}

impl MqttClientConnectionStatus {
    /// Returns the string representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Connecting => "connecting",
        }
    }
}

impl fmt::Display for MqttClientConnectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid [`MqttClientConnectionStatus`] string.
#[derive(Debug)]
pub struct ParseMqttClientConnectionStatusError;

impl fmt::Display for ParseMqttClientConnectionStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid MQTT client connection status value")
    }
}

impl std::error::Error for ParseMqttClientConnectionStatusError {}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [
            MqttClientConnectionStatus::Online,
            MqttClientConnectionStatus::Offline,
            MqttClientConnectionStatus::Connecting,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: MqttClientConnectionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in [
            MqttClientConnectionStatus::Online,
            MqttClientConnectionStatus::Offline,
            MqttClientConnectionStatus::Connecting,
        ] {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for variant in [
            MqttClientConnectionStatus::Online,
            MqttClientConnectionStatus::Offline,
            MqttClientConnectionStatus::Connecting,
        ] {
            let s = variant.as_str();
            let parsed: MqttClientConnectionStatus = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<MqttClientConnectionStatus>().is_err());
        assert!("".parse::<MqttClientConnectionStatus>().is_err());
        assert!("ONLINE".parse::<MqttClientConnectionStatus>().is_err());
    }

    #[test]
    fn default_is_offline() {
        assert_eq!(
            MqttClientConnectionStatus::default(),
            MqttClientConnectionStatus::Offline
        );
    }

    #[test]
    fn serde_values() {
        assert_eq!(
            serde_json::to_string(&MqttClientConnectionStatus::Online).unwrap(),
            r#""online""#
        );
        assert_eq!(
            serde_json::to_string(&MqttClientConnectionStatus::Offline).unwrap(),
            r#""offline""#
        );
        assert_eq!(
            serde_json::to_string(&MqttClientConnectionStatus::Connecting).unwrap(),
            r#""connecting""#
        );
    }
}
