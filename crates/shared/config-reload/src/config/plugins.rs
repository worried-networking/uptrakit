use serde::{Deserialize, Serialize};

/// Plugin configuration reload signal.
///
/// Plugins are DB-driven; this carries a reload trigger when instance plugin
/// settings change in the database (sent by `ConfigReconciler` on
/// `settings_version` bump for plugin-related keys).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[non_exhaustive]
pub struct PluginsConfig {
    /// Opaque version counter incremented by `ConfigReconciler` on each plugin
    /// settings change.
    pub version: u64,
}
