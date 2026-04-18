use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, PluginConfigValidationError};

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

impl PluginConfig for NpmConfig {
    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value).map_err(PluginConfigValidationError::InvalidIdentifier)
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType,
        };
        vec![
            FormFieldDescriptor::new("include_prereleases", "Include Pre-releases")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Include pre-release dist-tags (next, beta, alpha, rc, canary)"),
        ]
    }
}

#[cfg(test)]
mod tests {
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
    fn secret_masking_is_noop() {
        use uptrakit_plugin_infrastructure_core::PluginConfig;
        let config = NpmConfig {
            include_prereleases: true,
            registry_url: None,
        };
        let expected = config.clone();
        let masked = config.with_secrets_masked();
        assert_eq!(masked, expected);
    }
}
