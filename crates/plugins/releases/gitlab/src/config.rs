use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, SecretString};
use uptrakit_shared_types::network::is_private_host;
use url::Url;

use crate::error::{GitLabError, Result};

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Configuration for the GitLab Releases plugin.
///
/// Holds only auth credentials and behaviour toggles — no project path.
/// The project path is expressed as the `package_identifier` of the software
/// item (format: `"owner/repo"` or `"group/subgroup/project"`), not as plugin
/// config. A single `GitLabConfig` instance can therefore serve any number of
/// tracked GitLab projects.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitLabConfig {
    /// Optional personal access token for authentication (increases rate limits).
    /// Must have at least `read_api` scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<SecretString>,
    /// Optional custom API base URL (for self-hosted GitLab instances).
    /// Defaults to `https://gitlab.com` when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    /// Whether to include upcoming releases in the results.
    ///
    /// GitLab marks unreleased or embargoed releases with `upcoming_release: true`.
    /// When `false` (default), such releases are skipped.
    #[serde(default)]
    pub include_prereleases: bool,
    /// Prefix to strip from tags when extracting version strings (e.g. `"v"`).
    #[serde(default = "default_tag_strip_prefix")]
    pub tag_strip_prefix: String,
    /// Regex patterns to filter release asset link names.
    /// Only asset links whose names match at least one pattern are included.
    /// An empty list means all asset links are included.
    #[serde(default)]
    pub asset_patterns: Vec<String>,
}

fn default_tag_strip_prefix() -> String {
    "v".to_string()
}

impl Default for GitLabConfig {
    fn default() -> Self {
        Self {
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: default_tag_strip_prefix(),
            asset_patterns: vec![],
        }
    }
}

impl GitLabConfig {
    /// Validate the configuration, returning an error if any fields are invalid.
    ///
    /// An entirely empty `{}` config is valid — all fields are optional.
    pub fn validate_inner(&self) -> Result<()> {
        if let Some(ref url) = self.api_base_url {
            let parsed = Url::parse(url).map_err(|e| {
                report!(GitLabError::Configuration(format!(
                    "invalid api_base_url: {e}"
                )))
            })?;
            if parsed.scheme() != "https" {
                bail!(GitLabError::Configuration(
                    "api_base_url must use https".to_string()
                ));
            }
            let host = parsed.host_str().ok_or_else(|| {
                report!(GitLabError::Configuration(
                    "api_base_url must include a host".to_string()
                ))
            })?;
            if is_private_host(host) {
                bail!(GitLabError::Configuration(
                    "api_base_url must not point to private/loopback addresses".to_string()
                ));
            }
        }
        for pattern in &self.asset_patterns {
            regex::Regex::new(pattern).map_err(|e| {
                report!(GitLabError::InvalidPattern(format!(
                    "invalid regex pattern '{pattern}': {e}"
                )))
            })?;
        }
        Ok(())
    }

    /// Returns the API base URL, falling back to the public GitLab instance.
    pub fn api_base_url(&self) -> &str {
        self.api_base_url.as_deref().unwrap_or("https://gitlab.com")
    }
}

impl PluginConfig for GitLabConfig {
    fn validate(&self) -> std::result::Result<(), String> {
        self.validate_inner().map_err(|e| e.to_string())
    }

    fn validate_identifier(value: &str) -> std::result::Result<(), String> {
        crate::validate_identifier(value)
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
        use uptrakit_plugin_infrastructure_core::form_schema::{FieldDef, FieldType};
        vec![
            FieldDef::new("auth_token", "Auth Token")
                .with_type(FieldType::Password)
                .sensitive()
                .with_help_text("Personal access token (requires read_api scope)"),
            FieldDef::new("api_base_url", "API Base URL")
                .with_placeholder("https://gitlab.com")
                .with_help_text("Custom URL for self-hosted GitLab instances"),
            FieldDef::new("include_prereleases", "Include Pre-releases")
                .with_type(FieldType::Toggle)
                .with_help_text("Include upcoming/embargoed releases in results"),
            FieldDef::new("tag_strip_prefix", "Tag Strip Prefix")
                .with_default_value(serde_json::json!("v"))
                .with_help_text(
                    "Prefix to strip from git tags (e.g. \"v\" turns \"v1.0\" into \"1.0\")",
                ),
            FieldDef::new("asset_patterns", "Asset Patterns")
                .with_type(FieldType::Textarea)
                .list()
                .with_help_text("Regex patterns to filter release asset links (one per line)"),
        ]
    }

    /// Return a copy with secret fields masked for API responses.
    ///
    /// Unset secrets become `Some("***")` so the field always appears in JSON.
    fn with_secrets_masked(mut self) -> Self {
        self.auth_token = Some(SecretString::new(SECRET_MASK));
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
        let config: GitLabConfig = serde_json::from_str("{}").expect("deserialize");
        assert!(config.auth_token.is_none());
        assert!(config.api_base_url.is_none());
        assert!(!config.include_prereleases);
        assert_eq!(config.tag_strip_prefix, "v");
        assert!(config.asset_patterns.is_empty());
    }

    #[test]
    fn validation_passes_empty_config() {
        let config = GitLabConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_fails_invalid_regex() {
        let config = GitLabConfig {
            asset_patterns: vec!["[invalid".to_string()],
            ..GitLabConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid regex"));
    }

    #[test]
    fn validation_rejects_http_api_base_url() {
        let config = GitLabConfig {
            api_base_url: Some("http://gitlab.com".to_string()),
            ..GitLabConfig::default()
        };
        let err = config.validate().err();
        assert!(err.is_some(), "expected validation error");
        if let Some(err) = err {
            assert!(err.to_string().contains("https"));
        }
    }

    #[test]
    fn validation_rejects_private_api_base_url() {
        let config = GitLabConfig {
            api_base_url: Some("https://192.168.1.1".to_string()),
            ..GitLabConfig::default()
        };
        let err = config.validate().err();
        assert!(err.is_some(), "expected validation error");
        if let Some(err) = err {
            assert!(err.to_string().contains("private"));
        }
    }

    #[test]
    fn validation_passes_valid_regex() {
        let config = GitLabConfig {
            asset_patterns: vec![r".*\.tar\.gz$".to_string(), r".*-amd64\.deb$".to_string()],
            ..GitLabConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn serialization_roundtrip() {
        let config = GitLabConfig {
            auth_token: Some(SecretString::new("glpat-test")),
            api_base_url: Some("https://gitlab.corp.com".to_string()),
            include_prereleases: true,
            tag_strip_prefix: "release-".to_string(),
            asset_patterns: vec![r".*\.deb$".to_string()],
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: GitLabConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.auth_token, config.auth_token);
        assert_eq!(deserialized.api_base_url, config.api_base_url);
        assert_eq!(deserialized.include_prereleases, config.include_prereleases);
        assert_eq!(deserialized.tag_strip_prefix, config.tag_strip_prefix);
        assert_eq!(deserialized.asset_patterns, config.asset_patterns);
    }

    #[test]
    fn with_secrets_masked_always_shows_auth_token() {
        let config = GitLabConfig::default();
        let masked = config.with_secrets_masked();
        assert_eq!(masked.auth_token.unwrap().expose_secret(), SECRET_MASK);
    }

    #[test]
    fn with_secrets_masked_replaces_real_token() {
        let config = GitLabConfig {
            auth_token: Some(SecretString::new("glpat-real")),
            ..GitLabConfig::default()
        };
        let masked = config.with_secrets_masked();
        assert_eq!(masked.auth_token.unwrap().expose_secret(), SECRET_MASK);
    }

    #[test]
    fn restore_secrets_from_restores_masked_token() {
        let existing = GitLabConfig {
            auth_token: Some(SecretString::new("glpat-real")),
            ..GitLabConfig::default()
        };
        let mut incoming = existing.clone().with_secrets_masked();
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.auth_token.unwrap().expose_secret(), "glpat-real");
    }

    #[test]
    fn restore_secrets_from_keeps_new_token() {
        let existing = GitLabConfig {
            auth_token: Some(SecretString::new("glpat-old")),
            ..GitLabConfig::default()
        };
        let mut incoming = GitLabConfig {
            auth_token: Some(SecretString::new("glpat-new")),
            ..GitLabConfig::default()
        };
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.auth_token.unwrap().expose_secret(), "glpat-new");
    }

    #[test]
    fn api_base_url_default() {
        let config = GitLabConfig::default();
        assert_eq!(config.api_base_url(), "https://gitlab.com");
    }

    #[test]
    fn api_base_url_custom() {
        let config = GitLabConfig {
            api_base_url: Some("https://gitlab.corp.com".to_string()),
            ..GitLabConfig::default()
        };
        assert_eq!(config.api_base_url(), "https://gitlab.corp.com");
    }

    #[test]
    fn auth_token_omitted_when_none() {
        let config = GitLabConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("auth_token"));
    }
}
