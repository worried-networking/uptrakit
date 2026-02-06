use clap::Parser;
use uptrakit_enrollment::cli::CommonServiceArgs;

/// Uptrakit MQTT Service — WebSocket-connected MQTT client service.
///
/// Connects to the controller via mTLS WebSocket for tenant configuration
/// and lease coordination.
#[derive(Parser, Debug)]
#[command(name = "uptrakit-mqtt")]
pub struct Args {
    #[command(flatten)]
    pub common: CommonServiceArgs,

    /// Maximum number of tenants this instance will manage.
    /// 0 means unlimited.
    #[arg(long, default_value = "0")]
    pub max_tenants: u32,

    /// Heartbeat interval in seconds.
    #[arg(long, default_value = "15")]
    pub heartbeat_interval: u64,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn defaults_parse() {
        let args =
            Args::try_parse_from(["uptrakit-mqtt", "--url", "https://localhost:8443"]).unwrap();
        assert_eq!(args.common.url, "https://localhost:8443");
        assert!(args.common.config_dir.is_none());
        assert!(args.common.state_dir.is_none());
        assert_eq!(args.max_tenants, 0);
        assert_eq!(args.heartbeat_interval, 15);
        assert!(args.common.enrollment_token.is_none());
        assert!(args.common.friendly_name.is_none());
        assert!(!args.common.tofu);
    }

    #[test]
    fn custom_values_parsed() {
        let args = Args::try_parse_from([
            "uptrakit-mqtt",
            "--url",
            "https://controller.example.com:9443",
            "--config-dir",
            "/opt/mqtt-config",
            "--state-dir",
            "/opt/mqtt-state",
            "--max-tenants",
            "5",
            "--heartbeat-interval",
            "30",
            "--enrollment-token",
            "secret-token-123",
            "--friendly-name",
            "Production MQTT Node 1",
            "--tofu",
        ])
        .unwrap();
        assert_eq!(args.common.url, "https://controller.example.com:9443");
        assert_eq!(
            args.common.config_dir.as_ref().unwrap().to_str().unwrap(),
            "/opt/mqtt-config"
        );
        assert_eq!(
            args.common.state_dir.as_ref().unwrap().to_str().unwrap(),
            "/opt/mqtt-state"
        );
        assert_eq!(args.max_tenants, 5);
        assert_eq!(args.heartbeat_interval, 30);
        assert_eq!(
            args.common.enrollment_token.as_deref(),
            Some("secret-token-123")
        );
        assert_eq!(
            args.common.friendly_name.as_deref(),
            Some("Production MQTT Node 1")
        );
        assert!(args.common.tofu);
    }

    #[test]
    fn url_required() {
        let result = Args::try_parse_from(["uptrakit-mqtt"]);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_dirs_with_defaults() {
        let args = Args::try_parse_from(["uptrakit-mqtt", "--url", "https://localhost:8443"])
            .expect("should parse defaults");
        let dirs = args
            .common
            .resolve_dirs("mqtt")
            .expect("should resolve dirs");
        assert!(!dirs.config_dir().as_os_str().is_empty());
        assert!(!dirs.state_dir().as_os_str().is_empty());
    }

    #[test]
    fn resolve_dirs_with_overrides() {
        let args = Args::try_parse_from([
            "uptrakit-mqtt",
            "--url",
            "https://localhost:8443",
            "--config-dir",
            "/custom/config",
            "--state-dir",
            "/custom/state",
        ])
        .expect("should parse");
        let dirs = args
            .common
            .resolve_dirs("mqtt")
            .expect("should resolve dirs");
        assert_eq!(dirs.config_dir().to_str().unwrap(), "/custom/config");
        assert_eq!(dirs.state_dir().to_str().unwrap(), "/custom/state");
    }

    #[test]
    fn friendly_name_or_hostname_returns_provided() {
        let args = Args::try_parse_from([
            "uptrakit-mqtt",
            "--url",
            "https://localhost:8443",
            "--friendly-name",
            "My Node",
        ])
        .unwrap();
        assert_eq!(args.common.friendly_name_or_hostname(), "My Node");
    }

    #[test]
    fn friendly_name_or_hostname_falls_back_to_hostname() {
        let args =
            Args::try_parse_from(["uptrakit-mqtt", "--url", "https://localhost:8443"]).unwrap();
        assert!(!args.common.friendly_name_or_hostname().is_empty());
    }
}
