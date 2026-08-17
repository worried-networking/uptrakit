use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

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

    /// Embedded-agent maintenance commands, run on this host and exit.
    #[cfg(feature = "embedded-agent")]
    Agent {
        #[command(subcommand)]
        command: uptrakit_agent_runtime::AgentRuntimeCommand,
    },
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

    /// Increase log verbosity (-v for own-crate debug, -vv for uptrakit=debug, -vvv for uptrakit=trace).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Path to the TOML configuration file.
    ///
    /// When not set, probes the platform config directory
    /// (e.g., `~/.config/uptrakit/controller/controller.toml` on Linux) then
    /// `/etc/uptrakit/controller.toml`, using the first that exists.
    #[arg(long, env = "UPTRAKIT_CONFIG")]
    pub config: Option<PathBuf>,

    /// Source for the master encryption key.
    /// Supports file paths (file:/path/to/key), env vars (env:VAR_NAME), or inline hex.
    #[arg(long, env = "UPTRAKIT_MASTER_KEY_FROM")]
    pub master_key_from: Option<String>,

    /// Validate the config file and exit without starting the server.
    #[arg(long)]
    pub check_config: bool,

    /// Run database migrations and exit without starting the server.
    #[arg(long)]
    pub migrate_and_exit: bool,

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

impl Args {
    /// Find the TOML config file path.
    ///
    /// Returns `--config` directly if provided; otherwise probes platform config
    /// directory then `/etc/uptrakit/controller.toml`.
    ///
    /// # Errors
    ///
    /// Returns an error if `AppDirs::resolve` cannot determine the platform
    /// config directory (e.g., no home directory in the environment).
    pub(crate) fn find_config_path(&self) -> uptrakit_directories::Result<PathBuf> {
        let dirs = uptrakit_directories::AppDirs::resolve("controller", None, None)?;
        Ok(probe_config_path(dirs.config_dir(), self.config.as_ref()))
    }
}

/// Probe candidate TOML config paths and return the first that is a regular file.
///
/// If `explicit` is `Some`, returns it unconditionally without probing.
/// Otherwise checks in order:
///   1. `config_dir.join("controller.toml")`
///   2. `/etc/uptrakit/controller.toml`
///
/// Falls back to the platform path if neither exists so callers emit a
/// sensible "not found" message rather than pointing at a confusing location.
pub(crate) fn probe_config_path(config_dir: &Path, explicit: Option<&PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p.clone();
    }
    let platform = config_dir.join("controller.toml");
    if platform.is_file() {
        return platform;
    }
    let legacy = Path::new("/etc/uptrakit/controller.toml");
    if legacy.is_file() {
        return legacy.to_path_buf();
    }
    platform
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test code: `assert!(r.is_err())` is idiomatic in tests where the error variant is not inspected"
    )]

    use std::path::Path;

    use clap::Parser;

    #[test]
    fn bare_ipv4_parsed_as_host() {
        let net = parse_proxy("192.168.1.1").unwrap();
        let expected: ipnet::IpNet = "192.168.1.1/32".parse().unwrap();
        assert_eq!(net, expected);
    }

    #[test]
    fn cidr_notation_parsed() {
        let net = parse_proxy("10.0.0.0/8").unwrap();
        let expected: ipnet::IpNet = "10.0.0.0/8".parse().unwrap();
        assert_eq!(net, expected);
    }

    #[test]
    fn invalid_proxy_string() {
        assert!(parse_proxy("not-an-ip").is_err());
    }

    #[test]
    fn ipv6_bare_address() {
        let net = parse_proxy("::1").unwrap();
        assert_eq!(
            net,
            ipnet::IpNet::from(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST))
        );
    }

    #[test]
    fn ipv6_cidr() {
        let net = parse_proxy("fd00::/8").unwrap();
        let expected: ipnet::IpNet = "fd00::/8".parse().unwrap();
        assert_eq!(net, expected);
    }

    #[test]
    fn empty_proxy_list() {
        // Just verifying parse_proxy isn't called with empty strings accidentally
        assert!(parse_proxy("").is_err());
    }

    #[test]
    fn defaults_have_no_master_key() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(args.master_key_from.is_none());
        assert!(!args.version);
        assert_eq!(args.verbose, 0);
    }

    #[test]
    fn version_flag_parses() {
        let args = super::Args::try_parse_from(["uptrakit-controller", "--version"])
            .expect("should parse");
        assert!(args.version);
    }

    #[test]
    fn verbose_flag_counts_occurrences() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller", "-v", "-v"]).expect("should parse");
        assert_eq!(args.verbose, 2);
    }

    #[test]
    fn master_key_from_env_var() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--master-key-from",
            "file:/etc/uptrakit/master.key",
        ])
        .expect("should parse master-key-from");
        assert_eq!(
            args.master_key_from.as_deref(),
            Some("file:/etc/uptrakit/master.key")
        );
    }

    #[test]
    fn config_field_is_none_by_default() {
        let args = super::Args::try_parse_from(["uptrakit-controller"]).unwrap();
        assert!(args.config.is_none());
    }

    #[test]
    fn config_explicit_path_returned_directly() {
        // Explicit --config bypasses probing; no filesystem access or resolve_dirs occurs.
        let args = super::Args::try_parse_from(["uptrakit-controller", "--config", "/tmp/my.toml"])
            .unwrap();
        assert_eq!(
            super::probe_config_path(Path::new("/irrelevant"), args.config.as_ref()),
            std::path::PathBuf::from("/tmp/my.toml")
        );
    }

    #[test]
    fn check_config_default_false() {
        let args = super::Args::try_parse_from(["uptrakit-controller"]).unwrap();
        assert!(!args.check_config);
    }

    #[test]
    fn migrate_and_exit_default_false() {
        let args = super::Args::try_parse_from(["uptrakit-controller"]).unwrap();
        assert!(!args.migrate_and_exit);
    }

    #[cfg(feature = "embedded-agent")]
    #[test]
    fn agent_bootstrap_host_parses() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "agent",
            "bootstrap-host",
            "--user",
            "svc",
        ])
        .expect("should parse agent subcommand");
        assert!(matches!(
            args.command,
            Some(super::ControllerCommand::Agent { .. })
        ));
    }

    #[test]
    fn oidc_bootstrap_not_set_by_default() {
        let oidc =
            super::OidcBootstrapArgs::try_parse_from(["test"]).expect("should parse defaults");
        assert!(oidc.oidc_issuer_url.is_none());
        assert!(oidc.oidc_client_id.is_none());
        assert!(oidc.oidc_client_secret.is_none());
        assert!(oidc.oidc_allow_private_network_issuers.is_none());
        assert_eq!(oidc.oidc_provider_name.as_deref(), Some("SSO"));
        assert_eq!(oidc.oidc_provider_slug.as_deref(), Some("sso"));
        assert_eq!(
            oidc.oidc_scopes.as_deref(),
            Some("openid email profile groups")
        );
    }

    #[test]
    fn enrollment_bootstrap_defaults() {
        let eb = super::EnrollmentBootstrapArgs::try_parse_from(["test"])
            .expect("should parse defaults");
        assert!(eb.bootstrap_enrollment_token.is_none());
        assert_eq!(eb.bootstrap_enrollment_token_max_uses, 1);
        assert_eq!(eb.bootstrap_enrollment_token_ttl, 300);
        assert!(eb.bootstrap_system_enrollment_token.is_none());
        assert_eq!(eb.bootstrap_system_enrollment_token_max_uses, 1);
        assert_eq!(eb.bootstrap_system_enrollment_token_ttl, 300);
    }

    /// Parse a trusted proxy argument. Accepts:
    /// - Bare IP: `192.168.1.1` -> `192.168.1.1/32`
    /// - Bare IPv6: `::1` -> `::1/128`
    /// - CIDR: `10.0.0.0/8`, `fd00::/8`
    fn parse_proxy(s: &str) -> Result<ipnet::IpNet, String> {
        // Try CIDR first
        if let Ok(net) = s.parse::<ipnet::IpNet>() {
            return Ok(net);
        }
        // Try bare IP -> host network
        if let Ok(ip) = s.parse::<std::net::IpAddr>() {
            return Ok(ipnet::IpNet::from(ip));
        }
        Err(format!("invalid IP or CIDR: {s}"))
    }
}
