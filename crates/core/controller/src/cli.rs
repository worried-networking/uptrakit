use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use ipnet::IpNet;

/// Uptrakit Controller — central server for the Uptrakit update tracking toolkit.
#[derive(Parser, Debug)]
#[command(name = "uptrakit-controller")]
pub struct Args {
    /// Data directory (CA keys, certs, future DB).
    /// Supports `~` for home directory expansion.
    #[arg(long, default_value = "~/.uptrakit")]
    pub data_dir: String,

    /// Database URL. If not provided, defaults to SQLite in data directory.
    /// Supported schemes depend on enabled features:
    ///   SQLite (default): sqlite://path/to/db.sqlite
    ///   PostgreSQL: postgresql://user:pass@host:5432/dbname
    ///   MySQL: mysql://user:pass@host:3306/dbname
    #[arg(long)]
    pub db_url: Option<String>,

    /// HTTP listen address (dual-stack by default).
    #[arg(long, default_value = "[::]:8080")]
    pub http_addr: SocketAddr,

    /// HTTPS listen address (dual-stack by default).
    #[arg(long, default_value = "[::]:8443")]
    pub https_addr: SocketAddr,

    /// Override: path to PEM-encoded server certificate.
    /// If not provided, a certificate is auto-generated using the internal CA.
    #[arg(long)]
    pub tls_cert: Option<PathBuf>,

    /// Override: path to PEM-encoded server private key.
    /// Must be provided together with --tls-cert.
    #[arg(long)]
    pub tls_key: Option<PathBuf>,

    /// Trusted reverse proxy IP or CIDR (repeatable).
    /// Bare IPs are treated as /32 (IPv4) or /128 (IPv6).
    /// Both IPv4 and IPv6 are supported.
    #[arg(long = "trusted-proxy", value_parser = parse_proxy)]
    pub trusted_proxies: Vec<IpNet>,

    /// Header to extract the real client IP from when behind a trusted proxy.
    /// Supported: X-Forwarded-For (default), Forwarded (RFC 7239), X-Real-Ip,
    /// or any custom header name (parsed as comma-separated IPs).
    #[arg(long, default_value = "X-Forwarded-For")]
    pub real_ip_header: String,

    /// Override: path to PEM-encoded CA certificate.
    /// If not provided, a CA is auto-generated and managed internally.
    /// Must be provided together with --ca-key.
    #[arg(long)]
    pub ca_cert: Option<PathBuf>,

    /// Override: path to PEM-encoded CA private key.
    /// Must be provided together with --ca-cert.
    #[arg(long)]
    pub ca_key: Option<PathBuf>,

    /// Additional SAN for the generated server certificate (IP or DNS name, repeatable).
    #[arg(long = "san")]
    pub sans: Vec<String>,

    /// Path to the built frontend directory. Enables SPA serving.
    #[arg(long)]
    pub static_dir: Option<PathBuf>,

    #[cfg(feature = "mqtt")]
    #[command(flatten)]
    pub mqtt: MqttArgs,
}

/// MQTT broker connection options.
#[cfg(feature = "mqtt")]
#[derive(Parser, Debug)]
pub struct MqttArgs {
    /// MQTT broker host. Required to enable MQTT.
    #[arg(long)]
    pub mqtt_host: Option<String>,

    /// MQTT broker port.
    #[arg(long, default_value_t = 1883)]
    pub mqtt_port: u16,

    /// MQTT client ID.
    #[arg(long, default_value = "uptrakit-controller")]
    pub mqtt_client_id: String,

    /// MQTT username.
    #[arg(long)]
    pub mqtt_username: Option<String>,

    /// MQTT password. Requires --mqtt-username.
    #[arg(long)]
    pub mqtt_password: Option<String>,

    /// MQTT topic prefix.
    #[arg(long, default_value = "uptrakit")]
    pub mqtt_topic_prefix: String,
}

/// Parse a trusted proxy argument. Accepts:
/// - Bare IP: `192.168.1.1` -> `192.168.1.1/32`
/// - Bare IPv6: `::1` -> `::1/128`
/// - CIDR: `10.0.0.0/8`, `fd00::/8`
fn parse_proxy(s: &str) -> Result<IpNet, String> {
    // Try CIDR first
    if let Ok(net) = s.parse::<IpNet>() {
        return Ok(net);
    }
    // Try bare IP -> host network
    if let Ok(ip) = s.parse::<std::net::IpAddr>() {
        return Ok(IpNet::from(ip));
    }
    Err(format!("invalid IP or CIDR: {s}"))
}

impl Args {
    /// Resolve `data_dir` by expanding `~` to the user's home directory.
    pub fn resolve_data_dir(&self) -> Result<PathBuf, String> {
        let path = if self.data_dir.starts_with("~/") {
            let home = home_dir().ok_or("could not determine home directory")?;
            home.join(&self.data_dir[2..])
        } else if self.data_dir == "~" {
            home_dir().ok_or("could not determine home directory")?
        } else {
            PathBuf::from(&self.data_dir)
        };
        Ok(path)
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[cfg(feature = "mqtt")]
    use clap::Parser;
    use ipnet::IpNet;

    use super::parse_proxy;

    #[test]
    fn bare_ipv4_parsed_as_host() {
        let net = parse_proxy("192.168.1.1").unwrap();
        let expected: IpNet = "192.168.1.1/32".parse().unwrap();
        assert_eq!(net, expected);
    }

    #[test]
    fn cidr_notation_parsed() {
        let net = parse_proxy("10.0.0.0/8").unwrap();
        let expected: IpNet = "10.0.0.0/8".parse().unwrap();
        assert_eq!(net, expected);
    }

    #[test]
    fn invalid_proxy_string() {
        assert!(parse_proxy("not-an-ip").is_err());
    }

    #[test]
    fn ipv6_bare_address() {
        let net = parse_proxy("::1").unwrap();
        assert_eq!(net, IpNet::from(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn ipv6_cidr() {
        let net = parse_proxy("fd00::/8").unwrap();
        let expected: IpNet = "fd00::/8".parse().unwrap();
        assert_eq!(net, expected);
    }

    #[test]
    fn empty_proxy_list() {
        // Just verifying parse_proxy isn't called with empty strings accidentally
        assert!(parse_proxy("").is_err());
    }

    #[cfg(feature = "mqtt")]
    #[test]
    fn mqtt_args_not_set_by_default() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(args.mqtt.mqtt_host.is_none());
        assert_eq!(args.mqtt.mqtt_port, 1883);
        assert_eq!(args.mqtt.mqtt_client_id, "uptrakit-controller");
        assert!(args.mqtt.mqtt_username.is_none());
        assert!(args.mqtt.mqtt_password.is_none());
        assert_eq!(args.mqtt.mqtt_topic_prefix, "uptrakit");
    }

    #[cfg(feature = "mqtt")]
    #[test]
    fn mqtt_args_custom_values() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--mqtt-host",
            "broker.local",
            "--mqtt-port",
            "8883",
            "--mqtt-client-id",
            "my-controller",
            "--mqtt-username",
            "user",
            "--mqtt-password",
            "pass",
            "--mqtt-topic-prefix",
            "home/uptrakit",
        ])
        .expect("should parse custom values");

        assert_eq!(args.mqtt.mqtt_host.as_deref(), Some("broker.local"));
        assert_eq!(args.mqtt.mqtt_port, 8883);
        assert_eq!(args.mqtt.mqtt_client_id, "my-controller");
        assert_eq!(args.mqtt.mqtt_username.as_deref(), Some("user"));
        assert_eq!(args.mqtt.mqtt_password.as_deref(), Some("pass"));
        assert_eq!(args.mqtt.mqtt_topic_prefix, "home/uptrakit");
    }

    #[test]
    fn bare_ipv4_contains_only_self() {
        let net = parse_proxy("192.168.1.1").unwrap();
        assert!(net.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(!net.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2))));
    }
}
