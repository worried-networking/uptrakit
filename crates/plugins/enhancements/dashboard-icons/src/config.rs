use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::form_schema::{FormFieldDescriptor, FormFieldType};
use uptrakit_plugin_infrastructure_core::{PluginConfig, TypeSettings};

/// Type settings for the Dashboard Icons enhancement.
///
/// This plugin is enabled by default when the setting is absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardIconsConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

const fn default_enabled() -> bool {
    true
}

impl Default for DashboardIconsConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
        }
    }
}

impl PluginConfig for DashboardIconsConfig {}

impl TypeSettings for DashboardIconsConfig {
    fn type_settings_form_schema() -> Vec<FormFieldDescriptor> {
        vec![
            FormFieldDescriptor::new("enabled", "Enabled")
                .with_type(FormFieldType::Toggle)
                .with_default_value(serde_json::json!(true))
                .with_help_text("Enable automatic icon enrichment for software items"),
        ]
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({ "enabled": true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_enabled() {
        assert!(DashboardIconsConfig::default().enabled);
    }

    #[test]
    fn type_settings_schema_and_sample_expose_enabled_toggle() {
        let fields = DashboardIconsConfig::type_settings_form_schema();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, "enabled");
        assert_eq!(fields[0].field_type, FormFieldType::Toggle);
        assert_eq!(fields[0].default_value, Some(serde_json::json!(true)));

        assert_eq!(
            DashboardIconsConfig::type_settings_sample(),
            serde_json::json!({ "enabled": true })
        );
    }
}
