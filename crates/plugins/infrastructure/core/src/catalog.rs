//! Plugin catalog — unified descriptor index with singleton management.
//!
//! [`PluginCatalog`] replaces `PluginRegistry`. It indexes `PluginDescriptor`s
//! by type ID, constructs singleton transports and lifecycle plugins at startup,
//! and provides extension action routing.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use uptrakit_shared_types::PluginTypeId;

use crate::descriptor::{
    CatalogConfig, ExtensionActionContext, ExtensionActionHandler, PluginDescriptor,
};
use crate::error::PluginError;
use crate::plugin_ops::{
    NotificationOps, PluginConfigOps, PluginExtensionOps, PluginMetadataOps,
    SoftwareItemLifecycleOps,
};
use crate::roles::{
    NotificationTransport, SoftwareItemCreatedEvent, SoftwareItemLifecycle,
    SoftwareItemLifecycleContext, SoftwareItemPatch,
};

/// Errors during catalog construction.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CatalogError {
    #[error("duplicate plugin type_id: {0}")]
    DuplicateTypeId(&'static str),

    #[error("duplicate notification transport: {0}")]
    DuplicateTransport(&'static str),

    #[error("duplicate extension prefix: {0}")]
    DuplicateExtensionPrefix(&'static str),

    #[error(
        "overlapping extension prefix: '{new_prefix}' (from {new_owner}) \
         overlaps with '{existing_prefix}' (from {existing_owner})"
    )]
    OverlappingExtensionPrefix {
        new_prefix: &'static str,
        existing_prefix: &'static str,
        new_owner: &'static str,
        existing_owner: &'static str,
    },

    #[error("failed to create singleton: {0}")]
    SingletonCreation(String),
}

/// Unified plugin catalog — indexes descriptors, manages singletons.
///
/// BTreeMap for deterministic iteration order (alphabetical by type_id).
pub struct PluginCatalog {
    descriptors: BTreeMap<&'static str, &'static PluginDescriptor>,
    transports: BTreeMap<&'static str, Arc<dyn NotificationTransport>>,
    lifecycle_plugins: Vec<Arc<dyn SoftwareItemLifecycle>>,
    extension_routes: Vec<(&'static str, ExtensionActionHandler)>,
}

impl PluginCatalog {
    /// Construct a new catalog from descriptors and shared config.
    ///
    /// Validates uniqueness of type IDs and extension prefixes.
    /// Creates singleton transports and lifecycle plugins.
    pub fn new(
        descriptors: Vec<&'static PluginDescriptor>,
        config: &CatalogConfig,
    ) -> crate::Result<Self> {
        let mut map = BTreeMap::new();
        let mut transports = BTreeMap::new();
        let mut lifecycle_plugins = Vec::new();
        let mut extension_routes = Vec::new();
        // (prefix, owner_type_id) pairs for overlap detection
        let mut seen_ext_prefixes: Vec<(&'static str, &'static str)> = Vec::new();

        for desc in descriptors {
            // ── Uniqueness: type_id ──
            if map.insert(desc.type_id, desc).is_some() {
                return Err(rootcause::report!(PluginError::UnsupportedOperation(
                    format!("duplicate plugin type_id: {}", desc.type_id)
                )));
            }

            // ── Singleton: notification transport ──
            if let Some(create) = desc.roles.notification_transport {
                if transports.contains_key(desc.type_id) {
                    return Err(rootcause::report!(PluginError::UnsupportedOperation(
                        format!("duplicate notification transport: {}", desc.type_id)
                    )));
                }
                let transport = create(config).map_err(|e| {
                    rootcause::report!(PluginError::UnsupportedOperation(format!(
                        "failed to create transport '{}': {e}",
                        desc.type_id
                    )))
                })?;
                transports.insert(desc.type_id, transport);
            }

            // ── Singleton: software item lifecycle enhancement ──
            if let Some(create) = desc.roles.software_item_lifecycle {
                let plugin = create(config).map_err(|e| {
                    rootcause::report!(PluginError::UnsupportedOperation(format!(
                        "failed to create lifecycle plugin '{}': {e}",
                        desc.type_id
                    )))
                })?;
                lifecycle_plugins.push(plugin);
            }

            // ── Uniqueness + overlap: extension prefixes ──
            if let Some(ext) = desc.extensions {
                for prefix in ext.owned_ids {
                    // Reject overlapping prefixes from DIFFERENT descriptors
                    for &(existing_prefix, owner) in &seen_ext_prefixes {
                        if owner == desc.type_id {
                            continue;
                        }
                        if prefix.starts_with(existing_prefix)
                            || existing_prefix.starts_with(prefix)
                        {
                            return Err(rootcause::report!(PluginError::UnsupportedOperation(
                                format!(
                                    "overlapping extension prefix: '{prefix}' (from {}) \
                                     overlaps with '{existing_prefix}' (from {owner})",
                                    desc.type_id
                                )
                            )));
                        }
                    }
                    seen_ext_prefixes.push((prefix, desc.type_id));
                    extension_routes.push((*prefix, ext.handle_action));
                }
            }
        }

        // Longest prefix first for greedy matching
        extension_routes.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        Ok(Self {
            descriptors: map,
            transports,
            lifecycle_plugins,
            extension_routes,
        })
    }

    /// Route an extension action to the correct handler by prefix match.
    pub fn route_extension_action(&self, ext_id: &str) -> Option<ExtensionActionHandler> {
        self.extension_routes
            .iter()
            .find(|(prefix, _)| ext_id.starts_with(prefix))
            .map(|(_, handler)| *handler)
    }

    /// Collect all controller-side database migrations contributed by plugins.
    #[cfg(feature = "migrations")]
    pub fn all_controller_migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        self.descriptors
            .values()
            .filter_map(|d| d.migrations)
            .flat_map(|f| f())
            .collect()
    }

    /// Create all compiled-in infrastructure plugin bundles.
    #[cfg(feature = "agent-infra")]
    pub fn create_infra_bundles(
        &self,
        config: &CatalogConfig,
    ) -> Vec<crate::descriptor::InfraBundle> {
        self.descriptors
            .values()
            .filter_map(|d| d.roles.infra.as_ref())
            .filter_map(|slot| (slot.create)(config).ok())
            .collect()
    }
}

// ── Trait implementations ───────────────────────────────────────────────────

impl PluginMetadataOps for PluginCatalog {
    fn get(&self, id: &PluginTypeId) -> Option<&PluginDescriptor> {
        self.descriptors.get(id.as_str()).copied()
    }

    fn all(&self) -> Vec<&PluginDescriptor> {
        self.descriptors.values().copied().collect()
    }
}

impl PluginConfigOps for PluginCatalog {} // all defaults via PluginMetadataOps

impl PluginExtensionOps for PluginCatalog {
    fn extension_manifests_and_actions(
        &self,
    ) -> Vec<(
        uptrakit_extension_framework::ExtensionManifest,
        Vec<uptrakit_extension_framework::ActionDef>,
        Option<PluginTypeId>,
    )> {
        let mut result = Vec::new();
        for desc in self.descriptors.values() {
            if let Some(ext) = desc.extensions {
                let manifests = (ext.manifests)();
                let actions = (ext.actions)();
                let owner_plugin_type_id = Some(PluginTypeId::from_static(desc.type_id));
                for manifest in manifests {
                    result.push((manifest, actions.clone(), owner_plugin_type_id.clone()));
                }
            }
        }
        result
    }

    fn handle_extension_action<'a>(
        &'a self,
        ctx: &'a ExtensionActionContext<'a>,
        ext_id: &'a str,
        action_id: &'a str,
        params: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>>
    {
        Box::pin(async move {
            let handler = self
                .route_extension_action(ext_id)
                .ok_or_else(|| format!("no plugin handles extension '{ext_id}'"))?;
            handler(ctx, ext_id, action_id, params).await
        })
    }
}

impl NotificationOps for PluginCatalog {
    fn transport(&self, id: &PluginTypeId) -> Option<Arc<dyn NotificationTransport>> {
        self.transports.get(id.as_str()).cloned()
    }

    fn notification_supported_types(&self) -> Vec<PluginTypeId> {
        self.transports
            .keys()
            .map(|k| PluginTypeId::from_static(k))
            .collect()
    }
}

impl SoftwareItemLifecycleOps for PluginCatalog {
    fn on_software_item_created<'a>(
        &'a self,
        event: &'a SoftwareItemCreatedEvent,
        ctx: &'a SoftwareItemLifecycleContext,
    ) -> Pin<Box<dyn Future<Output = Option<SoftwareItemPatch>> + Send + 'a>> {
        Box::pin(async move {
            let mut merged: Option<SoftwareItemPatch> = None;

            for plugin in &self.lifecycle_plugins {
                match plugin.on_software_item_created(event, ctx).await {
                    Ok(Some(patch)) => {
                        let m = merged.get_or_insert_with(SoftwareItemPatch::new);
                        if patch.icon_url.is_some() {
                            m.icon_url = patch.icon_url;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(
                            plugin = %plugin.plugin_type_id(),
                            error = %e,
                            "software item lifecycle plugin error"
                        );
                    }
                }
            }

            merged
        })
    }

    fn software_item_lifecycle_plugins(&self) -> &[Arc<dyn SoftwareItemLifecycle>] {
        &self.lifecycle_plugins
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, OnceLock};

    use async_trait::async_trait;
    use uptrakit_extension_framework::FieldDef;
    use uptrakit_shared_types::PluginCapability;

    use super::*;
    use crate::descriptor::*;
    use crate::roles::SoftwareItemLifecycleContext;

    struct RecordingLifecyclePlugin;

    #[async_trait]
    impl SoftwareItemLifecycle for RecordingLifecyclePlugin {
        async fn on_software_item_created(
            &self,
            _event: &SoftwareItemCreatedEvent,
            ctx: &SoftwareItemLifecycleContext,
        ) -> std::result::Result<Option<SoftwareItemPatch>, crate::error::PluginError> {
            *recorded_context()
                .lock()
                .expect("recorded context lock poisoned") = Some(ctx.clone());
            Ok(None)
        }
    }

    impl crate::roles::PluginMeta for RecordingLifecyclePlugin {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::from_static(TEST_LIFECYCLE_PLUGIN_TYPE_ID)
        }
    }

    const TEST_LIFECYCLE_PLUGIN_TYPE_ID: &str = "test.lifecycle.recording";

    static RECORDED_CONTEXT: OnceLock<Mutex<Option<SoftwareItemLifecycleContext>>> =
        OnceLock::new();

    fn recorded_context() -> &'static Mutex<Option<SoftwareItemLifecycleContext>> {
        RECORDED_CONTEXT.get_or_init(|| Mutex::new(None))
    }

    fn test_validate_config(_config: &serde_json::Value) -> std::result::Result<(), String> {
        Ok(())
    }

    fn test_mask_config_secrets(config: &serde_json::Value) -> serde_json::Value {
        config.clone()
    }

    fn test_restore_config_secrets(
        _incoming: &mut serde_json::Value,
        _existing: &serde_json::Value,
    ) {
    }

    fn test_sample_config() -> serde_json::Value {
        serde_json::Value::Null
    }

    fn test_config_form_schema() -> Vec<FieldDef> {
        vec![]
    }

    fn test_validate_identifier(_value: &str) -> std::result::Result<(), String> {
        Ok(())
    }

    fn create_recording_lifecycle(
        _config: &CatalogConfig,
    ) -> crate::error::Result<Arc<dyn SoftwareItemLifecycle>> {
        Ok(Arc::new(RecordingLifecyclePlugin))
    }

    static TEST_LIFECYCLE_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
        type_id: TEST_LIFECYCLE_PLUGIN_TYPE_ID,
        display_name: "Test Lifecycle Recording",
        family: PluginFamily::Enhancement,
        config_model: ConfigModel::None,
        capabilities: &[PluginCapability::SoftwareItemLifecycle],
        config: ConfigOps {
            validate: test_validate_config,
            mask_secrets: test_mask_config_secrets,
            restore_secrets: test_restore_config_secrets,
            sample: test_sample_config,
            form_schema: test_config_form_schema,
            validate_identifier: test_validate_identifier,
        },
        roles: RoleCreators {
            discoverer: None,
            version_detector: None,
            release_fetcher: None,
            package_indexer: None,
            update_executor: None,
            lifecycle_hook: None,
            notification_transport: None,
            software_item_lifecycle: Some(create_recording_lifecycle),
            infra: None,
        },
        extensions: None,
        type_settings: None,
        config_test: None,
        sudo: None,
        raw_settings_keys: &[],
        migrations: None,
    };

    fn test_extension_manifests() -> Vec<uptrakit_extension_framework::ExtensionManifest> {
        vec![
            serde_json::from_value(serde_json::json!({
                "id": "test.extension",
                "label": "Test Extension",
                "priority": 0,
                "placement": {
                    "type": "page",
                    "nav_section": "test"
                },
                "targeting": "universal",
                "ui": {
                    "type": "actions",
                    "actions": ["refresh"]
                }
            }))
            .expect("test manifest JSON should be valid"),
        ]
    }

    fn test_extension_actions() -> Vec<uptrakit_extension_framework::ActionDef> {
        vec![uptrakit_extension_framework::ActionDef::new(
            "refresh", "Refresh",
        )]
    }

    fn test_handle_extension_action<'a>(
        _ctx: &'a ExtensionActionContext<'a>,
        _ext_id: &'a str,
        _action_id: &'a str,
        _params: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>>
    {
        Box::pin(async { Ok(serde_json::Value::Null) })
    }

    static TEST_EXTENSION_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
        type_id: "test.extension.owner",
        display_name: "Test Extension Owner",
        family: PluginFamily::Infrastructure,
        config_model: ConfigModel::None,
        capabilities: &[],
        config: ConfigOps {
            validate: test_validate_config,
            mask_secrets: test_mask_config_secrets,
            restore_secrets: test_restore_config_secrets,
            sample: test_sample_config,
            form_schema: test_config_form_schema,
            validate_identifier: test_validate_identifier,
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
            infra: None,
        },
        extensions: Some(&ExtensionOps {
            manifests: test_extension_manifests,
            actions: test_extension_actions,
            owned_ids: &["test.extension"],
            handle_action: test_handle_extension_action,
        }),
        type_settings: None,
        config_test: None,
        sudo: None,
        raw_settings_keys: &[],
        migrations: None,
    };

    /// Empty catalog builds successfully.
    #[test]
    fn empty_catalog() {
        let catalog = PluginCatalog::new(vec![], &CatalogConfig::default()).unwrap();
        assert!(catalog.all().is_empty());
        assert!(catalog.known_type_ids().is_empty());
    }

    #[tokio::test]
    async fn forwards_lifecycle_context_to_plugins() {
        *recorded_context()
            .lock()
            .expect("recorded context lock poisoned") = None;

        let catalog =
            PluginCatalog::new(vec![&TEST_LIFECYCLE_DESCRIPTOR], &CatalogConfig::default())
                .expect("catalog should build");

        let event = SoftwareItemCreatedEvent::new(
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            "Example".to_string(),
            false,
            None,
        );

        let mut ctx = SoftwareItemLifecycleContext::default();
        let expected = serde_json::json!({ "enabled": false });
        ctx.insert_type_setting(
            PluginTypeId::from_static(TEST_LIFECYCLE_PLUGIN_TYPE_ID),
            expected.clone(),
        );

        let _ = catalog.on_software_item_created(&event, &ctx).await;

        let seen = recorded_context()
            .lock()
            .expect("recorded context lock poisoned")
            .clone()
            .expect("plugin should observe context");
        assert_eq!(
            seen.type_setting(&PluginTypeId::from_static(TEST_LIFECYCLE_PLUGIN_TYPE_ID)),
            Some(&expected)
        );
    }

    #[test]
    fn extension_manifests_include_owner_plugin_type_id() {
        let catalog =
            PluginCatalog::new(vec![&TEST_EXTENSION_DESCRIPTOR], &CatalogConfig::default())
                .expect("catalog should build");

        let extensions = catalog.extension_manifests_and_actions();
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].0.id, "test.extension");
        assert_eq!(
            extensions[0].2,
            Some(PluginTypeId::from_static(TEST_EXTENSION_DESCRIPTOR.type_id))
        );
    }
}
