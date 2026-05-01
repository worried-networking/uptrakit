use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ipnet::IpNet;
use uptrakit_directories::AppDirs;

/// Available subcommands for the controller binary.
#[derive(Subcommand, Debug)]
pub(crate) enum ControllerCommand {
    /// Migrate all data from one database to another.
    ///
    /// Copies every application table from the source database to the target
    /// database. The target schema is set up automatically via the normal
    /// migrations path. All existing data in the target is erased before the
    /// copy begins.
    ///
    /// Run this while the controller is stopped. Take a backup of both
    /// databases before proceeding.
    DbMigrate(DbMigrateArgs),
}

/// Arguments for the `db-migrate` subcommand.
#[derive(Parser, Debug)]
pub(crate) struct DbMigrateArgs {
    /// Source database URL to read data from.
    ///
    /// Supported schemes depend on enabled build features:
    ///   SQLite (default): sqlite:///path/to/uptrakit.db
    ///   PostgreSQL: postgresql://user:pass@host:5432/dbname
    #[arg(long)]
    pub source_db: String,

    /// Target database URL to write data into.
    ///
    /// The schema will be created automatically. All existing rows in the
    /// target are erased before the copy begins.
    #[arg(long)]
    pub target_db: String,

    /// Number of rows to read and insert per batch.
    #[arg(long, default_value = "500")]
    pub batch_size: u64,

    /// Skip the non-empty target safety check.
    ///
    /// By default, migration is aborted if the target already contains user
    /// data. Use this flag to override that check.
    #[arg(long)]
    pub force: bool,

    /// Skip the interactive confirmation prompt.
    ///
    /// Useful for scripted or CI use-cases where interactive input is
    /// not available.
    #[arg(long)]
    pub yes: bool,
}

/// Uptrakit Controller — central server for the Uptrakit update tracking toolkit.
#[derive(Parser, Debug)]
#[command(name = "uptrakit-controller")]
#[command(disable_version_flag = true)]
pub(crate) struct Args {
    /// Show crate version and build metadata.
    #[arg(long)]
    pub version: bool,

    /// Config directory for persistent configuration (CA certificates, TLS certs).
    /// Supports `~` for home directory expansion.
    /// Default: platform-specific (e.g., ~/.config/controller on Linux).
    #[arg(long, env = "UPTRAKIT_CONFIG_DIR")]
    pub config_dir: Option<PathBuf>,

    /// State directory for runtime state (SQLite DB, JWT signing key).
    /// Supports `~` for home directory expansion.
    /// Default: platform-specific (e.g., ~/.local/state/controller on Linux).
    #[arg(long, env = "UPTRAKIT_STATE_DIR")]
    pub state_dir: Option<PathBuf>,

    /// Database URL. If not provided, defaults to SQLite in data directory.
    /// Supported schemes depend on enabled features:
    ///   SQLite (default): sqlite://path/to/db.sqlite
    ///   PostgreSQL: postgresql://user:pass@host:5432/dbname
    #[arg(long, env = "UPTRAKIT_DB_URL")]
    pub db_url: Option<String>,

    /// Maximum number of connections in the database connection pool.
    /// Increase this value under high REST/WebSocket load to prevent pool
    /// exhaustion. The default of 10 is suitable for small to medium deployments.
    #[arg(long, default_value = "10")]
    pub db_max_connections: u32,

    /// HTTPS listen address (dual-stack by default).
    /// Stored in DB as `network.https_addr`. CLI value used only on first run
    /// or with `--force-settings-override`.
    #[arg(long, env = "UPTRAKIT_HTTPS_ADDR")]
    pub https_addr: Option<SocketAddr>,

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
    /// Stored in DB as `network.trusted_proxies`. CLI value used only on first
    /// run or with `--force-settings-override`.
    #[arg(long = "trusted-proxy", value_parser = parse_proxy)]
    pub trusted_proxies: Vec<IpNet>,

    /// Header to extract the real client IP from when behind a trusted proxy.
    /// Supported: X-Forwarded-For, Forwarded (RFC 7239), X-Real-Ip,
    /// or any custom header name (parsed as comma-separated IPs).
    /// Stored in DB as `network.real_ip_header`. CLI value used only on first
    /// run or with `--force-settings-override`.
    #[arg(long)]
    pub real_ip_header: Option<String>,

    /// Override: path to PEM-encoded CA certificate.
    /// If not provided, a CA is auto-generated and managed internally.
    /// Must be provided together with --ca-key.
    #[arg(long)]
    pub ca_cert: Option<PathBuf>,

    /// Override: path to PEM-encoded CA private key.
    /// Must be provided together with --ca-cert.
    #[arg(long)]
    pub ca_key: Option<PathBuf>,

    /// Complete SAN list for the server certificate (IP or DNS name, repeatable).
    /// Disables auto-detection. Stored in DB as `network.sans`.
    /// CLI value used only on first run or with `--force-settings-override`.
    #[arg(long = "san")]
    pub sans: Vec<String>,

    /// Header name for structured forwarded client certificate info
    /// (e.g. `X-Forwarded-Tls-Client-Cert-Info` for Traefik).
    /// Requires `--trusted-proxy` to take effect.
    /// Stored in DB as `network.forwarded_client_cert_info_header`.
    #[arg(long)]
    pub forwarded_client_cert_info_header: Option<String>,

    /// Header name for PEM-encoded forwarded client certificate
    /// (e.g. `X-Forwarded-Tls-Client-Cert` for Traefik/Caddy).
    /// Used as fallback when the info header is absent.
    /// Requires `--trusted-proxy` to take effect.
    /// Stored in DB as `network.forwarded_client_cert_pem_header`.
    #[arg(long)]
    pub forwarded_client_cert_pem_header: Option<String>,

    /// URL for PKI endpoints (OCSP, CRL, CA cert) embedded in certificate extensions.
    /// Supports both http:// and https:// schemes. http:// is recommended because
    /// Nginx only supports http:// OCSP responder URLs — https:// AIA URLs are
    /// silently ignored by Nginx's ssl_ocsp directive.
    /// Example: http://controller:8080 (recommended) or https://controller.internal:8443
    /// Stored in DB as `network.pki_addr`. CLI value used only on first run
    /// or with `--force-settings-override`.
    #[arg(long, value_parser = parse_pki_addr)]
    pub pki_addr: Option<String>,

    /// How to serve PKI endpoints over plain HTTP.
    /// Use `listener` to start a built-in HTTP server on the port from --pki-addr,
    /// or `external` if PKI HTTP is handled by a reverse proxy (suppresses the
    /// http:// scheme warning when --pki-addr uses http://).
    #[arg(long, value_enum)]
    pub pki_http: Option<PkiHttpMode>,

    /// Path to the built frontend directory. Enables SPA serving.
    ///
    /// When the `embedded-frontend` feature is compiled in, this flag overrides the
    /// embedded assets, which is useful for development or hot-reload scenarios.
    /// Without `embedded-frontend`, the flag is required to enable frontend serving.
    #[arg(long)]
    pub static_dir: Option<PathBuf>,

    /// When set, CLI values for DB-managed settings (network, OIDC) overwrite
    /// any existing values in the database. Without this flag, DB values take
    /// priority and a warning is logged when CLI values differ.
    #[arg(long)]
    pub force_settings_override: bool,

    #[command(flatten)]
    pub oidc_bootstrap: OidcBootstrapArgs,

    #[command(flatten)]
    pub enrollment_bootstrap: EnrollmentBootstrapArgs,

    /// Enable SO_REUSEPORT socket option for zero-downtime restarts.
    /// Required on both the first process and the takeover process.
    /// This allows a new process to bind to the same port while the old
    /// process is still running.
    #[arg(long)]
    pub reuseport: bool,

    /// Path to the PID file written by this process.
    /// Required when --reuseport is used so the self-update plugin can signal
    /// the running process via `kill -USR2 $(cat <pid-file>)` during an update.
    #[arg(long)]
    pub pid_file: Option<std::path::PathBuf>,

    /// Path to a file containing the master encryption key (64-char hex string).
    /// The key is used for AES-256-GCM encryption of sensitive credentials at rest.
    /// Alternative: set UPTRAKIT_MASTER_KEY environment variable.
    #[arg(long)]
    pub master_key_file: Option<PathBuf>,

    /// Allow the controller to start without a master encryption key.
    /// Encryption at rest is disabled when no key is provided.
    /// This flag is for development only and logs a warning when used.
    #[arg(long)]
    pub allow_plaintext_secrets: bool,

    /// Allow notification webhook URLs that point to private / loopback /
    /// link-local addresses. By default, the webhook channel rejects such
    /// URLs to prevent SSRF. Enable this in single-tenant or self-hosted
    /// deployments where internal webhook targets (e.g. a Mattermost on the
    /// LAN) are legitimate. The header blocklist is always enforced regardless
    /// of this flag.
    #[arg(long)]
    pub allow_private_notification_urls: bool,

    /// Allow plugin config changes that contain dangerous command patterns.
    /// By default, creating or updating plugin configs with patterns such as
    /// `curl|bash`, `rm -rf /`, fork bombs, or bash network sockets will
    /// return HTTP 400. Pass this flag to downgrade to advisory-only warnings.
    /// This does not replace the `manage_commands` permission requirement —
    /// it removes a content-level safety net.
    #[arg(long, env = "UPTRAKIT_ALLOW_DANGEROUS_COMMANDS")]
    pub allow_dangerous_commands: bool,

    /// Path to a new master key file for key rotation.
    ///
    /// Re-wraps all data encryption keys from the old master key to the new one.
    /// O(1) cost regardless of data volume — only the DEK wrappers are updated,
    /// not the encrypted data itself.
    ///
    /// After rotation, restart all controllers with --master-key-file pointing
    /// to the new key file only.
    #[arg(long)]
    pub rotate_master_key_file: Option<std::path::PathBuf>,

    /// PID of old controller process to take over from.
    /// Sends SIGUSR1 to the old process to initiate graceful shutdown.
    /// Should be used together with --reuseport.
    #[arg(long)]
    pub takeover_from: Option<u32>,

    /// Graceful shutdown timeout in seconds.
    /// The time to wait for existing connections to drain before forcing shutdown.
    #[arg(long, default_value = "30")]
    pub shutdown_timeout_secs: u64,

    /// Audit log backend(s). Can be specified multiple times for concurrent
    /// fan-out (e.g. `--audit-log-backend db --audit-log-backend journald`).
    /// Use `none` to disable all audit logging (mutually exclusive with
    /// other backends). Default: `db`.
    #[arg(long = "audit-log-backend", value_enum, default_value = "db")]
    pub audit_log_backend: Vec<AuditLogBackendArg>,

    /// Separate database URL for audit log storage. When not provided,
    /// audit logs are stored in the main application database.
    /// Supported schemes depend on enabled database features.
    #[arg(long)]
    pub audit_log_db_url: Option<String>,

    /// Audit log filter mode. Controls which authenticated HTTP requests
    /// are recorded.
    #[arg(long, value_enum, default_value = "all")]
    pub audit_log_filter: AuditLogFilterArg,

    /// Advertise this controller via mDNS/DNS-SD for zero-configuration
    /// discovery. Services on the same LAN can discover the controller
    /// without --url.
    #[cfg(feature = "zeroconf")]
    #[arg(long)]
    pub zeroconf: bool,

    /// Override the HTTPS URL advertised via mDNS for reverse proxy
    /// deployments. When set, services use this URL instead of
    /// constructing one from the mDNS-resolved address.
    /// Example: https://proxy.example.com:443
    #[cfg(feature = "zeroconf")]
    #[arg(long, requires = "zeroconf")]
    pub zeroconf_url: Option<String>,

    /// Override the PKI address advertised via mDNS for reverse proxy
    /// deployments. Defaults to --pki-addr if set.
    #[cfg(feature = "zeroconf")]
    #[arg(long, requires = "zeroconf")]
    pub zeroconf_pki_addr: Option<String>,

    /// NATS server URL for cross-controller messaging.
    /// When set, NATS JetStream is used for inter-controller event delivery.
    /// Without this, the controller runs in single-instance mode.
    /// Example: nats://localhost:4222
    #[cfg(feature = "nats")]
    #[arg(long, env = "UPTRAKIT_NATS_URL")]
    pub nats_url: Option<String>,

    /// Increase log verbosity (-v for own-crate debug, -vv for uptrakit=debug, -vvv for uptrakit=trace).
    /// Use RUST_LOG to enable logging for other crates (e.g. `RUST_LOG=tokio=info`).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Optional subcommand.
    #[command(subcommand)]
    pub command: Option<ControllerCommand>,
}

/// OIDC provider bootstrap options.
///
/// When `--oidc-issuer-url` is provided, the controller ensures an OIDC provider
/// exists in the database at startup. This solves the chicken-and-egg problem
/// where OIDC configuration requires ManageSettings permission, but the first
/// user must log in via OIDC.
#[derive(Parser, Debug)]
pub(crate) struct OidcBootstrapArgs {
    /// OIDC issuer URL. When set, bootstraps an OIDC provider at startup.
    /// Requires --oidc-client-id and --oidc-client-secret.
    #[arg(long, env = "UPTRAKIT_OIDC_ISSUER_URL")]
    pub oidc_issuer_url: Option<String>,

    /// OIDC client ID. Required when --oidc-issuer-url is set.
    #[arg(long, env = "UPTRAKIT_OIDC_CLIENT_ID")]
    pub oidc_client_id: Option<String>,

    /// OIDC client secret. Required when --oidc-issuer-url is set.
    #[arg(long, env = "UPTRAKIT_OIDC_CLIENT_SECRET")]
    pub oidc_client_secret: Option<String>,

    /// Display name for the bootstrapped OIDC provider.
    #[arg(long, env = "UPTRAKIT_OIDC_PROVIDER_NAME", default_value = "SSO")]
    pub oidc_provider_name: Option<String>,

    /// URL-safe slug for the bootstrapped OIDC provider.
    #[arg(long, env = "UPTRAKIT_OIDC_PROVIDER_SLUG", default_value = "sso")]
    pub oidc_provider_slug: Option<String>,

    /// Space-separated OIDC scopes.
    #[arg(
        long,
        env = "UPTRAKIT_OIDC_SCOPES",
        default_value = "openid email profile groups"
    )]
    pub oidc_scopes: Option<String>,

    /// Whether the bootstrapped OIDC provider may resolve to private-network
    /// addresses. When omitted, the default depends on operation mode:
    /// allowed in single-tenant mode and forbidden in multi-tenant mode.
    /// Multi-tenant mode rejects an explicit `true`.
    #[arg(long, env = "UPTRAKIT_OIDC_ALLOW_PRIVATE_NETWORK_ISSUERS")]
    pub oidc_allow_private_network_issuers: Option<bool>,
}

/// Enrollment token bootstrap options.
///
/// When `--bootstrap-enrollment-token` is provided, the controller ensures
/// a tenant enrollment token named "bootstrap" exists at startup. This
/// enables zero-interaction docker-compose deployments where services
/// auto-enroll using a shared secret.
///
/// Same pattern applies for `--bootstrap-system-enrollment-token` for
/// system services (MQTT bridge, external scheduler).
#[derive(Parser, Debug)]
pub(crate) struct EnrollmentBootstrapArgs {
    /// Pre-shared token for tenant service auto-enrollment.
    /// Creates a token named "bootstrap" at startup if none with that name
    /// exists (active, not revoked, not expired).
    /// Services use the same value via --enrollment-token / UPTRAKIT_ENROLLMENT_TOKEN.
    #[arg(long, env = "UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN")]
    pub bootstrap_enrollment_token: Option<String>,

    /// Maximum number of uses for the bootstrap enrollment token.
    #[arg(
        long,
        env = "UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN_MAX_USES",
        default_value = "1"
    )]
    pub bootstrap_enrollment_token_max_uses: u32,

    /// TTL in seconds for the bootstrap enrollment token.
    #[arg(
        long,
        env = "UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN_TTL",
        default_value = "300"
    )]
    pub bootstrap_enrollment_token_ttl: u64,

    /// Pre-shared token for system service auto-enrollment (MQTT, scheduler).
    /// Creates a token named "bootstrap" at startup if none with that name
    /// exists (active, not revoked, not expired).
    #[arg(long, env = "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN")]
    pub bootstrap_system_enrollment_token: Option<String>,

    /// Maximum number of uses for the bootstrap system enrollment token.
    #[arg(
        long,
        env = "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN_MAX_USES",
        default_value = "1"
    )]
    pub bootstrap_system_enrollment_token_max_uses: u32,

    /// TTL in seconds for the bootstrap system enrollment token.
    #[arg(
        long,
        env = "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN_TTL",
        default_value = "300"
    )]
    pub bootstrap_system_enrollment_token_ttl: u64,
}

/// Audit log backend storage.
///
/// Multiple backends can be selected simultaneously for concurrent fan-out
/// (e.g. `--audit-log-backend db --audit-log-backend journald`).
/// `None` disables all audit logging and is mutually exclusive with other values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum AuditLogBackendArg {
    /// Store audit logs in the database.
    Db,
    /// Emit audit logs to journald via structured tracing events.
    #[cfg(feature = "journald")]
    Journald,
    /// Disable audit logging.
    None,
}

/// Audit log filter mode.
///
/// Controls which authenticated HTTP requests are recorded.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum AuditLogFilterArg {
    /// Log all authenticated HTTP requests (default).
    All,
    /// Log only mutation requests (POST, PUT, PATCH, DELETE).
    Mutations,
    /// Disable audit logging (overrides backend selection).
    None,
}

/// How to serve PKI endpoints over plain HTTP.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum PkiHttpMode {
    /// Start a built-in plain HTTP listener for PKI endpoints.
    Listener,
    /// PKI HTTP is handled externally (e.g., by a reverse proxy).
    /// Suppresses the warning about http:// pki-addr without a listener.
    External,
}

/// Validate a PKI address URL argument.
/// Must be a valid URL with http or https scheme, and no trailing slash.
fn parse_pki_addr(s: &str) -> Result<String, String> {
    let url: url::Url = s.parse().map_err(|e| format!("invalid URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "unsupported URL scheme: {other} (expected http or https)"
            ));
        }
    }
    let result = s.trim_end_matches('/').to_string();
    Ok(result)
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
    /// Resolve application directories using platform-specific defaults.
    ///
    /// Returns `AppDirs` with separate config and state directories.
    /// CLI overrides take precedence over platform defaults.
    pub(crate) fn resolve_dirs(&self) -> uptrakit_directories::Result<AppDirs> {
        AppDirs::resolve(
            "controller",
            self.config_dir.as_deref(),
            self.state_dir.as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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

    #[test]
    fn allow_dangerous_commands_default_false() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(!args.allow_dangerous_commands);
    }

    #[test]
    fn allow_dangerous_commands_flag() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller", "--allow-dangerous-commands"])
                .expect("should parse allow flag");
        assert!(args.allow_dangerous_commands);
    }

    #[test]
    fn defaults_have_no_addresses() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(!args.version);
        assert!(args.config_dir.is_none());
        assert!(args.state_dir.is_none());
        assert!(args.https_addr.is_none());
        assert!(args.real_ip_header.is_none());
        assert!(args.trusted_proxies.is_empty());
        assert!(args.sans.is_empty());
        assert!(!args.force_settings_override);
        assert_eq!(args.verbose, 0);
    }

    #[test]
    fn verbose_flag_parses() {
        let args = super::Args::try_parse_from(["uptrakit-controller", "-v"])
            .expect("should parse -v flag");
        assert_eq!(args.verbose, 1);
    }

    #[test]
    fn resolve_dirs_with_defaults() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        let dirs = args.resolve_dirs().expect("should resolve dirs");
        // Should return platform-specific paths
        assert!(!dirs.config_dir().as_os_str().is_empty());
        assert!(!dirs.state_dir().as_os_str().is_empty());
    }

    #[test]
    fn resolve_dirs_with_overrides() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
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
    fn explicit_addresses_parsed() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--https-addr",
            "0.0.0.0:9443",
            "--real-ip-header",
            "X-Real-Ip",
        ])
        .expect("should parse explicit addresses");
        assert_eq!(
            args.https_addr.unwrap(),
            "0.0.0.0:9443".parse::<std::net::SocketAddr>().unwrap()
        );
        assert_eq!(args.real_ip_header.as_deref(), Some("X-Real-Ip"));
    }

    #[test]
    fn version_flag_parses() {
        let args = super::Args::try_parse_from(["uptrakit-controller", "--version"])
            .expect("should parse");
        assert!(args.version);
    }

    #[test]
    fn force_settings_override_flag() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller", "--force-settings-override"])
                .expect("should parse force flag");
        assert!(args.force_settings_override);
    }

    #[test]
    fn forwarded_cert_headers_not_set_by_default() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(args.forwarded_client_cert_info_header.is_none());
        assert!(args.forwarded_client_cert_pem_header.is_none());
    }

    #[test]
    fn forwarded_cert_headers_custom_values() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--forwarded-client-cert-info-header",
            "X-Forwarded-Tls-Client-Cert-Info",
            "--forwarded-client-cert-pem-header",
            "X-Forwarded-Tls-Client-Cert",
        ])
        .expect("should parse custom values");
        assert_eq!(
            args.forwarded_client_cert_info_header.as_deref(),
            Some("X-Forwarded-Tls-Client-Cert-Info")
        );
        assert_eq!(
            args.forwarded_client_cert_pem_header.as_deref(),
            Some("X-Forwarded-Tls-Client-Cert")
        );
    }

    #[test]
    fn bare_ipv4_contains_only_self() {
        let net = parse_proxy("192.168.1.1").unwrap();
        assert!(net.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(!net.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2))));
    }

    #[test]
    fn oidc_bootstrap_not_set_by_default() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(args.oidc_bootstrap.oidc_issuer_url.is_none());
        assert!(args.oidc_bootstrap.oidc_client_id.is_none());
        assert!(args.oidc_bootstrap.oidc_client_secret.is_none());
        assert!(
            args.oidc_bootstrap
                .oidc_allow_private_network_issuers
                .is_none()
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_provider_name.as_deref(),
            Some("SSO")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_provider_slug.as_deref(),
            Some("sso")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_scopes.as_deref(),
            Some("openid email profile groups")
        );
    }

    #[test]
    fn oidc_bootstrap_custom_values() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--oidc-issuer-url",
            "https://auth.example.com",
            "--oidc-client-id",
            "my-client",
            "--oidc-client-secret",
            "my-secret",
            "--oidc-provider-name",
            "My IdP",
            "--oidc-provider-slug",
            "my-idp",
            "--oidc-scopes",
            "openid email",
            "--oidc-allow-private-network-issuers",
            "false",
        ])
        .expect("should parse custom values");

        assert_eq!(
            args.oidc_bootstrap.oidc_issuer_url.as_deref(),
            Some("https://auth.example.com")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_client_id.as_deref(),
            Some("my-client")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_client_secret.as_deref(),
            Some("my-secret")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_provider_name.as_deref(),
            Some("My IdP")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_provider_slug.as_deref(),
            Some("my-idp")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_scopes.as_deref(),
            Some("openid email")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_allow_private_network_issuers,
            Some(false)
        );
    }

    #[test]
    fn oidc_bootstrap_partial_requires_all_three() {
        // Only issuer URL without client ID and secret should still parse
        // (validation happens at runtime, not at CLI parse time)
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--oidc-issuer-url",
            "https://auth.example.com",
        ])
        .expect("should parse with only issuer URL");
        assert!(args.oidc_bootstrap.oidc_issuer_url.is_some());
        assert!(args.oidc_bootstrap.oidc_client_id.is_none());
        assert!(args.oidc_bootstrap.oidc_client_secret.is_none());
    }

    #[test]
    fn audit_log_defaults() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert_eq!(args.audit_log_backend, vec![super::AuditLogBackendArg::Db]);
        assert!(matches!(
            args.audit_log_filter,
            super::AuditLogFilterArg::All
        ));
        assert!(args.audit_log_db_url.is_none());
    }

    #[test]
    fn audit_log_backend_none() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller", "--audit-log-backend", "none"])
                .expect("should parse none backend");
        assert_eq!(
            args.audit_log_backend,
            vec![super::AuditLogBackendArg::None]
        );
    }

    #[test]
    fn audit_log_filter_mutations() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller", "--audit-log-filter", "mutations"])
                .expect("should parse mutations filter");
        assert!(matches!(
            args.audit_log_filter,
            super::AuditLogFilterArg::Mutations
        ));
    }

    #[test]
    fn audit_log_separate_db_url() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--audit-log-db-url",
            "postgresql://user:pass@host:5432/audit",
        ])
        .expect("should parse separate DB URL");
        assert_eq!(
            args.audit_log_db_url.as_deref(),
            Some("postgresql://user:pass@host:5432/audit")
        );
    }

    #[test]
    fn audit_log_multiple_backends() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--audit-log-backend",
            "db",
            "--audit-log-backend",
            "db",
        ])
        .expect("should parse multiple backends");
        assert_eq!(args.audit_log_backend.len(), 2);
    }

    #[test]
    fn graceful_restart_defaults() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(!args.reuseport);
        assert!(args.takeover_from.is_none());
        assert_eq!(args.shutdown_timeout_secs, 30);
    }

    #[test]
    fn graceful_restart_reuseport_flag() {
        let args = super::Args::try_parse_from(["uptrakit-controller", "--reuseport"])
            .expect("should parse reuseport flag");
        assert!(args.reuseport);
    }

    #[test]
    fn graceful_restart_takeover_from() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--reuseport",
            "--takeover-from",
            "12345",
        ])
        .expect("should parse takeover-from");
        assert!(args.reuseport);
        assert_eq!(args.takeover_from, Some(12345));
    }

    #[test]
    fn graceful_restart_custom_timeout() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--reuseport",
            "--shutdown-timeout-secs",
            "60",
        ])
        .expect("should parse custom timeout");
        assert!(args.reuseport);
        assert_eq!(args.shutdown_timeout_secs, 60);
    }

    #[test]
    fn enrollment_bootstrap_not_set_by_default() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(
            args.enrollment_bootstrap
                .bootstrap_enrollment_token
                .is_none()
        );
        assert_eq!(
            args.enrollment_bootstrap
                .bootstrap_enrollment_token_max_uses,
            1
        );
        assert_eq!(
            args.enrollment_bootstrap.bootstrap_enrollment_token_ttl,
            300
        );
        assert!(
            args.enrollment_bootstrap
                .bootstrap_system_enrollment_token
                .is_none()
        );
        assert_eq!(
            args.enrollment_bootstrap
                .bootstrap_system_enrollment_token_max_uses,
            1
        );
        assert_eq!(
            args.enrollment_bootstrap
                .bootstrap_system_enrollment_token_ttl,
            300
        );
    }

    #[test]
    fn enrollment_bootstrap_custom_values() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--bootstrap-enrollment-token",
            "my-secret-token",
            "--bootstrap-enrollment-token-max-uses",
            "5",
            "--bootstrap-enrollment-token-ttl",
            "600",
            "--bootstrap-system-enrollment-token",
            "system-secret",
            "--bootstrap-system-enrollment-token-max-uses",
            "3",
            "--bootstrap-system-enrollment-token-ttl",
            "120",
        ])
        .expect("should parse custom values");

        assert_eq!(
            args.enrollment_bootstrap
                .bootstrap_enrollment_token
                .as_deref(),
            Some("my-secret-token")
        );
        assert_eq!(
            args.enrollment_bootstrap
                .bootstrap_enrollment_token_max_uses,
            5
        );
        assert_eq!(
            args.enrollment_bootstrap.bootstrap_enrollment_token_ttl,
            600
        );
        assert_eq!(
            args.enrollment_bootstrap
                .bootstrap_system_enrollment_token
                .as_deref(),
            Some("system-secret")
        );
        assert_eq!(
            args.enrollment_bootstrap
                .bootstrap_system_enrollment_token_max_uses,
            3
        );
        assert_eq!(
            args.enrollment_bootstrap
                .bootstrap_system_enrollment_token_ttl,
            120
        );
    }

    #[test]
    fn enrollment_bootstrap_tenant_only() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--bootstrap-enrollment-token",
            "tenant-secret",
        ])
        .expect("should parse with tenant token only");
        assert!(
            args.enrollment_bootstrap
                .bootstrap_enrollment_token
                .is_some()
        );
        assert!(
            args.enrollment_bootstrap
                .bootstrap_system_enrollment_token
                .is_none()
        );
    }
}
