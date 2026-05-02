use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, SecretString,
};
use uptrakit_shared_types::network::is_private_host;
use url::Url;

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Configuration for the Forgejo Releases plugin.
///
/// Holds only auth credentials and behaviour toggles — no `owner` or `repo`.
/// Those identify *what* is tracked and are expressed as the `package_identifier`
/// of the software item (format: `"owner/repo"`), not as plugin config.
///
/// A single `ForgejoConfig` instance can therefore serve any number of tracked
/// Forgejo/Gitea repositories. `api_base_url` is **required** and must point to
/// the root URL of the target Forgejo or Gitea instance (e.g. `https://codeberg.org`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForgejoConfig {
    /// Optional personal access token for authentication (increases rate limits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<SecretString>,
    /// API base URL of the target Forgejo/Gitea instance (e.g. `https://codeberg.org`).
    ///
    /// Required — must use HTTPS and must not point to a private/loopback host.
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
}

fn default_tag_strip_prefix() -> String {
    "v".to_string()
}

impl Default for ForgejoConfig {
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

impl ForgejoConfig {
    /// Validate the configuration, returning an error if any fields are invalid.
    ///
    /// `api_base_url` is required; all other fields are optional.
    pub fn validate_inner(&self) -> std::result::Result<(), PluginConfigValidationError> {
        let Some(url) = self.api_base_url.as_deref() else {
            return Err(PluginConfigValidationError::invalid_field(
                "api_base_url",
                "is required",
            ));
        };

        let parsed = Url::parse(url).map_err(|e| {
            PluginConfigValidationError::invalid_field("api_base_url", format!("invalid URL: {e}"))
        })?;

        if parsed.scheme() != "https" {
            return Err(PluginConfigValidationError::invalid_field(
                "api_base_url",
                "must use https",
            ));
        }

        let host = parsed.host_str().ok_or_else(|| {
            PluginConfigValidationError::invalid_field("api_base_url", "must include a host")
        })?;

        if is_private_host(host) {
            return Err(PluginConfigValidationError::invalid_field(
                "api_base_url",
                "must not point to private/loopback addresses",
            ));
        }

        for pattern in &self.asset_patterns {
            regex::Regex::new(pattern).map_err(|e| {
                PluginConfigValidationError::invalid_field(
                    "asset_patterns",
                    format!("invalid regex pattern '{pattern}': {e}"),
                )
            })?;
        }

        Ok(())
    }

    /// Returns the API base URL, or `None` if not configured.
    ///
    /// Use this in code paths where the config may not have been validated yet.
    /// After a successful [`validate()`](PluginConfig::validate) call (or after
    /// [`ForgejoPlugin::new()`]) this is guaranteed to be `Some`.
    pub fn api_base_url(&self) -> Option<&str> {
        self.api_base_url.as_deref()
    }
}

impl PluginConfig for ForgejoConfig {
    fn validate(&self) -> std::result::Result<(), PluginConfigValidationError> {
        self.validate_inner()
    }

    fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value).map_err(PluginConfigValidationError::InvalidIdentifier)
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType,
        };
        vec![
            FormFieldDescriptor::new("api_base_url", "API Base URL")
                .required()
                .with_placeholder("https://codeberg.org")
                .with_help_text("Root URL of the Forgejo/Gitea instance (required)"),
            FormFieldDescriptor::new("auth_token", "Auth Token")
                .with_type(FormFieldType::Password)
                .sensitive()
                .with_help_text("Personal access token for authentication"),
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
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;

    #[test]
    fn defaults_empty_config() {
        let config: ForgejoConfig = serde_json::from_str("{}").expect("deserialize");
        assert!(config.auth_token.is_none());
        assert!(config.api_base_url.is_none());
        assert!(!config.include_prereleases);
        assert_eq!(config.tag_strip_prefix, "v");
        assert!(config.asset_patterns.is_empty());
    }

    #[test]
    fn validation_fails_missing_api_base_url() {
        let config = ForgejoConfig::default();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("api_base_url"));
    }

    #[test]
    fn validation_passes_with_api_base_url() {
        let config = ForgejoConfig {
            api_base_url: Some("https://codeberg.org".to_string()),
            ..ForgejoConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_fails_invalid_regex() {
        let config = ForgejoConfig {
            api_base_url: Some("https://forgejo.example.com".to_string()),
            asset_patterns: vec!["[invalid".to_string()],
            ..ForgejoConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid regex"));
    }

    #[test]
    fn validation_rejects_http_api_base_url() {
        let config = ForgejoConfig {
            api_base_url: Some("http://codeberg.org".to_string()),
            ..ForgejoConfig::default()
        };
        let err = config.validate().err();
        assert!(err.is_some(), "expected validation error");
        if let Some(err) = err {
            assert!(err.to_string().contains("https"));
        }
    }

    #[test]
    fn validation_rejects_private_api_base_url() {
        let config = ForgejoConfig {
            api_base_url: Some("https://127.0.0.1/api/v1".to_string()),
            ..ForgejoConfig::default()
        };
        let err = config.validate().err();
        assert!(err.is_some(), "expected validation error");
        if let Some(err) = err {
            assert!(err.to_string().contains("private"));
        }
    }

    #[test]
    fn validation_passes_valid_regex() {
        let config = ForgejoConfig {
            api_base_url: Some("https://forgejo.example.com".to_string()),
            asset_patterns: vec![r".*\.tar\.gz$".to_string(), r".*-amd64\.deb$".to_string()],
            ..ForgejoConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn serialization_roundtrip() {
        let config = ForgejoConfig {
            auth_token: Some(SecretString::new("my_token")),
            api_base_url: Some("https://forgejo.example.com".to_string()),
            include_prereleases: true,
            tag_strip_prefix: "release-".to_string(),
            asset_patterns: vec![r".*\.deb$".to_string()],
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ForgejoConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.auth_token, config.auth_token);
        assert_eq!(deserialized.api_base_url, config.api_base_url);
        assert_eq!(deserialized.include_prereleases, config.include_prereleases);
        assert_eq!(deserialized.tag_strip_prefix, config.tag_strip_prefix);
        assert_eq!(deserialized.asset_patterns, config.asset_patterns);
    }

    #[test]
    fn with_secrets_masked_always_shows_auth_token() {
        let config = ForgejoConfig::default();
        let masked = config.with_secrets_masked();
        assert_eq!(masked.auth_token.unwrap().expose_secret(), SECRET_MASK);
    }

    #[test]
    fn with_secrets_masked_replaces_real_token() {
        let config = ForgejoConfig {
            auth_token: Some(SecretString::new("real_token")),
            ..ForgejoConfig::default()
        };
        let masked = config.with_secrets_masked();
        assert_eq!(masked.auth_token.unwrap().expose_secret(), SECRET_MASK);
    }

    #[test]
    fn restore_secrets_from_restores_masked_token() {
        let existing = ForgejoConfig {
            auth_token: Some(SecretString::new("real_token")),
            ..ForgejoConfig::default()
        };
        let mut incoming = existing.clone().with_secrets_masked();
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.auth_token.unwrap().expose_secret(), "real_token");
    }

    #[test]
    fn restore_secrets_from_keeps_new_token() {
        let existing = ForgejoConfig {
            auth_token: Some(SecretString::new("old_token")),
            ..ForgejoConfig::default()
        };
        let mut incoming = ForgejoConfig {
            auth_token: Some(SecretString::new("new_token")),
            ..ForgejoConfig::default()
        };
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.auth_token.unwrap().expose_secret(), "new_token");
    }

    #[test]
    fn api_base_url_returns_set_value() {
        let config = ForgejoConfig {
            api_base_url: Some("https://codeberg.org".to_string()),
            ..ForgejoConfig::default()
        };
        assert_eq!(config.api_base_url(), Some("https://codeberg.org"));
    }

    #[test]
    fn api_base_url_returns_none_when_unset() {
        let config = ForgejoConfig::default();
        assert!(config.api_base_url().is_none());
    }

    #[test]
    fn api_base_url_custom() {
        let config = ForgejoConfig {
            api_base_url: Some("https://forgejo.example.com".to_string()),
            ..ForgejoConfig::default()
        };
        assert_eq!(config.api_base_url(), Some("https://forgejo.example.com"));
    }

    #[test]
    fn auth_token_omitted_when_none() {
        let config = ForgejoConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("auth_token"));
    }
}
