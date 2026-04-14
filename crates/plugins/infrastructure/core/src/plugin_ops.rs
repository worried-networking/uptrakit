//! Plugin operations traits — the public API for consuming plugins.
//!
//! Five focused traits replace the old monolithic `PluginOps` god trait.
//! `PluginCatalog` (in the registry crate) implements all five.
//! A blanket `PluginOps` alias exists for the few places that need everything.
//!
//! # Trait hierarchy
//!
//! - [`PluginMetadataOps`] — descriptor lookup, registry queries
//! - [`PluginConfigOps`]: [`PluginMetadataOps`] — config validation, masking, schemas
//! - [`PluginExtensionOps`] — extension manifests and action routing
//! - [`NotificationOps`] — transport lookup
//! - [`SoftwareItemLifecycleOps`] — enhancement plugin hooks

use std::future::Future;
use std::pin::Pin;

use uptrakit_extension_framework::{ActionDef, ExtensionManifest, FieldDef};
use uptrakit_shared_types::{PluginCapability, PluginTypeId};

use crate::descriptor::{ConfigTestOps, ExtensionActionContext, PluginDescriptor, PluginFamily};
use crate::host_requirements::{HostCompatibilityError, HostRequirements, RoleKey};
use crate::roles::{
    NotificationTransport, SoftwareItemCreatedEvent, SoftwareItemLifecycle,
    SoftwareItemLifecycleContext, SoftwareItemPatch,
};

// ── Error type ──────────────────────────────────────────────────────────────

/// Errors that can occur in plugin operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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

/// Result type for plugin operations.
pub type Result<T> = std::result::Result<T, rootcause::Report<PluginOpsError>>;

// ── Trait 1: PluginMetadataOps ──────────────────────────────────────────────

/// Descriptor lookup and registry-level queries.
///
/// Used by: plugin_configs.rs (list_plugin_types), discovery.rs,
///          discovery_allowlist.rs, controller startup.
pub trait PluginMetadataOps: Send + Sync + 'static {
    /// Look up a descriptor by plugin type ID.
    fn get(&self, id: &PluginTypeId) -> Option<&PluginDescriptor>;

    /// All registered descriptors.
    fn all(&self) -> Vec<&PluginDescriptor>;

    /// All registered plugin type IDs (deterministic order).
    fn known_type_ids(&self) -> Vec<PluginTypeId> {
        self.all()
            .iter()
            .map(|d| PluginTypeId::from_static(d.type_id))
            .collect()
    }

    /// Plugin types with `DiscoverLocalSoftware` capability.
    fn discovery_plugins(&self) -> Vec<PluginTypeId> {
        self.all()
            .iter()
            .filter(|d| {
                d.capabilities
                    .contains(&PluginCapability::DiscoverLocalSoftware)
            })
            .map(|d| PluginTypeId::from_static(d.type_id))
            .collect()
    }

    /// Capabilities for a given plugin type. Empty if unknown.
    fn capabilities(&self, id: &PluginTypeId) -> Vec<PluginCapability> {
        self.get(id)
            .map(|d| d.capabilities.to_vec())
            .unwrap_or_default()
    }

    /// Config test metadata for a given plugin type.
    fn config_test_info(&self, id: &PluginTypeId) -> Option<&'static ConfigTestOps> {
        self.get(id)?.config_test
    }

    /// All raw settings keys from all descriptors.
    fn all_raw_settings_keys(&self) -> Vec<&'static str> {
        self.all()
            .iter()
            .flat_map(|d| d.raw_settings_keys.iter().copied())
            .collect()
    }

    /// Display name for a plugin type. Returns the type ID string if unknown.
    fn display_name(&self, id: &PluginTypeId) -> String {
        self.get(id)
            .map(|d| d.display_name.to_string())
            .unwrap_or_else(|| id.to_string())
    }

    /// Check whether a plugin type has tenant-level type settings
    /// (i.e., is a "package manager" in old terminology).
    fn has_type_settings(&self, id: &PluginTypeId) -> bool {
        self.get(id).is_some_and(|d| d.type_settings.is_some())
    }

    /// Check whether a plugin type is a software plugin.
    fn is_software_plugin(&self, id: &PluginTypeId) -> bool {
        self.get(id)
            .is_some_and(|d| d.family == PluginFamily::Software)
    }

    /// Look up host requirements for a specific role of a plugin.
    fn host_requirements_for_role(
        &self,
        id: &PluginTypeId,
        role: RoleKey,
    ) -> Option<&HostRequirements> {
        let desc = self.get(id)?;
        match role {
            RoleKey::Discoverer => desc.roles.discoverer.as_ref().map(|s| &s.host_requirements),
            RoleKey::VersionDetector => desc
                .roles
                .version_detector
                .as_ref()
                .map(|s| &s.host_requirements),
            RoleKey::ReleaseFetcher => desc
                .roles
                .release_fetcher
                .as_ref()
                .map(|s| &s.host_requirements),
            RoleKey::PackageIndexer => desc
                .roles
                .package_indexer
                .as_ref()
                .map(|s| &s.host_requirements),
            RoleKey::UpdateExecutor => desc
                .roles
                .update_executor
                .as_ref()
                .map(|s| &s.host_requirements),
            RoleKey::LifecycleHook => desc
                .roles
                .lifecycle_hook
                .as_ref()
                .map(|s| &s.host_requirements),
        }
    }

    /// Validate host compatibility for a specific role assignment.
    fn validate_role_compatibility(
        &self,
        id: &PluginTypeId,
        role: RoleKey,
        host_caps: &uptrakit_shared_types::HostCapabilities,
    ) -> std::result::Result<(), rootcause::Report<HostCompatibilityError>> {
        let reqs = self.host_requirements_for_role(id, role).ok_or_else(|| {
            rootcause::report!(HostCompatibilityError::UnsupportedRole {
                plugin_type: id.to_string(),
                role,
            })
        })?;
        reqs.is_compatible_with(host_caps)
    }
}

// ── Trait 2: PluginConfigOps ────────────────────────────────────────────────

/// Config validation, secret masking, form schemas, and package identifier validation.
/// Unified for BOTH software plugin configs AND notification channel configs.
pub trait PluginConfigOps: PluginMetadataOps {
    /// Validate plugin configuration JSON.
    fn validate_config(
        &self,
        id: &PluginTypeId,
        config: &serde_json::Value,
    ) -> std::result::Result<(), String> {
        let desc = self
            .get(id)
            .ok_or_else(|| format!("unknown plugin: {id}"))?;
        (desc.config.validate)(config)
    }

    /// Mask secrets in plugin configuration JSON for API responses.
    fn mask_config_secrets(
        &self,
        id: &PluginTypeId,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        self.get(id)
            .map(|d| (d.config.mask_secrets)(config))
            .unwrap_or_else(|| config.clone())
    }

    /// Restore masked secrets from existing configuration.
    fn restore_config_secrets(
        &self,
        id: &PluginTypeId,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        if let Some(d) = self.get(id) {
            (d.config.restore_secrets)(incoming, existing);
        }
    }

    /// Sample/default configuration JSON.
    fn sample_config(&self, id: &PluginTypeId) -> serde_json::Value {
        self.get(id)
            .map(|d| (d.config.sample)())
            .unwrap_or_default()
    }

    /// Form field definitions for the plugin config.
    fn config_form_schema(&self, id: &PluginTypeId) -> Option<Vec<FieldDef>> {
        self.get(id).map(|d| (d.config.form_schema)())
    }

    /// Validate a package identifier.
    fn validate_package_identifier(
        &self,
        id: &PluginTypeId,
        value: &str,
    ) -> std::result::Result<(), String> {
        let desc = self
            .get(id)
            .ok_or_else(|| format!("unknown plugin: {id}"))?;
        (desc.config.validate_identifier)(value)
    }

    /// Type-settings form field definitions.
    fn type_settings_form_schema(&self, id: &PluginTypeId) -> Option<Vec<FieldDef>> {
        self.get(id)?.type_settings.map(|ts| (ts.form_schema)())
    }

    /// Sample type settings JSON.
    fn type_settings_sample(&self, id: &PluginTypeId) -> serde_json::Value {
        self.get(id)
            .and_then(|d| d.type_settings.map(|ts| (ts.sample)()))
            .unwrap_or_default()
    }
}

// ── Trait 3: PluginExtensionOps ─────────────────────────────────────────────

/// Extension manifest collection and action routing.
pub trait PluginExtensionOps: Send + Sync + 'static {
    /// Returns extension manifests paired with their associated action catalogues.
    fn extension_manifests_and_actions(&self) -> Vec<(ExtensionManifest, Vec<ActionDef>)>;

    /// Handle an extension action invocation.
    fn handle_extension_action<'a>(
        &'a self,
        ctx: &'a ExtensionActionContext<'a>,
        ext_id: &'a str,
        action_id: &'a str,
        params: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>>;
}

// ── Trait 4: NotificationOps ────────────────────────────────────────────────

/// Notification transport lookup.
pub trait NotificationOps: Send + Sync + 'static {
    /// Look up a notification transport by plugin type ID.
    fn transport(&self, id: &PluginTypeId) -> Option<std::sync::Arc<dyn NotificationTransport>>;

    /// All supported notification transport type IDs.
    fn notification_supported_types(&self) -> Vec<PluginTypeId>;
}

// ── Trait 5: SoftwareItemLifecycleOps ───────────────────────────────────────

/// Software item lifecycle enhancement hooks.
pub trait SoftwareItemLifecycleOps: Send + Sync + 'static {
    /// Fire `on_software_item_created` across all lifecycle plugins.
    fn on_software_item_created<'a>(
        &'a self,
        event: &'a SoftwareItemCreatedEvent,
        ctx: &'a SoftwareItemLifecycleContext,
    ) -> Pin<Box<dyn Future<Output = Option<SoftwareItemPatch>> + Send + 'a>>;

    /// All registered software-item lifecycle enhancement plugins.
    fn software_item_lifecycle_plugins(&self) -> &[std::sync::Arc<dyn SoftwareItemLifecycle>];
}

// ── Convenience alias: PluginOps ────────────────────────────────────────────

/// Combined trait for callers that need the full catalog surface.
///
/// Most code should depend on the narrower trait it actually uses.
/// This alias exists for the few places that genuinely need everything
/// (e.g., `AppState`).
pub trait PluginOps:
    PluginMetadataOps
    + PluginConfigOps
    + PluginExtensionOps
    + NotificationOps
    + SoftwareItemLifecycleOps
{
}

/// Blanket impl: anything implementing all five traits is automatically PluginOps.
impl<T> PluginOps for T where
    T: PluginMetadataOps
        + PluginConfigOps
        + PluginExtensionOps
        + NotificationOps
        + SoftwareItemLifecycleOps
{
}
