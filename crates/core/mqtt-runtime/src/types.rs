use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// MQTT broker transport protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MqttTransport {
    #[default]
    Tcp,
    Tls,
}

impl MqttTransport {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Tls => "tls",
        }
    }

    pub(crate) const fn default_port(self) -> u16 {
        match self {
            Self::Tcp => 1883,
            Self::Tls => 8883,
        }
    }
}

impl fmt::Display for MqttTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MqttTransport {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tcp" => Ok(Self::Tcp),
            "tls" => Ok(Self::Tls),
            _ => Err("invalid MQTT transport value".to_string()),
        }
    }
}

/// Connection status of an MQTT client managed by the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MqttClientConnectionStatus {
    Online,
    #[default]
    Offline,
    Connecting,
}

impl MqttClientConnectionStatus {
    pub const fn as_str(self) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::{MqttClientConnectionStatus, MqttTransport};

    #[test]
    fn transport_default_ports_match_protocol() {
        assert_eq!(MqttTransport::Tcp.default_port(), 1883);
        assert_eq!(MqttTransport::Tls.default_port(), 8883);
    }

    #[test]
    fn transport_display_matches_wire_values() {
        assert_eq!(MqttTransport::Tcp.to_string(), "tcp");
        assert_eq!(MqttTransport::Tls.to_string(), "tls");
    }

    #[test]
    fn status_display_matches_expected_values() {
        assert_eq!(MqttClientConnectionStatus::Online.to_string(), "online");
        assert_eq!(MqttClientConnectionStatus::Offline.to_string(), "offline");
        assert_eq!(
            MqttClientConnectionStatus::Connecting.to_string(),
            "connecting"
        );
    }
}
