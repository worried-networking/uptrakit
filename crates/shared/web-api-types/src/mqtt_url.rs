use crate::mqtt_transport::MqttTransport;

/// Error returned when parsing an MQTT URL fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MqttUrlError {
    /// The string is not a valid URL.
    InvalidUrl(String),
    /// The URL scheme is not recognised (expected mqtt, mqtts).
    UnsupportedScheme(String),
    /// The URL has no host component.
    MissingHost,
    /// The port number is out of range.
    InvalidPort,
    /// The URL contains a path, which is not supported for MQTT.
    PathNotSupported,
}

impl std::fmt::Display for MqttUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(e) => write!(f, "invalid URL: {e}"),
            Self::UnsupportedScheme(s) => {
                write!(f, "unsupported URL scheme: {s} (expected mqtt, mqtts)")
            }
            Self::MissingHost => f.write_str("URL must contain a host"),
            Self::InvalidPort => f.write_str("invalid port number"),
            Self::PathNotSupported => f.write_str("URL path is not supported for MQTT"),
        }
    }
}

/// Parsed components of an MQTT URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttUrl {
    pub transport: MqttTransport,
    pub host: String,
    pub port: u16,
}

impl MqttUrl {
    /// Parse an MQTT URL string.
    ///
    /// Accepted schemes: `mqtt://`, `mqtts://`.
    /// If the port is omitted, the default port for the transport is used.
    pub fn parse(url: &str) -> Result<Self, MqttUrlError> {
        // Split scheme manually because url::Url doesn't know mqtt/mqtts schemes
        let (scheme, rest) = url
            .split_once("://")
            .ok_or_else(|| MqttUrlError::InvalidUrl("missing :// separator".to_string()))?;

        let transport = MqttTransport::from_url_scheme(scheme)
            .ok_or_else(|| MqttUrlError::UnsupportedScheme(scheme.to_string()))?;

        // Parse the remainder as authority + optional path
        // rest = "host:port/path" or "host/path" or "host:port" or "host"
        let (authority, path) = match rest.find('/') {
            Some(idx) => {
                let path_str = &rest[idx..];
                let path = if path_str == "/" {
                    None
                } else {
                    Some(path_str.to_string())
                };
                (&rest[..idx], path)
            }
            None => (rest, None),
        };

        if path.is_some() {
            return Err(MqttUrlError::PathNotSupported);
        }

        if authority.is_empty() {
            return Err(MqttUrlError::MissingHost);
        }

        // Handle IPv6 bracket notation: [::1]:port
        let (host, port_str) = if authority.starts_with('[') {
            // IPv6
            match authority.find(']') {
                Some(close) => {
                    let host_part = &authority[1..close];
                    let after = &authority[close + 1..];
                    let port_part = after.strip_prefix(':');
                    (host_part, port_part)
                }
                None => {
                    return Err(MqttUrlError::InvalidUrl(
                        "unclosed bracket in IPv6 address".to_string(),
                    ));
                }
            }
        } else {
            // IPv4 or hostname — last colon separates host:port
            match authority.rfind(':') {
                Some(idx) => (&authority[..idx], Some(&authority[idx + 1..])),
                None => (authority, None),
            }
        };

        if host.is_empty() {
            return Err(MqttUrlError::MissingHost);
        }

        let port = match port_str {
            Some(s) if !s.is_empty() => s.parse::<u16>().map_err(|_| MqttUrlError::InvalidPort)?,
            _ => transport.default_port(),
        };

        Ok(Self {
            transport,
            host: host.to_string(),
            port,
        })
    }

    /// Format back into a URL string.
    pub fn to_url_string(&self) -> String {
        let scheme = self.transport.url_scheme();
        // Omit port if it matches the transport default
        if self.port == self.transport.default_port() {
            format!("{scheme}://{}", self.host)
        } else {
            format!("{scheme}://{}:{}", self.host, self.port)
        }
    }
}

/// Build a URL string from individual components (stored in the DB).
pub fn build_url(transport: MqttTransport, host: &str, port: u16) -> String {
    let url = MqttUrl {
        transport,
        host: host.to_string(),
        port,
    };
    url.to_url_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mqtt_tcp_default_port() {
        let url = MqttUrl::parse("mqtt://broker.local").unwrap();
        assert_eq!(url.transport, MqttTransport::Tcp);
        assert_eq!(url.host, "broker.local");
        assert_eq!(url.port, 1883);
    }

    #[test]
    fn parse_mqtt_tcp_custom_port() {
        let url = MqttUrl::parse("mqtt://broker.local:9883").unwrap();
        assert_eq!(url.transport, MqttTransport::Tcp);
        assert_eq!(url.host, "broker.local");
        assert_eq!(url.port, 9883);
    }

    #[test]
    fn parse_mqtts_default_port() {
        let url = MqttUrl::parse("mqtts://secure.broker").unwrap();
        assert_eq!(url.transport, MqttTransport::Tls);
        assert_eq!(url.host, "secure.broker");
        assert_eq!(url.port, 8883);
    }

    #[test]
    fn parse_trailing_slash_ignored() {
        let url = MqttUrl::parse("mqtt://broker.local/").unwrap();
        assert_eq!(url.transport, MqttTransport::Tcp);
        assert_eq!(url.host, "broker.local");
    }

    #[test]
    fn parse_path_rejected() {
        let err = MqttUrl::parse("mqtt://broker.local/mqtt").unwrap_err();
        assert_eq!(err, MqttUrlError::PathNotSupported);
    }

    #[test]
    fn parse_ipv6_host() {
        let url = MqttUrl::parse("mqtt://[::1]:1883").unwrap();
        assert_eq!(url.host, "::1");
        assert_eq!(url.port, 1883);
    }

    #[test]
    fn parse_error_unsupported_scheme() {
        let err = MqttUrl::parse("http://broker.local").unwrap_err();
        assert_eq!(err, MqttUrlError::UnsupportedScheme("http".to_string()));
    }

    #[test]
    fn parse_error_ws_scheme() {
        let err = MqttUrl::parse("ws://broker.local").unwrap_err();
        assert_eq!(err, MqttUrlError::UnsupportedScheme("ws".to_string()));
    }

    #[test]
    fn parse_error_wss_scheme() {
        let err = MqttUrl::parse("wss://broker.local").unwrap_err();
        assert_eq!(err, MqttUrlError::UnsupportedScheme("wss".to_string()));
    }

    #[test]
    fn parse_error_missing_host() {
        let err = MqttUrl::parse("mqtt://").unwrap_err();
        assert_eq!(err, MqttUrlError::MissingHost);
    }

    #[test]
    fn parse_error_no_separator() {
        let err = MqttUrl::parse("broker.local").unwrap_err();
        matches!(err, MqttUrlError::InvalidUrl(_));
    }

    #[test]
    fn parse_error_invalid_port() {
        let err = MqttUrl::parse("mqtt://broker.local:notaport").unwrap_err();
        assert_eq!(err, MqttUrlError::InvalidPort);
    }

    #[test]
    fn to_url_string_default_port_omitted() {
        let url = MqttUrl {
            transport: MqttTransport::Tcp,
            host: "broker.local".to_string(),
            port: 1883,
        };
        assert_eq!(url.to_url_string(), "mqtt://broker.local");
    }

    #[test]
    fn to_url_string_custom_port_included() {
        let url = MqttUrl {
            transport: MqttTransport::Tcp,
            host: "broker.local".to_string(),
            port: 9883,
        };
        assert_eq!(url.to_url_string(), "mqtt://broker.local:9883");
    }

    #[test]
    fn round_trip_all_schemes() {
        // Default ports are omitted in to_url_string(), so round-trip
        // produces the canonical form without explicit port.
        let cases = [
            ("mqtt://broker:1883", "mqtt://broker"),
            ("mqtts://broker:8883", "mqtts://broker"),
        ];
        for (input, expected) in cases {
            let parsed = MqttUrl::parse(input).unwrap();
            assert_eq!(parsed.to_url_string(), expected);
        }
    }

    #[test]
    fn round_trip_custom_ports() {
        let urls = ["mqtt://broker:9883", "mqtts://broker:9443"];
        for input in urls {
            let parsed = MqttUrl::parse(input).unwrap();
            assert_eq!(parsed.to_url_string(), input);
        }
    }

    #[test]
    fn build_url_helper() {
        assert_eq!(
            build_url(MqttTransport::Tcp, "broker", 1883),
            "mqtt://broker"
        );
        assert_eq!(
            build_url(MqttTransport::Tls, "broker", 9443),
            "mqtts://broker:9443"
        );
    }
}
