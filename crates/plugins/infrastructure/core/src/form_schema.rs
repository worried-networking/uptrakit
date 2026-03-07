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
    /// Returns field definitions for the plugin config form.
    fn form_schema() -> Vec<FieldDef>;
}
