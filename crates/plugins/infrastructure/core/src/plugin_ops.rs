//! Abstraction over plugin registry operations needed by the web API.
//!
//! This module defines the [`PluginOps`] trait and its associated error type,
//! enabling crates like `web-api-queries` to depend on `infrastructure-core`
//! (lightweight) rather than the full `infrastructure-registry` (which pulls
//! all plugin crate implementations).

use crate::types::{PluginCapability, PluginType};

/// Errors that can occur in [`PluginOps`] trait method implementations.
#[derive(Debug, thiserror::Error)]
pub enum PluginOpsError {
    /// Unknown plugin type.
    #[error("unknown plugin type: {0}")]
    UnknownPluginType(String),

    /// Failed to parse plugin configuration.
    #[error("failed to parse config: {0}")]
    ConfigParse(String),

    /// Plugin configuration validation failed.
    #[error("config validation failed: {0}")]
    ConfigValidation(String),
}

/// Result type for [`PluginOps`] trait methods.
pub type Result<T> = std::result::Result<T, rootcause::Report<PluginOpsError>>;

/// Abstraction over the plugin registry operations needed by the web API.
///
/// Defines operations used when persisting and returning plugin
/// configurations over the REST API: config validation, secret masking for
/// API responses, and secret restoration on update. Implemented by
/// `PluginRegistry` in the `infrastructure-registry` crate.
///
/// Storing this trait in `AppState` as `Arc<dyn PluginOps>` rather than
/// referencing `PluginRegistry` directly decouples route handlers and query
/// helpers from the concrete registry, making them testable in isolation.
pub trait PluginOps: Send + Sync + 'static {
    /// Validate plugin configuration JSON for the given string plugin type.
    fn validate_config_str(&self, plugin_type: &str, config: &serde_json::Value) -> Result<()>;

    /// Mask secrets in plugin configuration JSON for an API response.
    ///
    /// Returns the config with all secret fields replaced by `"***"`.
    /// Unknown plugin types are returned unchanged.
    fn mask_config_secrets_str(
        &self,
        plugin_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value;

    /// Restore masked secrets from an existing configuration.
    ///
    /// Fields in `incoming` that equal `"***"` are replaced with the
    /// corresponding values from `existing`. Non-masked fields are left
    /// untouched.
    fn restore_config_secrets_str(
        &self,
        plugin_type: &str,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    );

    /// Returns all plugin types registered in the registry.
    ///
    /// This is the authoritative list — no hardcoded lists should exist outside
    /// the registry. Use this to populate plugin-type selectors dynamically.
    fn known_plugin_types(&self) -> Vec<PluginType>;

    /// Returns all plugin types that have the `DiscoverLocalSoftware` capability.
    fn discovery_plugins(&self) -> Vec<PluginType>;

    /// Validate a package identifier for the given string plugin type.
    ///
    /// Returns `Ok(())` for unknown plugin types (no constraints apply) and for
    /// plugin types that impose no identifier constraints. Returns `Err(message)`
    /// when the identifier violates plugin-specific rules.
    fn validate_package_identifier_str(
        &self,
        plugin_type: &str,
        value: &str,
    ) -> std::result::Result<(), String>;

    /// Returns the capabilities declared by the given plugin type.
    ///
    /// Returns an empty vec for unknown plugin types.
    fn capabilities_for_str(&self, plugin_type: &str) -> Vec<PluginCapability>;

    /// Returns a sample/default configuration JSON for the given plugin type string.
    ///
    /// Serializes the `Default` implementation of the plugin's config type.
    /// Returns an empty JSON object `{}` for unknown plugin types.
    fn sample_config_for_str(&self, plugin_type: &str) -> serde_json::Value;

    /// Returns form field definitions for the given plugin type.
    ///
    /// Returns `None` for unknown plugin types, empty `Vec` for plugins
    /// with no configurable fields.
    fn config_form_schema_str(
        &self,
        plugin_type: &str,
    ) -> Option<Vec<uptrakit_extension_framework::FieldDef>>;

    /// Returns UI extension manifests provided by all registered plugins.
    ///
    /// Default returns empty — no plugin provides extensions yet. Override
    /// when a plugin declares compile-time UI extensions.
    fn extension_manifests(&self) -> Vec<uptrakit_extension_framework::ExtensionManifest> {
        vec![]
    }

    /// Returns the action library for all registered plugins.
    ///
    /// Actions are referenced by `action_id` from the extension manifests.
    /// Default returns empty.
    fn extension_actions(&self) -> Vec<uptrakit_extension_framework::ActionDef> {
        vec![]
    }

    /// Handle an extension action invocation for a plugin-backed extension.
    ///
    /// The controller calls this when an action is invoked on an extension
    /// owned by `ExtensionOwner::Plugin`. The plugin registry dispatches to
    /// the appropriate plugin based on the extension ID prefix.
    ///
    /// Returns `Ok(json)` on success or `Err(message)` on failure.
    /// The route handler maps these to HTTP 200/422 respectively.
    fn handle_extension_action<'a>(
        &'a self,
        _ctx: &'a ExtensionActionContext<'a>,
        _extension_id: &'a str,
        _action_id: &'a str,
        _params: serde_json::Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<serde_json::Value, String>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async { Err("plugin-backed extension actions not supported".to_string()) })
    }
}

/// Context passed to plugin extension action handlers.
///
/// Provides access to the database connection and tenant/user context
/// from the authenticated HTTP request.
pub struct ExtensionActionContext<'a> {
    /// Database connection for queries.
    pub db: &'a sea_orm::DatabaseConnection,
    /// Tenant ID from the authenticated request (if available).
    pub tenant_id: Option<uuid::Uuid>,
}
