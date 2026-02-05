use std::path::PathBuf;

use clap::Parser;
use uptrakit_directories::AppDirs;

/// Uptrakit MQTT Service — WebSocket-connected MQTT client service.
///
/// Connects to the controller via mTLS WebSocket for tenant configuration
/// and lease coordination.
#[derive(Parser, Debug)]
#[command(name = "uptrakit-mqtt")]
pub struct Args {
    /// Controller WebSocket URL (e.g., wss://controller:8443).
    /// Used for both enrollment and runtime communication.
    #[arg(long, env = "UPTRAKIT_CONTROLLER_URL")]
    pub controller_url: String,

    /// Config directory for persistent configuration (controller's CA cert).
    /// Supports `~` for home directory expansion.
    /// Default: platform-specific (e.g., ~/.config/mqtt on Linux).
    #[arg(long, env = "UPTRAKIT_CONFIG_DIR")]
    pub config_dir: Option<PathBuf>,

    /// State directory for runtime state (service_id, keypair, certificate).
    /// Supports `~` for home directory expansion.
    /// Default: platform-specific (e.g., ~/.local/state/mqtt on Linux).
    #[arg(long, env = "UPTRAKIT_STATE_DIR")]
    pub state_dir: Option<PathBuf>,

    /// Maximum number of tenants this instance will manage.
    /// 0 means unlimited.
    #[arg(long, default_value = "0")]
    pub max_tenants: u32,

    /// Heartbeat interval in seconds.
    #[arg(long, default_value = "15")]
    pub heartbeat_interval: u64,

    /// Enrollment token for auto-approval.
    /// If provided and valid, the service is approved immediately.
    #[arg(long, env = "UPTRAKIT_ENROLLMENT_TOKEN")]
    pub enrollment_token: Option<String>,

    /// Friendly name for this service instance.
    /// Defaults to hostname if not specified.
    #[arg(long)]
    pub friendly_name: Option<String>,

    /// Skip TLS certificate verification (DANGEROUS).
    /// Only use for initial CA trust establishment or testing.
    #[arg(long, default_value = "false")]
    pub insecure: bool,
}

impl Args {
    /// Resolve application directories using platform-specific defaults.
    ///
    /// Returns `AppDirs` with separate config and state directories.
    /// CLI overrides take precedence over platform defaults.
    pub fn resolve_dirs(&self) -> uptrakit_directories::Result<AppDirs> {
        AppDirs::resolve(
            "mqtt",
            self.config_dir.as_deref(),
            self.state_dir.as_deref(),
        )
    }

    /// Get the friendly name, falling back to hostname.
    pub fn friendly_name_or_hostname(&self) -> String {
        self.friendly_name.clone().unwrap_or_else(|| {
            hostname::get()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn defaults_parse() {
        let args =
            Args::try_parse_from(["uptrakit-mqtt", "--controller-url", "wss://localhost:8443"])
                .unwrap();
        assert_eq!(args.controller_url, "wss://localhost:8443");
        assert!(args.config_dir.is_none());
        assert!(args.state_dir.is_none());
        assert_eq!(args.max_tenants, 0);
        assert_eq!(args.heartbeat_interval, 15);
        assert!(args.enrollment_token.is_none());
        assert!(args.friendly_name.is_none());
        assert!(!args.insecure);
    }

    #[test]
    fn custom_values_parsed() {
        let args = Args::try_parse_from([
            "uptrakit-mqtt",
            "--controller-url",
            "wss://controller.example.com:9443",
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
            "--insecure",
        ])
        .unwrap();
        assert_eq!(args.controller_url, "wss://controller.example.com:9443");
        assert_eq!(
            args.config_dir.as_ref().unwrap().to_str().unwrap(),
            "/opt/mqtt-config"
        );
        assert_eq!(
            args.state_dir.as_ref().unwrap().to_str().unwrap(),
            "/opt/mqtt-state"
        );
        assert_eq!(args.max_tenants, 5);
        assert_eq!(args.heartbeat_interval, 30);
        assert_eq!(args.enrollment_token.as_deref(), Some("secret-token-123"));
        assert_eq!(
            args.friendly_name.as_deref(),
            Some("Production MQTT Node 1")
        );
        assert!(args.insecure);
    }

    #[test]
    fn controller_url_required() {
        let result = Args::try_parse_from(["uptrakit-mqtt"]);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_dirs_with_defaults() {
        let args =
            Args::try_parse_from(["uptrakit-mqtt", "--controller-url", "wss://localhost:8443"])
                .expect("should parse defaults");
        let dirs = args.resolve_dirs().expect("should resolve dirs");
        // Should return platform-specific paths
        assert!(!dirs.config_dir().as_os_str().is_empty());
        assert!(!dirs.state_dir().as_os_str().is_empty());
    }

    #[test]
    fn resolve_dirs_with_overrides() {
        let args = Args::try_parse_from([
            "uptrakit-mqtt",
            "--controller-url",
            "wss://localhost:8443",
            "--config-dir",
            "/custom/config",
            "--state-dir",
            "/custom/state",
        ])
        .expect("should parse");
        let dirs = args.resolve_dirs().expect("should resolve dirs");
        assert_eq!(dirs.config_dir().to_str().unwrap(), "/custom/config");
        assert_eq!(dirs.state_dir().to_str().unwrap(), "/custom/state");
    }

    #[test]
    fn friendly_name_or_hostname_returns_provided() {
        let args = Args::try_parse_from([
            "uptrakit-mqtt",
            "--controller-url",
            "wss://localhost:8443",
            "--friendly-name",
            "My Node",
        ])
        .unwrap();
        assert_eq!(args.friendly_name_or_hostname(), "My Node");
    }

    #[test]
    fn friendly_name_or_hostname_falls_back_to_hostname() {
        let args =
            Args::try_parse_from(["uptrakit-mqtt", "--controller-url", "wss://localhost:8443"])
                .unwrap();
        // Should return the system hostname, which we can't predict in tests,
        // but it should not be empty
        assert!(!args.friendly_name_or_hostname().is_empty());
    }
}
