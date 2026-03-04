use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{SecretMasking, SecretString};
use uptrakit_shared_types::network::is_private_host;
use url::Url;

use crate::error::{GitHubError, Result};

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Configuration for the GitHub Releases plugin.
///
/// Holds only auth credentials and behaviour toggles — no `owner` or `repo`.
/// Those identify *what* is tracked and are expressed as the `package_identifier`
/// of the software item (format: `"owner/repo"`), not as plugin config.
///
/// A single `GitHubConfig` instance can therefore serve any number of tracked
/// GitHub repositories.
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
    /// Only assets whose names match at least one pattern are included.
    /// An empty list means all assets are included.
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
}

fn default_tag_strip_prefix() -> String {
    "v".to_string()
}

fn default_verify_attestation() -> bool {
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
        }
    }
}

impl GitHubConfig {
    /// Validate a GitHub package identifier string.
    ///
    /// A valid identifier has exactly one `/`, with non-empty `owner` and `repo`
    /// parts, and neither part may contain `..`.
    ///
    /// Called by the plugin registry's `validate_package_identifier` dispatch.
    pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
        crate::validate_identifier(value)
    }

    /// Validate the configuration, returning an error if any fields are invalid.
    ///
    /// An entirely empty `{}` config is valid — all fields are optional.
    pub fn validate(&self) -> Result<()> {
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
        Ok(())
    }

    /// Returns the API base URL, falling back to the public GitHub API.
    pub fn api_base_url(&self) -> &str {
        self.api_base_url
            .as_deref()
            .unwrap_or("https://api.github.com")
    }
}

impl SecretMasking for GitHubConfig {
    /// Return a copy with secret fields masked for API responses.
    ///
    /// Unset secrets become `Some("***")` so the field always appears in JSON.
    fn with_secrets_masked(mut self) -> Self {
        self.auth_token = Some(SecretString::new(SECRET_MASK.to_string()));
        self
    }

    /// Restore masked secrets from an existing config (for PUT updates).
    ///
    /// If `auth_token` is the mask sentinel, take the value from `existing`.
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
            auth_token: Some(SecretString::new("ghp_test".to_string())),
            api_base_url: Some("https://ghe.corp.com/api/v3".to_string()),
            include_prereleases: true,
            tag_strip_prefix: "release-".to_string(),
            asset_patterns: vec![r".*\.deb$".to_string()],
            verify_attestation: false,
            require_attestation: true,
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
            auth_token: Some(SecretString::new("ghp_real".to_string())),
            ..GitHubConfig::default()
        };
        let masked = config.with_secrets_masked();
        assert_eq!(masked.auth_token.unwrap().expose_secret(), SECRET_MASK);
    }

    #[test]
    fn restore_secrets_from_restores_masked_token() {
        let existing = GitHubConfig {
            auth_token: Some(SecretString::new("ghp_real_token".to_string())),
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
            auth_token: Some(SecretString::new("ghp_old".to_string())),
            ..GitHubConfig::default()
        };
        let mut incoming = GitHubConfig {
            auth_token: Some(SecretString::new("ghp_new".to_string())),
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
}
