use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use uptrakit_provider_core::{SecretMasking, SecretString};
use url::Url;

use crate::error::{GitHubError, Result};

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Configuration for the GitHub Releases provider.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHubConfig {
    /// GitHub repository owner (user or organization).
    pub owner: String,
    /// GitHub repository name.
    pub repo: String,
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
    /// Prefix to strip from tags when extracting version strings (e.g. "v").
    #[serde(default = "default_tag_strip_prefix")]
    pub tag_strip_prefix: String,
    /// Regex patterns to filter release assets.
    /// Only assets whose names match at least one pattern are included.
    /// An empty list means all assets are included.
    #[serde(default)]
    pub asset_patterns: Vec<String>,
    /// Shell command to run after downloading the release.
    ///
    /// Supports `{version}`, `{tag}`, and `{package_identifier}` placeholders.
    /// If absent, no automated installation is performed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_command: Option<String>,
    /// Shell command to detect the installed version on the agent host.
    ///
    /// Supports the `{package_identifier}` placeholder (shell-escaped at runtime).
    /// The first non-empty trimmed line of stdout is used as the version string.
    /// If absent, `detect_installed_version()` returns `None`.
    ///
    /// Example: `"cat -- \"${HOME}/.{package_identifier}\""` reads a PHS version file.
    /// Example: `"myapp --version"` uses the application's built-in version output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect_installed_version_command: Option<String>,
}

fn default_tag_strip_prefix() -> String {
    "v".to_string()
}

impl GitHubConfig {
    /// Validate the configuration, returning an error if any required fields are
    /// missing or any regex patterns are invalid.
    pub fn validate(&self) -> Result<()> {
        if self.owner.is_empty() {
            bail!(GitHubError::Configuration(
                "owner must not be empty".to_string()
            ));
        }
        if self.repo.is_empty() {
            bail!(GitHubError::Configuration(
                "repo must not be empty".to_string()
            ));
        }
        if self.owner.contains('/') || self.owner.contains("..") {
            bail!(GitHubError::Configuration(
                "owner must not contain '/' or '..'".to_string()
            ));
        }
        if self.repo.contains('/') || self.repo.contains("..") {
            bail!(GitHubError::Configuration(
                "repo must not contain '/' or '..'".to_string()
            ));
        }
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

fn is_private_host(host: &str) -> bool {
    let lower = host.to_lowercase();
    if lower == "localhost"
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower.ends_with(".localhost")
    {
        return true;
    }

    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_unspecified()
                || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
        }
        Ok(IpAddr::V6(v6)) => v6.is_loopback() || v6.is_unspecified(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let json = r#"{"owner":"octocat","repo":"hello-world"}"#;
        let config: GitHubConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(config.owner, "octocat");
        assert_eq!(config.repo, "hello-world");
        assert!(config.auth_token.is_none());
        assert!(config.api_base_url.is_none());
        assert!(!config.include_prereleases);
        assert_eq!(config.tag_strip_prefix, "v");
        assert!(config.asset_patterns.is_empty());
        assert!(config.detect_installed_version_command.is_none());
    }

    #[test]
    fn validation_passes_minimal() {
        let config = GitHubConfig {
            owner: "octocat".to_string(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_fails_empty_owner() {
        let config = GitHubConfig {
            owner: String::new(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("owner"));
    }

    #[test]
    fn validation_fails_empty_repo() {
        let config = GitHubConfig {
            owner: "octocat".to_string(),
            repo: String::new(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("repo"));
    }

    #[test]
    fn validation_fails_invalid_regex() {
        let config = GitHubConfig {
            owner: "octocat".to_string(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec!["[invalid".to_string()],
            install_command: None,
            detect_installed_version_command: None,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid regex"));
    }

    #[test]
    fn validation_rejects_http_api_base_url() {
        let config = GitHubConfig {
            owner: "octocat".to_string(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: Some("http://api.github.com".to_string()),
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
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
            owner: "octocat".to_string(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: Some("https://127.0.0.1/api/v3".to_string()),
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        let err = config.validate().err();
        assert!(err.is_some(), "expected validation error");
        if let Some(err) = err {
            assert!(err.to_string().contains("private"));
        }
    }

    #[test]
    fn validation_rejects_owner_with_slash() {
        let config = GitHubConfig {
            owner: "octo/cat".to_string(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validation_rejects_repo_with_parent_traversal() {
        let config = GitHubConfig {
            owner: "octocat".to_string(),
            repo: "../hello".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validation_passes_valid_regex() {
        let config = GitHubConfig {
            owner: "octocat".to_string(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![r".*\.tar\.gz$".to_string(), r".*-amd64\.deb$".to_string()],
            install_command: None,
            detect_installed_version_command: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn serialization_roundtrip() {
        let config = GitHubConfig {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            auth_token: Some(SecretString::new("ghp_test".to_string())),
            api_base_url: Some("https://ghe.corp.com/api/v3".to_string()),
            include_prereleases: true,
            tag_strip_prefix: "release-".to_string(),
            asset_patterns: vec![r".*\.deb$".to_string()],
            install_command: None,
            detect_installed_version_command: None,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: GitHubConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.owner, config.owner);
        assert_eq!(deserialized.repo, config.repo);
        assert_eq!(deserialized.auth_token, config.auth_token);
        assert_eq!(deserialized.api_base_url, config.api_base_url);
        assert_eq!(deserialized.include_prereleases, config.include_prereleases);
        assert_eq!(deserialized.tag_strip_prefix, config.tag_strip_prefix);
        assert_eq!(deserialized.asset_patterns, config.asset_patterns);
    }

    #[test]
    fn with_secrets_masked_always_shows_auth_token() {
        let config = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        let masked = config.with_secrets_masked();
        assert_eq!(masked.auth_token.unwrap().expose_secret(), SECRET_MASK);
    }

    #[test]
    fn with_secrets_masked_replaces_real_token() {
        let config = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: Some(SecretString::new("ghp_real".to_string())),
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        let masked = config.with_secrets_masked();
        assert_eq!(masked.auth_token.unwrap().expose_secret(), SECRET_MASK);
        assert_eq!(masked.owner, "o");
    }

    #[test]
    fn restore_secrets_from_restores_masked_token() {
        let existing = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: Some(SecretString::new("ghp_real_token".to_string())),
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
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
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: Some(SecretString::new("ghp_old".to_string())),
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        let mut incoming = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: Some(SecretString::new("ghp_new".to_string())),
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.auth_token.unwrap().expose_secret(), "ghp_new");
    }

    #[test]
    fn api_base_url_default() {
        let config = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        assert_eq!(config.api_base_url(), "https://api.github.com");
    }

    #[test]
    fn api_base_url_custom() {
        let config = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: None,
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        assert_eq!(config.api_base_url(), "https://ghe.example.com/api/v3");
    }

    #[test]
    fn auth_token_omitted_when_none() {
        let config = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("auth_token"));
    }

    #[test]
    fn detect_installed_version_command_roundtrip() {
        let cmd = r#"cat -- "${HOME}/.{package_identifier}""#;
        let config = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: Some(cmd.to_string()),
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(json.contains("detect_installed_version_command"));
        let deserialized: GitHubConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            deserialized.detect_installed_version_command.as_deref(),
            Some(cmd)
        );
    }

    #[test]
    fn detect_installed_version_command_omitted_when_none() {
        let config = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
            install_command: None,
            detect_installed_version_command: None,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("detect_installed_version_command"));
    }
}
