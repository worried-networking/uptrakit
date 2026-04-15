use std::path::Path;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, SecretString};
use uptrakit_shared_types::network::is_private_host;
use url::Url;

use crate::error::{GitHubError, Result};

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Maximum length for `install_path` to prevent abuse.
const MAX_INSTALL_PATH_LENGTH: usize = 4096;

/// Configuration for the GitHub Releases plugin.
///
/// Holds auth credentials, behaviour toggles, and optional asset install
/// settings — no `owner` or `repo`. Those identify *what* is tracked and are
/// expressed as the `package_identifier` of the software item (format:
/// `"owner/repo"`), not as plugin config.
///
/// A single `GitHubConfig` instance can therefore serve any number of tracked
/// GitHub repositories.
///
/// ## Asset installation
///
/// When `install_path` is set the plugin gains `execute_update` capability:
/// it downloads the matching release asset and places it at the configured
/// path. Use `asset_patterns` to narrow the selection to a single OS/arch
/// asset. Lifecycle hooks (e.g. stop/start a systemd service) are handled
/// by the hook plugin system.
///
/// Per-host overrides via `config_override` on the `execute_update` role
/// assignment allow different `asset_patterns` and `install_path` per host.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHubConfig {
    /// Optional personal access token for authentication (increases rate limits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<SecretString>,
    /// Optional custom API base URL (for GitHub Enterprise).
    /// Defaults to `https://api.github.com` when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    /// Whether to include pre-releases in the results.
    #[serde(default)]
    pub include_prereleases: bool,
    /// Prefix to strip from tags when extracting version strings (e.g. `"v"`).
    #[serde(default = "default_tag_strip_prefix")]
    pub tag_strip_prefix: String,
    /// Regex patterns to filter release assets.
    ///
    /// Only assets whose names match at least one pattern are included.
    /// An empty list means all assets are included.
    ///
    /// During `fetch_releases` (controller-side): filters which assets appear
    /// in release metadata.
    ///
    /// During `execute_update` (agent-side): selects which asset to download.
    /// Exactly one asset must match after filtering; zero or multiple matches
    /// are an error. Use per-host `config_override` to narrow patterns to the
    /// host's specific OS/architecture.
    #[serde(default)]
    pub asset_patterns: Vec<String>,
    /// Check GitHub Actions attestations for the latest release.
    ///
    /// Downloads the release checksums file and queries the GitHub Attestations
    /// API. Results are stored in `UpstreamRelease.attestation_status` and
    /// `ReleaseAsset.sha256_digest`. Default: `true`.
    #[serde(default = "default_verify_attestation")]
    pub verify_attestation: bool,
    /// Abort the update on the agent if no attestation is found.
    ///
    /// When `true`, `ExecuteUpdatePayload.release_info.require_attestation` is
    /// set to `true`, and the agent refuses to install a release with
    /// `attestation_status = NotFound`. Default: `false` (warn only).
    #[serde(default)]
    pub require_attestation: bool,

    // ── Asset installation fields ──────────────────────────────────────
    /// Absolute path where the downloaded asset is installed on the target host.
    ///
    /// When set, the plugin supports `execute_update`: it downloads the
    /// matching release asset, verifies its SHA-256 checksum (if available),
    /// and places it at this path using `install(1)` via sudo.
    ///
    /// Example: `"/usr/local/bin/pocket-id"`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,
    /// Whether to make the installed file executable (mode 0755).
    ///
    /// Default: `true` when `install_path` is set. Set to `false` for
    /// non-executable assets (e.g. data files, configuration templates).
    #[serde(default = "default_make_executable")]
    pub make_executable: bool,
}

fn default_tag_strip_prefix() -> String {
    "v".to_string()
}

fn default_verify_attestation() -> bool {
    true
}

fn default_make_executable() -> bool {
    true
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: default_tag_strip_prefix(),
            asset_patterns: vec![],
            verify_attestation: default_verify_attestation(),
            require_attestation: false,
            install_path: None,
            make_executable: default_make_executable(),
        }
    }
}

impl GitHubConfig {
    /// Validate the configuration, returning an error if any fields are invalid.
    ///
    /// An entirely empty `{}` config is valid — all fields are optional.
    pub fn validate_inner(&self) -> Result<()> {
        if let Some(ref url) = self.api_base_url {
            let parsed = Url::parse(url).map_err(|e| {
                report!(GitHubError::Configuration(format!(
                    "invalid api_base_url: {e}"
                )))
            })?;
            if parsed.scheme() != "https" {
                bail!(GitHubError::Configuration(
                    "api_base_url must use https".to_string()
                ));
            }
            let host = parsed.host_str().ok_or_else(|| {
                report!(GitHubError::Configuration(
                    "api_base_url must include a host".to_string()
                ))
            })?;
            if is_private_host(host) {
                bail!(GitHubError::Configuration(
                    "api_base_url must not point to private/loopback addresses".to_string()
                ));
            }
        }
        for pattern in &self.asset_patterns {
            regex::Regex::new(pattern).map_err(|e| {
                report!(GitHubError::InvalidPattern(format!(
                    "invalid regex pattern '{pattern}': {e}"
                )))
            })?;
        }

        // Validate install_path if set.
        if let Some(ref path) = self.install_path {
            if path.is_empty() {
                bail!(GitHubError::Configuration(
                    "install_path must not be empty".to_string()
                ));
            }
            if path.len() > MAX_INSTALL_PATH_LENGTH {
                bail!(GitHubError::Configuration(format!(
                    "install_path exceeds maximum length of {MAX_INSTALL_PATH_LENGTH}"
                )));
            }
            let p = Path::new(path);
            if !p.is_absolute() {
                bail!(GitHubError::Configuration(
                    "install_path must be an absolute path".to_string()
                ));
            }
            // Reject path traversal components.
            for component in p.components() {
                if matches!(component, std::path::Component::ParentDir) {
                    bail!(GitHubError::Configuration(
                        "install_path must not contain '..' components".to_string()
                    ));
                }
            }
            if path.contains('\0') {
                bail!(GitHubError::Configuration(
                    "install_path must not contain null bytes".to_string()
                ));
            }
        }

        Ok(())
    }

    /// Returns the API base URL, falling back to the public GitHub API.
    pub fn api_base_url(&self) -> &str {
        self.api_base_url
            .as_deref()
            .unwrap_or("https://api.github.com")
    }
}

impl PluginConfig for GitHubConfig {
    fn validate(&self) -> std::result::Result<(), String> {
        self.validate_inner().map_err(|e| e.to_string())
    }

    fn validate_identifier(value: &str) -> std::result::Result<(), String> {
        crate::validate_identifier(value)
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType,
        };
        vec![
            FormFieldDescriptor::new("auth_token", "Auth Token")
                .with_type(FormFieldType::Password)
                .sensitive()
                .with_help_text("Personal access token for private repos and higher rate limits"),
            FormFieldDescriptor::new("api_base_url", "API Base URL")
                .with_placeholder("https://api.github.com")
                .with_help_text("Custom URL for GitHub Enterprise instances"),
            FormFieldDescriptor::new("include_prereleases", "Include Pre-releases")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Include pre-release versions in results"),
            FormFieldDescriptor::new("tag_strip_prefix", "Tag Strip Prefix")
                .with_default_value(serde_json::json!("v"))
                .with_help_text(
                    "Prefix to strip from git tags (e.g. \"v\" turns \"v1.0\" into \"1.0\")",
                ),
            FormFieldDescriptor::new("asset_patterns", "Asset Patterns")
                .with_type(FormFieldType::Textarea)
                .list()
                .with_help_text("Regex patterns to filter release assets (one per line)"),
            FormFieldDescriptor::new("verify_attestation", "Verify Attestation")
                .with_type(FormFieldType::Toggle)
                .with_default_value(serde_json::json!(true))
                .with_help_text("Check GitHub Actions attestations for the latest release"),
            FormFieldDescriptor::new("require_attestation", "Require Attestation")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Abort update if no attestation is found"),
            FormFieldDescriptor::new("install_path", "Install Path").with_help_text(
                "Absolute path for the downloaded asset (e.g. /usr/local/bin/pocket-id). \
                     Enables execute_update capability.",
            ),
            FormFieldDescriptor::new("make_executable", "Make Executable")
                .with_type(FormFieldType::Toggle)
                .with_default_value(serde_json::json!(true))
                .with_help_text("Set executable permission (mode 0755) on the installed file"),
        ]
    }

    fn with_secrets_masked(mut self) -> Self {
        self.auth_token = Some(SecretString::new(SECRET_MASK));
        self
    }

    fn restore_secrets_from(&mut self, existing: &Self) {
        if let Some(ref token) = self.auth_token
            && token.expose_secret() == SECRET_MASK
        {
            self.auth_token = existing.auth_token.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_empty_config() {
        let config: GitHubConfig = serde_json::from_str("{}").expect("deserialize");
        assert!(config.auth_token.is_none());
        assert!(config.api_base_url.is_none());
        assert!(!config.include_prereleases);
        assert_eq!(config.tag_strip_prefix, "v");
        assert!(config.asset_patterns.is_empty());
        assert!(
            config.verify_attestation,
            "verify_attestation should default to true"
        );
        assert!(
            !config.require_attestation,
            "require_attestation should default to false"
        );
        assert!(config.install_path.is_none());
        assert!(
            config.make_executable,
            "make_executable should default to true"
        );
    }

    #[test]
    fn validation_passes_empty_config() {
        let config = GitHubConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_fails_invalid_regex() {
        let config = GitHubConfig {
            asset_patterns: vec!["[invalid".to_string()],
            ..GitHubConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid regex"));
    }

    #[test]
    fn validation_rejects_http_api_base_url() {
        let config = GitHubConfig {
            api_base_url: Some("http://api.github.com".to_string()),
            ..GitHubConfig::default()
        };
        let err = config.validate().err();
        assert!(err.is_some(), "expected validation error");
        if let Some(err) = err {
            assert!(err.to_string().contains("https"));
        }
    }

    #[test]
    fn validation_rejects_private_api_base_url() {
        let config = GitHubConfig {
            api_base_url: Some("https://127.0.0.1/api/v3".to_string()),
            ..GitHubConfig::default()
        };
        let err = config.validate().err();
        assert!(err.is_some(), "expected validation error");
        if let Some(err) = err {
            assert!(err.to_string().contains("private"));
        }
    }

    #[test]
    fn validation_passes_valid_regex() {
        let config = GitHubConfig {
            asset_patterns: vec![r".*\.tar\.gz$".to_string(), r".*-amd64\.deb$".to_string()],
            ..GitHubConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn serialization_roundtrip() {
        let config = GitHubConfig {
            auth_token: Some(SecretString::new("ghp_test")),
            api_base_url: Some("https://ghe.corp.com/api/v3".to_string()),
            include_prereleases: true,
            tag_strip_prefix: "release-".to_string(),
            asset_patterns: vec![r".*\.deb$".to_string()],
            verify_attestation: false,
            require_attestation: true,
            install_path: Some("/usr/local/bin/myapp".to_string()),
            make_executable: false,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: GitHubConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.auth_token, config.auth_token);
        assert_eq!(deserialized.api_base_url, config.api_base_url);
        assert_eq!(deserialized.include_prereleases, config.include_prereleases);
        assert_eq!(deserialized.tag_strip_prefix, config.tag_strip_prefix);
        assert_eq!(deserialized.asset_patterns, config.asset_patterns);
        assert_eq!(deserialized.verify_attestation, config.verify_attestation);
        assert_eq!(deserialized.require_attestation, config.require_attestation);
        assert_eq!(deserialized.install_path, config.install_path);
        assert_eq!(deserialized.make_executable, config.make_executable);
    }

    #[test]
    fn with_secrets_masked_always_shows_auth_token() {
        let config = GitHubConfig::default();
        let masked = config.with_secrets_masked();
        assert_eq!(masked.auth_token.unwrap().expose_secret(), SECRET_MASK);
    }

    #[test]
    fn with_secrets_masked_replaces_real_token() {
        let config = GitHubConfig {
            auth_token: Some(SecretString::new("ghp_real")),
            ..GitHubConfig::default()
        };
        let masked = config.with_secrets_masked();
        assert_eq!(masked.auth_token.unwrap().expose_secret(), SECRET_MASK);
    }

    #[test]
    fn restore_secrets_from_restores_masked_token() {
        let existing = GitHubConfig {
            auth_token: Some(SecretString::new("ghp_real_token")),
            ..GitHubConfig::default()
        };
        let mut incoming = existing.clone().with_secrets_masked();
        incoming.restore_secrets_from(&existing);
        assert_eq!(
            incoming.auth_token.unwrap().expose_secret(),
            "ghp_real_token"
        );
    }

    #[test]
    fn restore_secrets_from_keeps_new_token() {
        let existing = GitHubConfig {
            auth_token: Some(SecretString::new("ghp_old")),
            ..GitHubConfig::default()
        };
        let mut incoming = GitHubConfig {
            auth_token: Some(SecretString::new("ghp_new")),
            ..GitHubConfig::default()
        };
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.auth_token.unwrap().expose_secret(), "ghp_new");
    }

    #[test]
    fn api_base_url_default() {
        let config = GitHubConfig::default();
        assert_eq!(config.api_base_url(), "https://api.github.com");
    }

    #[test]
    fn api_base_url_custom() {
        let config = GitHubConfig {
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
            ..GitHubConfig::default()
        };
        assert_eq!(config.api_base_url(), "https://ghe.example.com/api/v3");
    }

    #[test]
    fn auth_token_omitted_when_none() {
        let config = GitHubConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("auth_token"));
    }

    // ── install_path validation tests ────────────────────────────────

    #[test]
    fn validation_passes_with_install_path() {
        let config = GitHubConfig {
            install_path: Some("/usr/local/bin/myapp".to_string()),
            ..GitHubConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_rejects_empty_install_path() {
        let config = GitHubConfig {
            install_path: Some(String::new()),
            ..GitHubConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn validation_rejects_relative_install_path() {
        let config = GitHubConfig {
            install_path: Some("relative/path/binary".to_string()),
            ..GitHubConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("absolute path"));
    }

    #[test]
    fn validation_rejects_traversal_in_install_path() {
        let config = GitHubConfig {
            install_path: Some("/usr/local/../etc/shadow".to_string()),
            ..GitHubConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn validation_rejects_too_long_install_path() {
        let config = GitHubConfig {
            install_path: Some(format!("/{}", "a".repeat(MAX_INSTALL_PATH_LENGTH))),
            ..GitHubConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn install_fields_omitted_when_none() {
        let config = GitHubConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("install_path"));
    }
}
