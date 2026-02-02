use serde::{Deserialize, Serialize};

/// MQTT connection transport type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum MqttTransport {
    #[default]
    Tcp,
    Tls,
    Ws,
    Wss,
}

impl MqttTransport {
    /// DB / wire representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Tls => "tls",
            Self::Ws => "ws",
            Self::Wss => "wss",
        }
    }

    /// Parse from a DB / wire string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tcp" => Some(Self::Tcp),
            "tls" => Some(Self::Tls),
            "ws" => Some(Self::Ws),
            "wss" => Some(Self::Wss),
            _ => None,
        }
    }

    /// Default port for this transport.
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Tcp => 1883,
            Self::Tls => 8883,
            Self::Ws => 80,
            Self::Wss => 443,
        }
    }

    /// URL scheme for this transport.
    pub const fn url_scheme(self) -> &'static str {
        match self {
            Self::Tcp => "mqtt",
            Self::Tls => "mqtts",
            Self::Ws => "ws",
            Self::Wss => "wss",
        }
    }

    /// Parse from a URL scheme string.
    pub fn from_url_scheme(scheme: &str) -> Option<Self> {
        match scheme {
            "mqtt" => Some(Self::Tcp),
            "mqtts" => Some(Self::Tls),
            "ws" => Some(Self::Ws),
            "wss" => Some(Self::Wss),
            _ => None,
        }
    }

    /// Whether this transport uses TLS.
    pub const fn requires_tls(self) -> bool {
        matches!(self, Self::Tls | Self::Wss)
    }

    /// Whether this transport uses WebSocket framing.
    pub const fn is_websocket(self) -> bool {
        matches!(self, Self::Ws | Self::Wss)
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
        for transport in [
            MqttTransport::Tcp,
            MqttTransport::Tls,
            MqttTransport::Ws,
            MqttTransport::Wss,
        ] {
            let s = transport.as_str();
            let parsed = MqttTransport::parse(s);
            assert_eq!(parsed, Some(transport), "round-trip failed for {s}");
        }
    }

    #[test]
    fn serde_round_trip() {
        for transport in [
            MqttTransport::Tcp,
            MqttTransport::Tls,
            MqttTransport::Ws,
            MqttTransport::Wss,
        ] {
            let json = serde_json::to_string(&transport).unwrap();
            let deserialized: MqttTransport = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, transport);
        }
    }

    #[test]
    fn default_ports() {
        assert_eq!(MqttTransport::Tcp.default_port(), 1883);
        assert_eq!(MqttTransport::Tls.default_port(), 8883);
        assert_eq!(MqttTransport::Ws.default_port(), 80);
        assert_eq!(MqttTransport::Wss.default_port(), 443);
    }

    #[test]
    fn url_scheme_round_trip() {
        for transport in [
            MqttTransport::Tcp,
            MqttTransport::Tls,
            MqttTransport::Ws,
            MqttTransport::Wss,
        ] {
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
    fn tls_and_websocket_flags() {
        assert!(!MqttTransport::Tcp.requires_tls());
        assert!(MqttTransport::Tls.requires_tls());
        assert!(!MqttTransport::Ws.requires_tls());
        assert!(MqttTransport::Wss.requires_tls());

        assert!(!MqttTransport::Tcp.is_websocket());
        assert!(!MqttTransport::Tls.is_websocket());
        assert!(MqttTransport::Ws.is_websocket());
        assert!(MqttTransport::Wss.is_websocket());
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(MqttTransport::parse("unknown"), None);
        assert_eq!(MqttTransport::parse(""), None);
        assert_eq!(MqttTransport::parse("TCP"), None);
    }

    #[test]
    fn from_url_scheme_unknown_returns_none() {
        assert_eq!(MqttTransport::from_url_scheme("http"), None);
        assert_eq!(MqttTransport::from_url_scheme(""), None);
    }

    #[test]
    fn display_matches_as_str() {
        for transport in [
            MqttTransport::Tcp,
            MqttTransport::Tls,
            MqttTransport::Ws,
            MqttTransport::Wss,
        ] {
            assert_eq!(format!("{transport}"), transport.as_str());
        }
    }

    #[test]
    fn default_is_tcp() {
        assert_eq!(MqttTransport::default(), MqttTransport::Tcp);
    }
}
