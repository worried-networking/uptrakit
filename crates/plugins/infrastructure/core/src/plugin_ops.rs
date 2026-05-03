//! Plugin operations traits — the public API for consuming plugins.
//!
//! Focused traits replace the old monolithic `PluginOps` god trait.
//! `PluginCatalog` (in the registry crate) implements the full set.
//! A blanket `PluginOps` alias exists for the few places that need everything.
//!
//! # Trait hierarchy
//!
//! - [`PluginMetadataOps`] — descriptor lookup, registry queries
//! - [`PluginConfigOps`]: [`PluginMetadataOps`] — config validation, masking, schemas
//! - [`PluginSurfaceActionOps`] — surface-oriented action routing call surface
//! - [`PluginSurfaceOps`] — plugin-backed surface registrations
//! - [`NotificationOps`] — transport lookup
//! - [`SoftwareItemLifecycleOps`] — enhancement plugin hooks
//! - [`ControllerUpdateProtectionOps`] — controller-side pre/post update protection singleton

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use uptrakit_shared_types::{PluginCapability, PluginTypeId};
use uptrakit_surfaces as surfaces;

use crate::descriptor::{
    ConfigTestOps, PluginDescriptor, PluginFamily, SurfaceActionContext, SurfaceActionError,
};
use crate::form_schema::FormFieldDescriptor;
use crate::host_requirements::{HostCompatibilityError, HostRequirements, RoleKey};
use crate::plugin_config::PluginConfigValidationError;
use crate::roles::{
    ControllerUpdateProtection, NotificationTransport, SoftwareItemCreatedEvent,
    SoftwareItemLifecycle, SoftwareItemLifecycleContext, SoftwareItemPatch,
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
    ConfigValidation(PluginConfigValidationError),
}

/// Result type for plugin operations.
pub type Result<T> = std::result::Result<T, rootcause::Report<PluginOpsError>>;

// ── TransactionalEmailError ──────────────────────────────────────────────────

/// Error type for transactional email delivery via `NotificationOps::send_transactional_email`.
#[non_exhaustive]
#[derive(Debug)]
pub enum TransactionalEmailError {
    /// No SMTP transport is configured for this tenant.
    NotConfigured,
    /// Delivery was attempted but failed.
    DeliveryFailed(String),
}

impl std::fmt::Display for TransactionalEmailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "email transport not configured"),
            Self::DeliveryFailed(msg) => write!(f, "email delivery failed: {msg}"),
        }
    }
}

impl std::error::Error for TransactionalEmailError {}

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
    ) -> std::result::Result<(), PluginConfigValidationError> {
        let desc = self.get(id).ok_or_else(|| {
            PluginConfigValidationError::Contract(format!("unknown plugin: {id}"))
        })?;
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
    fn config_form_schema(&self, id: &PluginTypeId) -> Option<Vec<FormFieldDescriptor>> {
        self.get(id).map(|d| (d.config.form_schema)())
    }

    /// Validate a package identifier.
    fn validate_package_identifier(
        &self,
        id: &PluginTypeId,
        value: &str,
    ) -> std::result::Result<(), PluginConfigValidationError> {
        let desc = self.get(id).ok_or_else(|| {
            PluginConfigValidationError::Contract(format!("unknown plugin: {id}"))
        })?;
        (desc.config.validate_identifier)(value)
    }

    /// Type-settings form field definitions.
    fn type_settings_form_schema(&self, id: &PluginTypeId) -> Option<Vec<FormFieldDescriptor>> {
        self.get(id)?.type_settings.map(|ts| (ts.form_schema)())
    }

    /// Sample type settings JSON.
    fn type_settings_sample(&self, id: &PluginTypeId) -> serde_json::Value {
        self.get(id)
            .and_then(|d| d.type_settings.map(|ts| (ts.sample)()))
            .unwrap_or_default()
    }
}

// ── Trait 3: PluginSurfaceActionOps ─────────────────────────────────────────

/// Surface action routing.
pub trait PluginSurfaceActionOps: Send + Sync + 'static {
    /// Handle a surface action invocation.
    fn handle_surface_action<'a>(
        &'a self,
        ctx: &'a SurfaceActionContext<'a>,
        surface_id: &'a str,
        action_id: &'a str,
        params: serde_json::Value,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>>
                + Send
                + 'a,
        >,
    >;
}

// ── Trait 4: PluginSurfaceOps ───────────────────────────────────────────────

/// Plugin-backed surface registration discovery.
pub trait PluginSurfaceOps: Send + Sync + 'static {
    /// Surface registrations exported by compiled-in plugin providers.
    fn surface_registrations(&self) -> Vec<surfaces::SurfaceRegistration>;
}

// ── Trait 5: NotificationOps ────────────────────────────────────────────────

/// Notification transport lookup.
#[async_trait]
pub trait NotificationOps: Send + Sync + 'static {
    /// Look up a notification transport by plugin type ID.
    fn transport(&self, id: &PluginTypeId) -> Option<std::sync::Arc<dyn NotificationTransport>>;

    /// All supported notification transport type IDs.
    fn notification_supported_types(&self) -> Vec<PluginTypeId>;

    /// Send a transactional email to a single recipient using the tenant's SMTP transport.
    ///
    /// Returns `Err(TransactionalEmailError::NotConfigured)` by default when the
    /// email feature is not enabled or no transport is available.
    #[cfg(feature = "plugin-ops")]
    async fn send_transactional_email(
        &self,
        tenant_db: &uptrakit_tenant_db::TenantDb,
        to: &str,
        subject: &str,
        text_body: &str,
        html_body: &str,
    ) -> std::result::Result<(), TransactionalEmailError> {
        let _ = (tenant_db, to, subject, text_body, html_body);
        Err(TransactionalEmailError::NotConfigured)
    }
}

// ── Trait 6: SoftwareItemLifecycleOps ───────────────────────────────────────

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

// ── Trait 7: ControllerUpdateProtectionOps ────────────────────────────────

/// Controller-side singleton update protection accessor.
pub trait ControllerUpdateProtectionOps: Send + Sync + 'static {
    /// The registered controller update protection plugin (if configured).
    fn controller_update_protection(
        &self,
    ) -> Option<std::sync::Arc<dyn ControllerUpdateProtection>>;
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
    + PluginSurfaceActionOps
    + PluginSurfaceOps
    + NotificationOps
    + SoftwareItemLifecycleOps
    + ControllerUpdateProtectionOps
{
}

/// Blanket impl: anything implementing the full trait set is automatically `PluginOps`.
impl<T> PluginOps for T where
    T: PluginMetadataOps
        + PluginConfigOps
        + PluginSurfaceActionOps
        + PluginSurfaceOps
        + NotificationOps
        + SoftwareItemLifecycleOps
        + ControllerUpdateProtectionOps
{
}
