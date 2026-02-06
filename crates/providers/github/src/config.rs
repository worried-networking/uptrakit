use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::{GitHubError, Result};

/// Configuration for the GitHub Releases provider.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHubConfig {
    /// GitHub repository owner (user or organization).
    pub owner: String,
    /// GitHub repository name.
    pub repo: String,
    /// Optional personal access token for authentication (increases rate limits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
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
}

fn default_tag_strip_prefix() -> String {
    "v".to_string()
}

impl GitHubConfig {
    /// Validate the configuration, returning an error if any required fields are
    /// missing or any regex patterns are invalid.
    pub fn validate(&self) -> Result<()> {
        if self.owner.is_empty() {
            return Err(report!(GitHubError::Configuration(
                "owner must not be empty".to_string()
            )));
        }
        if self.repo.is_empty() {
            return Err(report!(GitHubError::Configuration(
                "repo must not be empty".to_string()
            )));
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
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid regex"));
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
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn serialization_roundtrip() {
        let config = GitHubConfig {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            auth_token: Some("ghp_test".to_string()),
            api_base_url: Some("https://ghe.corp.com/api/v3".to_string()),
            include_prereleases: true,
            tag_strip_prefix: "release-".to_string(),
            asset_patterns: vec![r".*\.deb$".to_string()],
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
    fn api_base_url_default() {
        let config = GitHubConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
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
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("auth_token"));
    }
}
