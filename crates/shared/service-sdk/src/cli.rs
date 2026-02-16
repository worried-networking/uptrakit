//! Common CLI arguments shared by agent and MQTT service binaries.
//!
//! Provides [`CommonServiceArgs`] which can be flattened into each binary's
//! `Args` struct via `#[command(flatten)]`.

use std::path::PathBuf;

use uptrakit_directories::AppDirs;

/// Common CLI arguments for services that enroll with the controller.
///
/// Both the agent and the MQTT service share these flags for controller
/// connection, CA bootstrap, enrollment, and directory management.
#[derive(clap::Args, Debug)]
pub struct CommonServiceArgs {
    /// Show crate version and build metadata.
    #[arg(long)]
    pub version: bool,

    /// Controller URL (e.g., `https://controller:8443`).
    /// Port defaults to 443 if omitted.
    #[arg(long)]
    pub url: Option<String>,

    /// Trust the controller's TLS certificate on first connection (TOFU).
    /// Only effective when no CA certificate is cached locally.
    #[arg(long, conflicts_with_all = ["ca_cert", "pki_addr"])]
    pub tofu: bool,

    /// Expected SHA-256 fingerprint of the controller's CA certificate (hex).
    /// Used during TOFU to verify the fetched CA matches. Requires `--tofu`.
    #[arg(long, requires = "tofu")]
    pub tofu_fingerprint: Option<String>,

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
}

impl CommonServiceArgs {
    /// Resolve application directories using platform-specific defaults.
    ///
    /// Returns `AppDirs` with separate config and state directories.
    /// CLI overrides take precedence over platform defaults.
    pub fn resolve_dirs(&self, app_name: &str) -> uptrakit_directories::Result<AppDirs> {
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
        assert!(!args.common.tofu);
        assert!(!args.common.version);
        assert!(args.common.ca_cert.is_none());
        assert!(args.common.config_dir.is_none());
        assert!(args.common.state_dir.is_none());
        assert!(args.common.friendly_name.is_none());
        assert!(args.common.enrollment_token.is_none());
        assert!(!args.common.force_enroll);
        assert!(args.common.pki_addr.is_none());
        assert_eq!(args.common.url.as_deref(), Some("https://controller:8443"));
    }

    #[test]
    fn tofu_and_ca_cert_conflict() {
        let result = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://host:8443",
            "--tofu",
            "--ca-cert",
            "/some/path.pem",
        ]);
        assert!(result.is_err(), "--tofu and --ca-cert should conflict");
    }

    #[test]
    fn tofu_and_pki_addr_conflict() {
        let result = TestArgs::try_parse_from([
            "test-service",
            "--url",
            "https://host:8443",
            "--tofu",
            "--pki-addr",
            "http://pki.local:8080",
        ]);
        assert!(result.is_err(), "--tofu and --pki-addr should conflict");
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
