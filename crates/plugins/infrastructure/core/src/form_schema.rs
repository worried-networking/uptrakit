// Re-export form schema types so plugin crates don't need a direct extension-framework dep.
pub use uptrakit_extension_framework::{FieldDef, FieldType, SelectOption};

/// Trait for plugin configs that can describe their form schema.
///
/// Implementing this trait allows the plugin registry to expose form field
/// definitions via `GET /api/v1/plugin-types`, enabling the frontend to
/// render typed input forms instead of raw JSON textareas.
///
/// Configs with no user-editable fields (e.g., `MasConfig`,
/// `ProxmoxHelperScriptsConfig`) should return an empty `Vec`.
pub trait ConfigFormSchema {
    /// Returns field definitions for the plugin config (profile/credential) form.
    fn form_schema() -> Vec<FieldDef>;

    /// Returns field definitions for the plugin type settings form.
    ///
    /// Type settings are tenant-level, per-plugin-type preferences stored in
    /// `plugin_type_settings` (e.g. APT `discovery_filter`, Homebrew
    /// `package_type`). Plugins that have no type-level settings return an
    /// empty `Vec` (the default).
    fn type_settings_form_schema() -> Vec<FieldDef> {
        vec![]
    }

    /// Returns a sample/default JSON for type settings.
    ///
    /// Used by `GET /api/v1/plugin-types` to provide a starting value for
    /// the type settings form. Default returns an empty object.
    fn type_settings_sample() -> serde_json::Value {
        serde_json::Value::Object(serde_json::Map::new())
    }
}
