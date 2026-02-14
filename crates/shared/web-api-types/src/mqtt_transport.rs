use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// MQTT connection transport type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum MqttTransport {
    #[default]
    Tcp,
    Tls,
}

/// Error returned when parsing an invalid [`MqttTransport`] string.
#[derive(Debug, Error)]
#[error("invalid MQTT transport value")]
pub struct ParseMqttTransportError;

impl MqttTransport {
    /// DB / wire representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Tls => "tls",
        }
    }

    /// Default port for this transport.
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Tcp => 1883,
            Self::Tls => 8883,
        }
    }

    /// URL scheme for this transport.
    pub const fn url_scheme(self) -> &'static str {
        match self {
            Self::Tcp => "mqtt",
            Self::Tls => "mqtts",
        }
    }

    /// Parse from a URL scheme string.
    pub fn from_url_scheme(scheme: &str) -> Option<Self> {
        match scheme {
            "mqtt" => Some(Self::Tcp),
            "mqtts" => Some(Self::Tls),
            _ => None,
        }
    }

    /// Whether this transport uses TLS.
    pub const fn requires_tls(self) -> bool {
        matches!(self, Self::Tls)
    }
}

impl FromStr for MqttTransport {
    type Err = ParseMqttTransportError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tcp" => Ok(Self::Tcp),
            "tls" => Ok(Self::Tls),
            _ => Err(ParseMqttTransportError),
        }
    }
}

impl std::fmt::Display for MqttTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_variants() {
        for transport in [MqttTransport::Tcp, MqttTransport::Tls] {
            let s = transport.as_str();
            let parsed: MqttTransport = s.parse().expect("round-trip should succeed");
            assert_eq!(parsed, transport, "round-trip failed for {s}");
        }
    }

    #[test]
    fn serde_round_trip() {
        for transport in [MqttTransport::Tcp, MqttTransport::Tls] {
            let json = serde_json::to_string(&transport).unwrap();
            let deserialized: MqttTransport = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, transport);
        }
    }

    #[test]
    fn default_ports() {
        assert_eq!(MqttTransport::Tcp.default_port(), 1883);
        assert_eq!(MqttTransport::Tls.default_port(), 8883);
    }

    #[test]
    fn url_scheme_round_trip() {
        for transport in [MqttTransport::Tcp, MqttTransport::Tls] {
            let scheme = transport.url_scheme();
            let parsed = MqttTransport::from_url_scheme(scheme);
            assert_eq!(
                parsed,
                Some(transport),
                "scheme round-trip failed for {scheme}"
            );
        }
    }

    #[test]
    fn tls_flags() {
        assert!(!MqttTransport::Tcp.requires_tls());
        assert!(MqttTransport::Tls.requires_tls());
    }

    #[test]
    fn from_str_unknown_returns_err() {
        assert!("unknown".parse::<MqttTransport>().is_err());
        assert!("".parse::<MqttTransport>().is_err());
        assert!("TCP".parse::<MqttTransport>().is_err());
    }

    #[test]
    fn from_url_scheme_unknown_returns_none() {
        assert_eq!(MqttTransport::from_url_scheme("http"), None);
        assert_eq!(MqttTransport::from_url_scheme(""), None);
    }

    #[test]
    fn display_matches_as_str() {
        for transport in [MqttTransport::Tcp, MqttTransport::Tls] {
            assert_eq!(format!("{transport}"), transport.as_str());
        }
    }

    #[test]
    fn default_is_tcp() {
        assert_eq!(MqttTransport::default(), MqttTransport::Tcp);
    }
}
