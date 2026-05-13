//! Common CLI arguments shared by agent and MQTT service binaries.
//!
//! Provides [`CommonServiceArgs`] which can be flattened into each binary's
//! `Args` struct via `#[command(flatten)]`.

use std::path::PathBuf;

use crate::dirs::AppDirs;

/// Common CLI arguments for services that enroll with the controller.
///
/// Both the agent and the MQTT service share these flags for controller
/// connection, CA bootstrap, enrollment, and directory management.
#[derive(clap::Args, Debug)]
#[command(group(
    clap::ArgGroup::new("tofu_mode")
        .multiple(false)
        .args(["tofu_fingerprint", "tofu_spki", "tofu_insecure"])
))]
pub struct CommonServiceArgs {
    /// Show crate version and build metadata.
    #[arg(long)]
    pub version: bool,

    /// Controller URL (e.g., `https://controller:8443`).
    /// Port defaults to 443 if omitted.
    #[arg(long)]
    pub url: Option<String>,

    /// Pin the Controller's CA bundle by SHA-256 fingerprint. On first
    /// successful connection, the bundle is persisted to disk.
    /// Conflicts with `--ca-cert` and `--pki-addr`.
    #[arg(long, value_name = "SHA256", conflicts_with_all = ["ca_cert", "pki_addr"])]
    pub tofu_fingerprint: Option<crate::tofu::Sha256Hash>,

    /// Pin the Controller's CA by SubjectPublicKeyInfo SHA-256 hash.
    /// Survives cert renewals that reuse the same keypair.
    /// Conflicts with `--ca-cert` and `--pki-addr`.
    #[arg(long, value_name = "SHA256", conflicts_with_all = ["ca_cert", "pki_addr"])]
    pub tofu_spki: Option<crate::tofu::Sha256Hash>,

    /// Accept any chain. Operates as stateless TOFU. Implies
    /// `--tofu-skip-hostname`. Logs WARN on every connection.
    /// Conflicts with `--ca-cert` and `--pki-addr`.
    #[arg(long, conflicts_with_all = ["ca_cert", "pki_addr"])]
    pub tofu_insecure: bool,

    /// Disable ServerName check. Requires one of `--tofu-fingerprint`,
    /// `--tofu-spki`, or `--tofu-insecure`.
    #[arg(long, requires = "tofu_mode")]
    pub tofu_skip_hostname: bool,

    /// Acknowledge a fingerprint observed in a previous `--tofu-insecure`
    /// run. Required to persist the CA bundle in insecure mode.
    #[arg(long, value_name = "SHA256", requires = "tofu_insecure")]
    pub tofu_fingerprint_acknowledge: Option<crate::tofu::Sha256Hash>,

    /// Add compiled-in `webpki-roots` to the trust store.
    #[arg(long)]
    pub trust_public_roots: bool,

    /// Add the OS root store via `rustls-native-certs` to the trust store.
    #[arg(long)]
    pub trust_native_roots: bool,

    /// Path to a PEM-encoded CA certificate file.
    #[arg(long)]
    pub ca_cert: Option<PathBuf>,

    /// Optional URL for PKI endpoints (CA certificate, OCSP).
    /// Supports both `http://` and `https://` schemes.
    #[arg(long, value_parser = parse_pki_addr)]
    pub pki_addr: Option<String>,

    /// Config directory for persistent configuration (controller's CA cert).
    /// Supports `~` for home directory expansion.
    #[arg(long, env = "UPTRAKIT_CONFIG_DIR")]
    pub config_dir: Option<PathBuf>,

    /// State directory for runtime state (enrollment, keys, certificate).
    /// Supports `~` for home directory expansion.
    #[arg(long, env = "UPTRAKIT_STATE_DIR")]
    pub state_dir: Option<PathBuf>,

    /// Friendly name for this service instance (defaults to system hostname).
    #[arg(long)]
    pub friendly_name: Option<String>,

    /// Pre-shared enrollment token for auto-approval.
    #[arg(long, env = "UPTRAKIT_ENROLLMENT_TOKEN")]
    pub enrollment_token: Option<String>,

    /// Force fresh enrollment, discarding existing state.
    /// Preserves the cached CA certificate.
    #[arg(long)]
    pub force_enroll: bool,

    /// Clear the cached discovery result and re-discover the controller.
    #[cfg(feature = "zeroconf")]
    #[arg(long)]
    pub clear_discovery_cache: bool,

    /// Increase log verbosity (-v for own-crate debug, -vv for uptrakit=debug, -vvv for uptrakit=trace).
    /// Use RUST_LOG to enable logging for other crates (e.g. `RUST_LOG=tokio=info`).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbose: u8,
}

impl CommonServiceArgs {
    /// Resolve application directories using platform-specific defaults.
    ///
    /// Returns `AppDirs` with separate config and state directories.
    /// CLI overrides take precedence over platform defaults.
    pub fn resolve_dirs(&self, app_name: &str) -> crate::dirs::Result<AppDirs> {
        AppDirs::resolve(
            app_name,
            self.config_dir.as_deref(),
            self.state_dir.as_deref(),
        )
    }

    /// Get the friendly name, falling back to system hostname.
    pub fn friendly_name_or_hostname(&self) -> String {
        self.friendly_name.clone().unwrap_or_else(|| {
            hostname::get()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        })
    }

    /// Get the system hostname.
    pub fn hostname(&self) -> String {
        hostname::get()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    }

    /// Parse `--url` into `(host, port)`.
    pub fn parsed_url(&self) -> std::result::Result<(String, u16), String> {
        let raw = self
            .url
            .as_deref()
            .ok_or("URL is required unless --version is used")?;
        let url_str = raw.trim_end_matches('/');
        let parsed = url::Url::parse(url_str).map_err(|e| format!("invalid URL: {e}"))?;
        if parsed.scheme() != "https" {
            return Err("URL scheme must be https".to_string());
        }
        let host = parsed
            .host_str()
            .ok_or("URL must contain a host")?
            .to_string();
        let port = parsed.port().unwrap_or(443);
        Ok((host, port))
    }

    /// The base URL (trimmed of trailing slashes).
    pub fn base_url(&self) -> &str {
        self.url
            .as_deref()
            .unwrap_or_default()
            .trim_end_matches('/')
    }

    /// The PKI address, if set.
    pub fn pki_addr(&self) -> Option<&str> {
        self.pki_addr.as_deref()
    }

    /// Parse the TOFU flags into a validated `TofuConfig`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::tofu::TofuConfigError`] if the flag combination is invalid.
    pub fn tofu_config(&self) -> Result<crate::tofu::TofuConfig, crate::tofu::TofuConfigError> {
        crate::tofu::TofuConfig::from_flags(
            self.tofu_fingerprint,
            self.tofu_spki,
            self.tofu_insecure,
            self.tofu_skip_hostname,
            self.tofu_fingerprint_acknowledge,
        )
    }
}

/// Validate `--pki-addr`: must be `http://` or `https://`, must have a host,
/// trailing slashes are stripped.
fn parse_pki_addr(s: &str) -> std::result::Result<String, String> {
    let trimmed = s.trim_end_matches('/');
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("invalid PKI address URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "unsupported URL scheme: {other} (expected http or https)"
            ));
        }
    }
    if parsed.host_str().is_none() {
        return Err("PKI address URL must contain a host".to_string());
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    /// Test harness: a minimal CLI wrapper that flattens `CommonServiceArgs`.
    #[derive(Parser, Debug)]
    #[command(name = "test-service")]
    struct TestArgs {
        #[command(flatten)]
        pub common: super::CommonServiceArgs,
    }

    #[test]
    fn defaults_parse() {
        let args = TestArgs::try_parse_from(["test-service", "--url", "https://controller:8443"])
            .expect("should parse defaults");
        assert!(!args.common.tofu_insecure);
        assert!(args.common.tofu_fingerprint.is_none());
        assert!(args.common.tofu_spki.is_none());
        assert!(!args.common.tofu_skip_hostname);
        assert!(!args.common.trust_public_roots);
        assert!(!args.common.trust_native_roots);
        assert!(!args.common.version);
        assert!(args.common.ca_cert.is_none());
        assert!(args.common.config_dir.is_none());
        assert!(args.common.state_dir.is_none());
        assert!(args.common.friendly_name.is_none());
        assert!(args.common.enrollment_token.is_none());
        assert!(!args.common.force_enroll);
        assert!(args.common.pki_addr.is_none());
        assert_eq!(args.common.url.as_deref(), Some("https://controller:8443"));
        assert_eq!(args.common.verbose, 0);
    }

    #[test]
    fn tofu_fingerprint_and_ca_cert_conflict() {
        let hex = "aa".repeat(32);
        let result = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://host:8443",
            "--tofu-fingerprint",
            &hex,
            "--ca-cert",
            "/some/path.pem",
        ]);
        assert!(
            result.is_err(),
            "--tofu-fingerprint and --ca-cert should conflict"
        );
    }

    #[test]
    fn tofu_fingerprint_and_pki_addr_conflict() {
        let hex = "aa".repeat(32);
        let result = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://host:8443",
            "--tofu-fingerprint",
            &hex,
            "--pki-addr",
            "http://pki.local:8080",
        ]);
        assert!(
            result.is_err(),
            "--tofu-fingerprint and --pki-addr should conflict"
        );
    }

    #[test]
    fn tofu_insecure_implies_skip_hostname_via_config() {
        let args = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://host:8443",
            "--tofu-insecure",
        ])
        .expect("should parse");
        let cfg = args.common.tofu_config().expect("valid config");
        assert!(cfg.skip_hostname, "insecure mode forces skip_hostname");
    }

    #[test]
    fn two_tofu_modes_conflict() {
        let hex = "aa".repeat(32);
        let result = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://host:8443",
            "--tofu-fingerprint",
            &hex,
            "--tofu-insecure",
        ]);
        assert!(
            result.is_err(),
            "two TOFU modes should conflict via ArgGroup"
        );
    }

    #[test]
    fn parsed_url_with_port() {
        let args = TestArgs::try_parse_from(["test-service", "--url", "https://myhost:9443"])
            .expect("should parse");
        let (host, port) = args.common.parsed_url().expect("should parse URL");
        assert_eq!(host, "myhost");
        assert_eq!(port, 9443);
    }

    #[test]
    fn parsed_url_default_port() {
        let args = TestArgs::try_parse_from(["test-service", "--url", "https://myhost"])
            .expect("should parse");
        let (host, port) = args.common.parsed_url().expect("should parse URL");
        assert_eq!(host, "myhost");
        assert_eq!(port, 443);
    }

    #[test]
    fn parsed_url_trailing_slash() {
        let args = TestArgs::try_parse_from(["test-service", "--url", "https://myhost:8443/"])
            .expect("should parse");
        let (host, port) = args.common.parsed_url().expect("should parse URL");
        assert_eq!(host, "myhost");
        assert_eq!(port, 8443);
    }

    #[test]
    fn parsed_url_rejects_http() {
        let args = TestArgs::try_parse_from(["test-service", "--url", "http://myhost:8443"])
            .expect("should parse");
        let err = args.common.parsed_url().unwrap_err();
        assert!(err.contains("https"), "should reject non-https: {err}");
    }

    #[test]
    fn base_url_trims_trailing_slash() {
        let args = TestArgs::try_parse_from(["test-service", "--url", "https://host:8443/"])
            .expect("should parse");
        assert_eq!(args.common.base_url(), "https://host:8443");
    }

    #[test]
    fn pki_addr_accepts_http() {
        let args = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://controller:8443",
            "--pki-addr",
            "http://controller:8080",
        ])
        .expect("should parse --pki-addr with http://");
        assert_eq!(
            args.common.pki_addr.as_deref(),
            Some("http://controller:8080")
        );
    }

    #[test]
    fn pki_addr_accepts_https() {
        let args = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://controller:8443",
            "--pki-addr",
            "https://pki.example.com",
        ])
        .expect("should parse --pki-addr with https://");
        assert_eq!(
            args.common.pki_addr.as_deref(),
            Some("https://pki.example.com")
        );
    }

    #[test]
    fn pki_addr_strips_trailing_slash() {
        let args = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://controller:8443",
            "--pki-addr",
            "http://controller:8080/",
        ])
        .expect("should strip trailing slash");
        assert_eq!(
            args.common.pki_addr.as_deref(),
            Some("http://controller:8080")
        );
    }

    #[test]
    fn pki_addr_rejects_ftp() {
        let result = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://controller:8443",
            "--pki-addr",
            "ftp://controller:21",
        ]);
        assert!(result.is_err(), "should reject ftp:// scheme");
    }

    #[test]
    fn friendly_name_or_hostname_returns_provided() {
        let args = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://controller:8443",
            "--friendly-name",
            "My Service",
        ])
        .expect("should parse");
        assert_eq!(args.common.friendly_name_or_hostname(), "My Service");
    }

    #[test]
    fn friendly_name_or_hostname_falls_back() {
        let args = TestArgs::try_parse_from(["test-service", "--url", "https://controller:8443"])
            .expect("should parse");
        // Should return the system hostname — non-empty
        assert!(!args.common.friendly_name_or_hostname().is_empty());
    }

    #[test]
    fn resolve_dirs_with_defaults() {
        let args = TestArgs::try_parse_from(["test-service", "--url", "https://controller:8443"])
            .expect("should parse");
        let dirs = args
            .common
            .resolve_dirs("agent")
            .expect("should resolve dirs");
        assert!(!dirs.config_dir().as_os_str().is_empty());
        assert!(!dirs.state_dir().as_os_str().is_empty());
    }

    #[test]
    fn version_flag_parses_without_url() {
        let args = TestArgs::try_parse_from(["test-service", "--version"]).expect("should parse");
        assert!(args.common.version);
    }

    #[test]
    fn verbose_flag_counts_occurrences() {
        let args = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://controller:8443",
            "-v",
            "-v",
        ])
        .expect("should parse");
        assert_eq!(args.common.verbose, 2);
    }

    #[test]
    fn resolve_dirs_with_overrides() {
        let args = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://controller:8443",
            "--config-dir",
            "/custom/config",
            "--state-dir",
            "/custom/state",
        ])
        .expect("should parse");
        let dirs = args
            .common
            .resolve_dirs("agent")
            .expect("should resolve dirs");
        assert_eq!(dirs.config_dir().to_str().unwrap(), "/custom/config");
        assert_eq!(dirs.state_dir().to_str().unwrap(), "/custom/state");
    }
}
