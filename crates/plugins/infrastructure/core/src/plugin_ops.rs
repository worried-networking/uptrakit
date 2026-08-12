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

    /// Returns `true` if the plugin is "instance-enabled" at the catalog
    /// snapshot taken at controller boot.
    ///
    /// Semantics:
    /// - For `scope == Tenant` plugins: always `true` (no instance-level kill switch exists).
    /// - For `scope == Instance` plugins: the snapshot value loaded at boot.
    /// - For unknown plugin ids: `false`.
    ///
    /// This reflects the *running* catalog state, not the live DB row.
    fn instance_enabled(&self, id: &PluginTypeId) -> bool;

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
    ///
    /// Key-set masking: only paths present in the object are replaced with
    /// `"***"`; sparse objects never gain keys. Unknown plugin type is a
    /// passthrough (unreachable in practice — responses are filtered to
    /// cataloged plugins).
    fn mask_config_secrets(
        &self,
        id: &PluginTypeId,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        crate::secret_paths::mask_present_keys(config, &self.sensitive_paths(id))
    }

    /// Restore masked secrets from existing configuration.
    fn restore_config_secrets(
        &self,
        id: &PluginTypeId,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        crate::secret_paths::restore_masked_keys(incoming, existing, &self.sensitive_paths(id));
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

    /// Effective sensitive dotted paths for a plugin: the UNION of the
    /// schema-derived set (a field is sensitive when `sensitive == true` OR
    /// its type is `Password`/`SshPrivateKey`, across the config,
    /// type-settings, and instance-config form schemas) and the descriptor's
    /// explicit `sensitive_paths` declarations. Explicit declarations may
    /// only add, never shrink, the derived set. Sorted and deduplicated.
    /// Unknown plugin type yields an empty set (masking is a passthrough).
    fn sensitive_paths(&self, id: &PluginTypeId) -> Vec<String> {
        let Some(desc) = self.get(id) else {
            return Vec::new();
        };
        let mut paths: Vec<String> = desc
            .sensitive_paths
            .iter()
            .map(|p| (*p).to_string())
            .collect();
        let schemas = [
            Some((desc.config.form_schema)()),
            desc.type_settings.map(|ts| (ts.form_schema)()),
            desc.instance_config.map(|ic| (ic.form_schema)()),
        ];
        for schema in schemas.into_iter().flatten() {
            for field in schema {
                let secret_typed = matches!(
                    field.field_type,
                    crate::form_schema::FormFieldType::Password
                        | crate::form_schema::FormFieldType::SshPrivateKey
                );
                if field.sensitive || secret_typed {
                    paths.push(crate::secret_paths::normalize_form_key(&field.key));
                }
            }
        }
        paths.sort();
        paths.dedup();
        paths
    }

    /// Reject a config write whose incoming value still carries the mask
    /// sentinel at a sensitive path (i.e. the client echoed back a masked
    /// value that could not be resolved against a stored config).
    fn assert_no_sentinel(
        &self,
        id: &PluginTypeId,
        config: &serde_json::Value,
    ) -> std::result::Result<(), PluginConfigValidationError> {
        let paths = self.sensitive_paths(id);
        match crate::secret_paths::first_sentinel_path(config, &paths) {
            Some(path) => Err(PluginConfigValidationError::Contract(format!(
                "sensitive field '{path}' still contains the masked sentinel \"{sentinel}\"; \
                 re-enter the secret value",
                sentinel = crate::secret_paths::SECRET_SENTINEL
            ))),
            None => Ok(()),
        }
    }

    /// True when any sensitive path differs between the incoming and stored
    /// configs (covers add, change, and removal of a credential).
    fn sensitive_value_changed_for(
        &self,
        id: &PluginTypeId,
        incoming: &serde_json::Value,
        stored: &serde_json::Value,
    ) -> bool {
        let paths = self.sensitive_paths(id);
        crate::secret_paths::sensitive_value_changed(incoming, stored, &paths)
    }

    /// Deserialize→reserialize a config through its typed representation to
    /// prune stale variant keys. Unknown plugin types are a passthrough.
    fn normalize_config(
        &self,
        id: &PluginTypeId,
        config: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, PluginConfigValidationError> {
        match self.get(id) {
            Some(desc) => (desc.config.normalize)(config),
            None => Ok(config.clone()),
        }
    }

    /// Remove every sensitive path present in `config`; returns the paths
    /// actually removed (autodiscovery strip-and-warn).
    fn strip_sensitive_paths_from(
        &self,
        id: &PluginTypeId,
        config: &mut serde_json::Value,
    ) -> Vec<String> {
        let paths = self.sensitive_paths(id);
        crate::secret_paths::strip_sensitive_paths(config, &paths)
    }

    /// First sensitive path present in `config`, if any (layer-3 reject).
    fn first_sensitive_path_in(
        &self,
        id: &PluginTypeId,
        config: &serde_json::Value,
    ) -> Option<String> {
        let paths = self.sensitive_paths(id);
        crate::secret_paths::first_sensitive_path_present(config, &paths)
    }

    /// True when `config` holds a live (present, non-null, non-sentinel)
    /// value at any sensitive path. Used to stamp `credential_updated_at`
    /// on create.
    fn has_live_secret_in(&self, id: &PluginTypeId, config: &serde_json::Value) -> bool {
        let paths = self.sensitive_paths(id);
        crate::secret_paths::has_live_secret_value(config, &paths)
    }

    /// Prune-only variant hygiene (spec §5): remove sensitive paths present
    /// in `config` but absent from its typed round-trip. Returns pruned paths.
    ///
    /// A config that does not round-trip is left untouched: every write path
    /// validates before pruning, so a normalize failure here means the value
    /// is already rejected (or will be by the sentinel assertion that follows).
    fn prune_stale_sensitive_keys(
        &self,
        id: &PluginTypeId,
        config: &mut serde_json::Value,
    ) -> Vec<String> {
        let round_trip = match self.normalize_config(id, config) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(plugin_type = %id, error = %e, "skipping sensitive-key prune");
                return Vec::new();
            }
        };
        let stale: Vec<String> = self
            .sensitive_paths(id)
            .into_iter()
            .filter(|p| {
                let single = std::slice::from_ref(p);
                crate::secret_paths::first_sensitive_path_present(config, single).is_some()
                    && crate::secret_paths::first_sensitive_path_present(&round_trip, single)
                        .is_none()
            })
            .collect();
        crate::secret_paths::strip_sensitive_paths(config, &stale)
    }
}

// ── Trait 3: PluginSurfaceActionOps ─────────────────────────────────────────

/// Surface action routing.
#[async_trait]
pub trait PluginSurfaceActionOps: Send + Sync + 'static {
    /// Handle a surface action invocation.
    ///
    /// `method` is the effective HTTP method of the resolved interaction
    /// (ADR-0030 `(id, effective_http_method)` model). A single interaction ID
    /// may be registered under several methods (e.g. a GET read and a PUT
    /// write sharing `smtp`); `method` selects the matching handler.
    async fn handle_surface_action(
        &self,
        ctx: &SurfaceActionContext<'_>,
        surface_id: &str,
        action_id: &str,
        method: surfaces::InteractionHttpMethod,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, SurfaceActionError>;
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
#[async_trait]
pub trait SoftwareItemLifecycleOps: Send + Sync + 'static {
    /// Fire `on_software_item_created` across all lifecycle plugins.
    async fn on_software_item_created(
        &self,
        event: &SoftwareItemCreatedEvent,
        ctx: &SoftwareItemLifecycleContext,
    ) -> Option<SoftwareItemPatch>;

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

// ── Trait 8: ControllerUpdateHookOps ────────────────────────────────────────

/// Controller-side singleton update hook accessor.
pub trait ControllerUpdateHookOps: Send + Sync + 'static {
    /// The registered controller update hook plugin (if configured).
    #[cfg(feature = "plugin-ops")]
    fn controller_update_hook(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::roles::ControllerUpdateHook>> {
        None
    }
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
    + ControllerUpdateHookOps
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
        + ControllerUpdateHookOps
{
}

#[cfg(all(test, feature = "plugin-ops"))]
mod update_hook_ops_tests {
    use super::*;

    struct TestOps;
    impl ControllerUpdateHookOps for TestOps {}

    #[test]
    fn default_impl_returns_none() {
        let ops = TestOps;
        assert!(ops.controller_update_hook().is_none());
    }
}

#[cfg(test)]
mod sensitive_paths_tests {
    use super::*;
    use crate::descriptor::{ConfigModel, ConfigOps, PluginScope, RoleCreators};
    use crate::form_schema::FormFieldType;

    fn noop_validate(
        _: &serde_json::Value,
    ) -> std::result::Result<(), PluginConfigValidationError> {
        Ok(())
    }

    fn noop_normalize(
        config: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, PluginConfigValidationError> {
        Ok(config.clone())
    }

    fn noop_sample() -> serde_json::Value {
        serde_json::json!({})
    }

    fn noop_validate_identifier(_: &str) -> std::result::Result<(), PluginConfigValidationError> {
        Ok(())
    }

    fn stub_form_schema() -> Vec<FormFieldDescriptor> {
        vec![
            FormFieldDescriptor::new("auth._type", "Auth Type"),
            FormFieldDescriptor::new("api_token", "API Token").with_type(FormFieldType::Password),
            FormFieldDescriptor::new("webhook_secret", "Webhook Secret").sensitive(),
        ]
    }

    static STUB_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
        type_id: "stub.plugin",
        display_name: "Stub Plugin",
        family: PluginFamily::Software,
        config_model: ConfigModel::PluginConfig,
        capabilities: &[],
        scope: PluginScope::Tenant,
        instance_config: None,
        sensitive_paths: &["extra_declared"],
        config: ConfigOps {
            validate: noop_validate,
            normalize: noop_normalize,
            sample: noop_sample,
            form_schema: stub_form_schema,
            validate_identifier: noop_validate_identifier,
        },
        roles: RoleCreators {
            discoverer: None,
            version_detector: None,
            release_fetcher: None,
            package_indexer: None,
            update_executor: None,
            lifecycle_hook: None,
            notification_transport: None,
            software_item_lifecycle: None,
            controller_update_protection: None,
            controller_update_hook: None,
            infra: None,
            installed_version_enricher: None,
        },
        surfaces: None,
        type_settings: None,
        config_test: None,
        sudo: None,
        raw_settings_keys: &[],
        global_provider_consumers: &[],
        migrations: None,
        agent_migrations: None,
        agent_surfaces: None,
        reset_tenant_data: None,
        db_migrate_tables: None,
    };

    struct StubOps;

    impl PluginMetadataOps for StubOps {
        fn get(&self, id: &PluginTypeId) -> Option<&PluginDescriptor> {
            if id.as_str() == STUB_DESCRIPTOR.type_id {
                Some(&STUB_DESCRIPTOR)
            } else {
                None
            }
        }

        fn all(&self) -> Vec<&PluginDescriptor> {
            vec![&STUB_DESCRIPTOR]
        }

        fn instance_enabled(&self, _id: &PluginTypeId) -> bool {
            true
        }
    }

    impl PluginConfigOps for StubOps {}

    #[test]
    fn derivation_unions_schema_and_declarations() {
        let ops = StubOps;
        let id = PluginTypeId::new("stub.plugin".to_string());
        let paths = ops.sensitive_paths(&id);
        // Password-typed without .sensitive() IS included — no live plugin
        // currently has such a field (every Password field also calls
        // .sensitive(), e.g. proxmox config.rs:108-111), so this guards the
        // future case where an author sets the type but forgets the flag;
        // .sensitive() Text included; explicit declaration included; plain Text
        // ("auth.type" after `_`-normalization) excluded. Sorted + deduped.
        assert_eq!(paths, vec!["api_token", "extra_declared", "webhook_secret"]);
    }

    #[test]
    fn unknown_plugin_has_no_paths_and_mask_is_passthrough() {
        let ops = StubOps;
        let id = PluginTypeId::new("no.such".to_string());
        assert!(ops.sensitive_paths(&id).is_empty());
    }

    #[test]
    fn sentinel_assert_names_the_path() {
        let ops = StubOps;
        let id = PluginTypeId::new("stub.plugin".to_string());
        let err = ops
            .assert_no_sentinel(&id, &serde_json::json!({"api_token": "***"}))
            .expect_err("sentinel must be rejected");
        assert_eq!(
            err.to_string(),
            "sensitive field 'api_token' still contains the masked sentinel \"***\"; \
             re-enter the secret value"
        );
    }
}
