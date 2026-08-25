use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, PluginConfigValidationError};
use url::Url;

/// Configuration for the npm package manager plugin.
///
/// No secrets — npm registry queries are unauthenticated public API calls.
/// The `package_identifier` in `SoftwareItem` is the npm package name
/// (e.g., `n8n`, `@angular/cli`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpmConfig {
    /// Include pre-release dist-tags (`next`, `beta`, `alpha`, `rc`, `canary`)
    /// in `fetch_releases` results.
    ///
    /// When `false` (default), only the `latest` dist-tag is returned.
    #[serde(default)]
    pub include_prereleases: bool,

    /// Override the npm registry URL.
    ///
    /// When `None` (default), uses `https://registry.npmjs.org`.
    /// Set this to use a private registry or a self-hosted mirror.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_url: Option<String>,
}

impl NpmConfig {
    /// Validate the configuration. An entirely empty `{}` config is valid.
    ///
    /// `registry_url` deliberately accepts private hosts: a custom npm
    /// registry is operator infrastructure (for example Verdaccio on a
    /// LAN), and setting it flips the HTTP client to
    /// `SsrfMode::Permissive`. This is the documented exception to the
    /// config-time `is_private_host` convention.
    pub fn validate_inner(&self) -> std::result::Result<(), PluginConfigValidationError> {
        if let Some(ref url) = self.registry_url {
            let parsed = Url::parse(url).map_err(|e| {
                PluginConfigValidationError::invalid_field(
                    "registry_url",
                    format!("invalid URL: {e}"),
                )
            })?;
            if parsed.scheme() != "https" {
                return Err(PluginConfigValidationError::invalid_field(
                    "registry_url",
                    "must use https",
                ));
            }
            if parsed.host_str().is_none() {
                return Err(PluginConfigValidationError::invalid_field(
                    "registry_url",
                    "must include a host",
                ));
            }
        }
        Ok(())
    }
}

impl PluginConfig for NpmConfig {
    fn validate(&self) -> std::result::Result<(), PluginConfigValidationError> {
        self.validate_inner()
    }

    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value)
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType,
        };
        vec![
            FormFieldDescriptor::new("include_prereleases", "Include Pre-releases")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Include pre-release dist-tags (next, beta, alpha, rc, canary)"),
            FormFieldDescriptor::new("registry_url", "Registry URL")
                .with_type(FormFieldType::Text)
                .with_help_text(
                    "Override the npm registry. Defaults to https://registry.npmjs.org. \
                     Must use https. Set for a private registry or self-hosted mirror.",
                ),
        ]
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
    fn default_config_include_prereleases_is_false() {
        let config = NpmConfig::default();
        assert!(!config.include_prereleases);
    }

    #[test]
    fn deserialize_empty_object() {
        let config: NpmConfig = serde_json::from_str("{}").expect("deserialize");
        assert!(!config.include_prereleases);
    }

    #[test]
    fn deserialize_include_prereleases_true() {
        let config: NpmConfig =
            serde_json::from_str(r#"{"include_prereleases": true}"#).expect("deserialize");
        assert!(config.include_prereleases);
    }

    #[test]
    fn deserialize_include_prereleases_false() {
        let config: NpmConfig =
            serde_json::from_str(r#"{"include_prereleases": false}"#).expect("deserialize");
        assert!(!config.include_prereleases);
    }

    #[test]
    fn serialization_roundtrip_default() {
        let config = NpmConfig::default();
        let json = serde_json::to_value(&config).expect("serialize");
        let deserialized: NpmConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_prereleases_enabled() {
        let config = NpmConfig {
            include_prereleases: true,
            registry_url: None,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["include_prereleases"], true);
        let deserialized: NpmConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        use uptrakit_plugin_infrastructure_core::PluginConfig;
        assert!(NpmConfig::default().validate().is_ok());
    }

    #[test]
    fn form_schema_exposes_registry_url() {
        use uptrakit_plugin_infrastructure_core::PluginConfig;
        let fields = NpmConfig::form_schema();
        assert!(
            fields.iter().any(|f| f.key == "registry_url"),
            "form_schema() must expose registry_url so operators can configure a private registry"
        );
    }

    // ── validate_inner ────────────────────────────────────────────────────

    #[test]
    fn empty_config_is_valid() {
        let config = NpmConfig::default();
        assert!(config.validate_inner().is_ok());
    }

    #[test]
    fn https_private_host_registry_is_accepted() {
        // Deliberate exception: custom registries may point at private/LAN
        // hosts (e.g. Verdaccio), unlike release-source plugins.
        let config = NpmConfig {
            include_prereleases: false,
            registry_url: Some("https://npm.internal.lan".to_string()),
        };
        assert!(config.validate_inner().is_ok());
    }

    #[test]
    fn http_registry_is_rejected() {
        let config = NpmConfig {
            include_prereleases: false,
            registry_url: Some("http://registry.example.com".to_string()),
        };
        let err = config.validate_inner().expect_err("should fail");
        assert!(matches!(
            err,
            PluginConfigValidationError::InvalidField {
                field: "registry_url",
                ..
            }
        ));
    }

    #[test]
    fn garbage_url_is_rejected() {
        let config = NpmConfig {
            include_prereleases: false,
            registry_url: Some("not a url".to_string()),
        };
        let err = config.validate_inner().expect_err("should fail");
        assert!(matches!(
            err,
            PluginConfigValidationError::InvalidField {
                field: "registry_url",
                ..
            }
        ));
    }
}
