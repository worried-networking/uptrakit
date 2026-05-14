use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, PluginConfigValidationError};

use crate::lock::parse_skill_identifier;

#[expect(
    dead_code,
    reason = "used by SkillsConfig::default; not yet referenced from plugin entry point"
)]
fn default_skills_version() -> String {
    "latest".to_string()
}

/// Configuration for the Agent Skills package-manager plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[expect(
    unreachable_pub,
    reason = "will be re-exported from lib.rs once the plugin entry point is wired up"
)]
#[expect(
    dead_code,
    reason = "plugin entry point not yet wired up; struct will be constructed in a later task"
)]
pub struct SkillsConfig {
    /// npm dist-tag or version string passed to `npx skills@<version>`.
    ///
    /// Default `"latest"`. Pin to a specific version (e.g. `"1.2.3"`) or
    /// dist-tag (e.g. `"next"`) for reproducible behaviour.
    #[serde(default = "default_skills_version")]
    pub skills_version: String,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skills_version: default_skills_version(),
        }
    }
}

impl PluginConfig for SkillsConfig {
    fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
        parse_skill_identifier(value)
            .map(|_| ())
            .map_err(|e| PluginConfigValidationError::InvalidIdentifier(e.to_string()))
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType,
        };
        vec![
            FormFieldDescriptor::new("skills_version", "Skills Package Version")
                .with_type(FormFieldType::Text)
                .with_help_text(
                    "npm dist-tag or version for the skills CLI (default: \"latest\"). \
                     Pin to a specific version, e.g. \"1.2.3\" or \"next\".",
                ),
        ]
    }
}
