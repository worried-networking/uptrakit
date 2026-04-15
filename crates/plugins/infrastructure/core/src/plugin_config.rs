//! Unified plugin configuration trait.
//!
//! [`PluginConfig`] replaces `ConfigFormSchema` + `SecretMasking` + the `validate()` convention.
//! Every plugin config struct implements this single trait, and the `declare_plugin!` macro
//! generates JSON ↔ typed delegation functions from it.
//!
//! [`TypeSettings`] is a **separate** contract for plugins with tenant-level type settings
//! (e.g., APT discovery_filter, Homebrew package_type). If `declare_plugin!` says
//! `type_settings: true` but the config doesn't implement `TypeSettings`, compilation fails.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::form_schema::FormFieldDescriptor;

/// Per-instance plugin configuration.
///
/// Every plugin config struct implements this trait. The `declare_plugin!` macro
/// generates JSON-level wrapper functions that delegate to these methods.
pub trait PluginConfig:
    Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static
{
    /// Validate the configuration after deserialization.
    ///
    /// Called by the macro-generated `validate` function pointer after JSON
    /// deserialization succeeds. Return `Err(message)` for semantic validation
    /// failures (e.g., empty required field, invalid URL format).
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }

    /// Validate a package identifier for this plugin type.
    ///
    /// Called by the catalog's `validate_package_identifier()` method.
    /// Default accepts any identifier.
    fn validate_identifier(_value: &str) -> Result<(), String> {
        Ok(())
    }

    /// Return a copy with secret fields replaced by `"***"`.
    ///
    /// Plugins with no secrets use the default (returns self unchanged).
    fn with_secrets_masked(self) -> Self {
        self
    }

    /// Restore secret fields from an existing config where `self` contains `"***"` sentinels.
    ///
    /// Plugins with no secrets use the default (no-op).
    fn restore_secrets_from(&mut self, _existing: &Self) {}

    /// Returns form field definitions for the plugin config form.
    ///
    /// Used by `GET /api/v1/plugin-types` to render typed input forms.
    /// Configs with no user-editable fields return an empty `Vec`.
    fn form_schema() -> Vec<FormFieldDescriptor> {
        vec![]
    }
}

/// Tenant-level type settings for plugins that have per-type configuration.
///
/// Separate from [`PluginConfig`] — only implemented by the ~5 configs that have
/// type settings (APT, Homebrew, Pacman, APK, Cargo). The `declare_plugin!` macro
/// requires explicit `type_settings: true` which generates a compile-time assertion
/// that the config struct implements this trait.
pub trait TypeSettings: PluginConfig {
    /// Returns form field definitions for the plugin type settings form.
    fn type_settings_form_schema() -> Vec<FormFieldDescriptor>;

    /// Returns a sample/default JSON for type settings.
    fn type_settings_sample() -> serde_json::Value;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
    struct TestConfig {
        url: String,
        token: String,
    }

    impl PluginConfig for TestConfig {
        fn validate(&self) -> Result<(), String> {
            if self.url.is_empty() {
                return Err("url is required".to_string());
            }
            Ok(())
        }

        fn with_secrets_masked(mut self) -> Self {
            if !self.token.is_empty() {
                self.token = "***".to_string();
            }
            self
        }

        fn restore_secrets_from(&mut self, existing: &Self) {
            if self.token == "***" {
                self.token = existing.token.clone();
            }
        }
    }

    #[test]
    fn validate_rejects_empty_url() {
        let cfg = TestConfig {
            url: String::new(),
            token: "tok".into(),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_accepts_valid_config() {
        let cfg = TestConfig {
            url: "https://example.com".into(),
            token: "tok".into(),
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn mask_and_restore_secrets() {
        let original = TestConfig {
            url: "https://example.com".into(),
            token: "secret123".into(),
        };
        let masked = original.clone().with_secrets_masked();
        assert_eq!(masked.token, "***");
        assert_eq!(masked.url, "https://example.com");

        let mut restored = masked;
        restored.restore_secrets_from(&original);
        assert_eq!(restored.token, "secret123");
    }

    #[test]
    fn default_validate_identifier_accepts_anything() {
        assert!(TestConfig::validate_identifier("anything").is_ok());
    }

    #[test]
    fn default_form_schema_is_empty() {
        assert!(TestConfig::form_schema().is_empty());
    }
}
