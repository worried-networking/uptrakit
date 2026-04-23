use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

/// Configuration for the Cargo install plugin.
///
/// Tracks crates installed via `cargo install` on agent hosts.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the crate name
/// (e.g., `ripgrep`, `bat`, `cargo-nextest`).
///
/// The default config (`{}`) uses the crates.io sparse index for release lookups.
/// Set `include_prereleases: true` to include pre-release versions, or set
/// `registry_url` to use a custom/private sparse registry index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoConfig {
    /// Include pre-release crate versions (versions containing `-` in their
    /// version string, e.g. `1.0.0-alpha.1`) when fetching upstream releases.
    ///
    /// Defaults to `false` — only stable releases are reported.
    #[serde(default, skip_serializing_if = "crate::config::is_false")]
    pub include_prereleases: bool,

    /// Custom sparse Cargo registry index URL.
    ///
    /// When `None` (the default), the plugin uses the crates.io sparse index
    /// (`https://index.crates.io`). Set this to the sparse index URL of a
    /// private registry (e.g., `https://my-registry.example.com`).
    ///
    /// Private registry URLs allow private/LAN addresses (SSRF protection is
    /// relaxed via `SsrfSafeResolver::permissive()`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_url: Option<String>,

    /// Pass `--locked` to `cargo install`, using the exact dependency versions
    /// from the crate's `Cargo.lock`.
    ///
    /// Required by some crates (e.g. `cargo-nextest`) that use a
    /// `locked-tripwire` dependency to enforce deterministic builds.
    /// Set to `false` only for crates that do not ship a `Cargo.lock`.
    ///
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub use_locked: bool,
}

impl Default for CargoConfig {
    fn default() -> Self {
        Self {
            include_prereleases: false,
            registry_url: None,
            use_locked: true,
        }
    }
}

fn is_false(b: &bool) -> bool {
    !b
}

fn default_true() -> bool {
    true
}

impl PluginConfig for CargoConfig {
    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value)
    }

    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        if let Some(url) = &self.registry_url
            && url.is_empty()
        {
            return Err(PluginConfigValidationError::invalid_field(
                "registry_url",
                "must not be empty when set; omit the field to use the default crates.io sparse index",
            ));
        }
        Ok(())
    }
}

impl TypeSettings for CargoConfig {
    fn type_settings_form_schema()
    -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType,
        };
        vec![
            FormFieldDescriptor::new("include_prereleases", "Include pre-releases")
                .with_type(FormFieldType::Toggle)
                .with_help_text(
                    "Include pre-release versions (e.g. 1.0.0-alpha.1) in available updates. \
                     Defaults to false.",
                ),
            FormFieldDescriptor::new("registry_url", "Custom registry URL")
                .with_type(FormFieldType::Text)
                .with_help_text(
                    "Sparse Cargo registry index URL. Defaults to the crates.io sparse index \
                     (https://index.crates.io). Set for private registries.",
                ),
            FormFieldDescriptor::new("use_locked", "Install with --locked")
                .with_type(FormFieldType::Toggle)
                .with_help_text(
                    "Pass --locked to cargo install, using the exact dependency versions from \
                     the crate's Cargo.lock. Required by some crates (e.g. cargo-nextest). \
                     Disable only for crates that do not ship a Cargo.lock. Defaults to true.",
                ),
        ]
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({})
    }
}

impl CargoConfig {
    /// Returns the effective sparse registry index base URL.
    ///
    /// `None` (default config) uses the crates.io sparse index.
    pub(crate) fn effective_registry_url(&self) -> &str {
        self.registry_url
            .as_deref()
            .unwrap_or("https://index.crates.io")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── effective_registry_url ────────────────────────────────────────────────

    #[test]
    fn effective_registry_url_default_is_crates_io_sparse() {
        assert_eq!(
            CargoConfig::default().effective_registry_url(),
            "https://index.crates.io"
        );
    }

    #[test]
    fn effective_registry_url_custom() {
        let config = CargoConfig {
            include_prereleases: false,
            registry_url: Some("https://my-registry.example.com".to_string()),
            use_locked: true,
        };
        assert_eq!(
            config.effective_registry_url(),
            "https://my-registry.example.com"
        );
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn deserialize_empty_object_gives_defaults() {
        let config: CargoConfig = serde_json::from_str("{}").expect("deserialize");
        assert!(!config.include_prereleases);
        assert!(config.registry_url.is_none());
        assert!(config.use_locked);
    }

    #[test]
    fn deserialize_with_include_prereleases() {
        let config: CargoConfig =
            serde_json::from_str(r#"{"include_prereleases": true}"#).expect("deserialize");
        assert!(config.include_prereleases);
        assert!(config.registry_url.is_none());
        assert!(config.use_locked);
    }

    #[test]
    fn deserialize_with_registry_url() {
        let config: CargoConfig =
            serde_json::from_str(r#"{"registry_url": "https://my.registry.com"}"#)
                .expect("deserialize");
        assert!(!config.include_prereleases);
        assert_eq!(
            config.registry_url,
            Some("https://my.registry.com".to_string())
        );
        assert!(config.use_locked);
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialization_default_roundtrips() {
        let config = CargoConfig::default();
        let json = serde_json::to_value(&config).expect("serialize");
        // include_prereleases=false and registry_url=None are elided; use_locked=true is present
        assert!(json.get("include_prereleases").is_none());
        assert!(json.get("registry_url").is_none());
        assert_eq!(json["use_locked"], true);
        let deserialized: CargoConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_include_prereleases() {
        let config = CargoConfig {
            include_prereleases: true,
            registry_url: None,
            use_locked: true,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["include_prereleases"], true);
        let deserialized: CargoConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_registry_url() {
        let config = CargoConfig {
            include_prereleases: false,
            registry_url: Some("https://my.registry.com".to_string()),
            use_locked: true,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert!(
            json.get("include_prereleases")
                .is_none_or(|v| v.as_bool() != Some(false))
        );
        assert_eq!(json["registry_url"], "https://my.registry.com");
        let deserialized: CargoConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_use_locked_false_is_present() {
        let config = CargoConfig {
            include_prereleases: false,
            registry_url: None,
            use_locked: false,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["use_locked"], false);
    }

    #[test]
    fn deserialize_use_locked_false() {
        let config: CargoConfig =
            serde_json::from_str(r#"{"use_locked": false}"#).expect("deserialize");
        assert!(!config.use_locked);
    }

    // ── validate ──────────────────────────────────────────────────────────────

    #[test]
    fn validate_accepts_default_config() {
        assert!(CargoConfig::default().validate().is_ok());
    }

    #[test]
    fn validate_accepts_include_prereleases() {
        let config = CargoConfig {
            include_prereleases: true,
            registry_url: None,
            use_locked: true,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_valid_registry_url() {
        let config = CargoConfig {
            include_prereleases: false,
            registry_url: Some("https://my-registry.example.com".to_string()),
            use_locked: true,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_registry_url() {
        let config = CargoConfig {
            include_prereleases: false,
            registry_url: Some(String::new()),
            use_locked: true,
        };
        assert!(config.validate().is_err());
    }
}
