//! Plugin catalog — unified descriptor index with singleton management.
//!
//! [`PluginCatalog`] replaces `PluginRegistry`. It indexes `PluginDescriptor`s
//! by type ID, constructs singleton transports and lifecycle plugins at startup,
//! and provides surface action routing.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use uptrakit_shared_types::PluginTypeId;

use crate::descriptor::{
    CatalogConfig, PluginDescriptor, SurfaceActionContext, SurfaceActionHandler,
};
use crate::error::PluginError;
use crate::plugin_ops::{
    ControllerUpdateProtectionOps, NotificationOps, PluginConfigOps, PluginMetadataOps,
    PluginSurfaceActionOps, PluginSurfaceOps, SoftwareItemLifecycleOps,
};
use crate::roles::{
    ControllerUpdateProtection, NotificationTransport, SoftwareItemCreatedEvent,
    SoftwareItemLifecycle, SoftwareItemLifecycleContext, SoftwareItemPatch,
};

/// Errors during catalog construction.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CatalogError {
    #[error("duplicate plugin type_id: {0}")]
    DuplicateTypeId(&'static str),

    #[error("duplicate notification transport: {0}")]
    DuplicateTransport(&'static str),

    #[error("duplicate surface action prefix: {0}")]
    DuplicateSurfaceActionPrefix(&'static str),

    #[error(
        "overlapping surface action prefix: '{new_prefix}' (from {new_owner}) \
         overlaps with '{existing_prefix}' (from {existing_owner})"
    )]
    OverlappingSurfaceActionPrefix {
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
    controller_update_protection: Option<Arc<dyn ControllerUpdateProtection>>,
    surface_action_routes: Vec<(&'static str, SurfaceActionHandler)>,
}

impl PluginCatalog {
    /// Construct a new catalog from descriptors and shared config.
    ///
    /// Validates uniqueness of type IDs and surface action prefixes.
    /// Creates singleton transports and lifecycle plugins.
    pub fn new(
        descriptors: Vec<&'static PluginDescriptor>,
        config: &CatalogConfig,
    ) -> crate::Result<Self> {
        let mut map = BTreeMap::new();
        let mut transports = BTreeMap::new();
        let mut lifecycle_plugins = Vec::new();
        let mut controller_update_protection: Option<Arc<dyn ControllerUpdateProtection>> = None;
        let mut surface_action_routes = Vec::new();
        // (prefix, owner_type_id) pairs for overlap detection
        let mut seen_surface_prefixes: Vec<(&'static str, &'static str)> = Vec::new();

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

            // ── Singleton: controller update protection ──
            if let Some(create) = desc.roles.controller_update_protection {
                if controller_update_protection.is_some() {
                    return Err(rootcause::report!(PluginError::UnsupportedOperation(
                        format!("duplicate controller update protection: {}", desc.type_id)
                    )));
                }
                let plugin = create(config).map_err(|e| {
                    rootcause::report!(PluginError::UnsupportedOperation(format!(
                        "failed to create controller update protection '{}': {e}",
                        desc.type_id
                    )))
                })?;
                controller_update_protection = Some(plugin);
            }

            // ── Uniqueness + overlap: surface action prefixes ──
            if let Some(ext) = desc.surface_actions {
                for prefix in ext.owned_surface_ids() {
                    // Reject overlapping prefixes from DIFFERENT descriptors
                    for &(existing_prefix, owner) in &seen_surface_prefixes {
                        if owner == desc.type_id {
                            continue;
                        }
                        if prefix.starts_with(existing_prefix)
                            || existing_prefix.starts_with(prefix)
                        {
                            return Err(rootcause::report!(PluginError::UnsupportedOperation(
                                format!(
                                    "overlapping surface action prefix: '{prefix}' (from {}) \
                                     overlaps with '{existing_prefix}' (from {owner})",
                                    desc.type_id
                                )
                            )));
                        }
                    }
                    seen_surface_prefixes.push((prefix, desc.type_id));
                    surface_action_routes.push((*prefix, ext.handle_action));
                }
            }
        }

        // Longest prefix first for greedy matching
        surface_action_routes.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        Ok(Self {
            descriptors: map,
            transports,
            lifecycle_plugins,
            controller_update_protection,
            surface_action_routes,
        })
    }

    /// Route a surface action to the correct handler by prefix match.
    pub fn route_surface_action(&self, surface_id: &str) -> Option<SurfaceActionHandler> {
        self.surface_action_routes
            .iter()
            .find(|(prefix, _)| surface_id.starts_with(prefix))
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

impl PluginSurfaceActionOps for PluginCatalog {
    fn handle_surface_action<'a>(
        &'a self,
        ctx: &'a SurfaceActionContext<'a>,
        surface_id: &'a str,
        action_id: &'a str,
        params: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>>
    {
        Box::pin(async move {
            let handler = self
                .route_surface_action(surface_id)
                .ok_or_else(|| format!("no plugin handles surface '{surface_id}'"))?;
            handler(ctx, surface_id, action_id, params).await
        })
    }
}

impl PluginSurfaceOps for PluginCatalog {
    fn surface_registrations(&self) -> Vec<uptrakit_internal_wire::surfaces::SurfaceRegistration> {
        self.descriptors
            .values()
            .filter_map(|descriptor| descriptor.surfaces)
            .flat_map(|surface_ops| (surface_ops.registrations)())
            .collect()
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

impl ControllerUpdateProtectionOps for PluginCatalog {
    fn controller_update_protection(&self) -> Option<Arc<dyn ControllerUpdateProtection>> {
        self.controller_update_protection.clone()
    }
}

#[cfg(test)]
#[allow(dead_code, unreachable_pub)]
mod tests {
    use std::sync::{Arc, Mutex, OnceLock};

    use async_trait::async_trait;
    use uptrakit_internal_wire::surfaces;
    use uptrakit_shared_types::PluginCapability;

    use super::*;
    use crate::descriptor::*;
    use crate::form_schema::FormFieldDescriptor;
    use crate::plugin_ops::PluginOps;
    use crate::roles::{
        ControllerPostUpdateContext, ControllerProtectionContext, ControllerProtectionDecision,
        ControllerUpdateProtection, PostUpdateOutcome, SoftwareItemLifecycleContext,
    };

    fn noop_validate(_: &serde_json::Value) -> std::result::Result<(), String> {
        Ok(())
    }

    fn noop_mask(config: &serde_json::Value) -> serde_json::Value {
        config.clone()
    }

    fn noop_restore(_: &mut serde_json::Value, _: &serde_json::Value) {}

    fn noop_sample() -> serde_json::Value {
        serde_json::json!({})
    }

    fn noop_form_schema() -> Vec<FormFieldDescriptor> {
        vec![]
    }

    fn noop_validate_identifier(_: &str) -> std::result::Result<(), String> {
        Ok(())
    }

    #[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
    struct TestGlobalProviderConfig;

    impl crate::PluginConfig for TestGlobalProviderConfig {}

    #[allow(dead_code)]
    struct TestGlobalProviderPlugin;

    const TEST_GLOBAL_PROVIDER_PLUGIN_TYPE_ID: &str = "__test_global_provider_plugin";

    crate::declare_plugin!(
        TestGlobalProviderPlugin,
        TestGlobalProviderConfig,
        TEST_GLOBAL_PROVIDER_PLUGIN_TYPE_ID,
        {
            display_name: "Test Global Provider Consumer",
            family: PluginFamily::Enhancement,
            config_model: ConfigModel::None,
            roles: [],
            global_provider_consumers: ["github"],
        }
    );

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

    fn create_recording_lifecycle(
        _config: &CatalogConfig,
    ) -> crate::error::Result<Arc<dyn SoftwareItemLifecycle>> {
        Ok(Arc::new(RecordingLifecyclePlugin))
    }

    struct TestControllerProtectionPlugin {
        plugin_type_id: PluginTypeId,
    }

    #[async_trait]
    impl ControllerUpdateProtection for TestControllerProtectionPlugin {
        async fn prepare_pre_update_protection(
            &self,
            _ctx: &ControllerProtectionContext<'_>,
        ) -> crate::error::Result<ControllerProtectionDecision> {
            Ok(ControllerProtectionDecision {
                attempted: false,
                succeeded: false,
                protection_status: None,
                protection_summary: None,
            })
        }

        async fn finalize_post_update(
            &self,
            _ctx: &ControllerPostUpdateContext<'_>,
        ) -> crate::error::Result<PostUpdateOutcome> {
            Ok(PostUpdateOutcome {
                recovery_hint: None,
            })
        }
    }

    impl crate::roles::PluginMeta for TestControllerProtectionPlugin {
        fn plugin_type_id(&self) -> PluginTypeId {
            self.plugin_type_id.clone()
        }
    }

    fn create_controller_update_protection_a(
        _config: &CatalogConfig,
    ) -> crate::error::Result<Arc<dyn ControllerUpdateProtection>> {
        Ok(Arc::new(TestControllerProtectionPlugin {
            plugin_type_id: PluginTypeId::from_static("__test_controller_protection_a"),
        }))
    }

    fn create_controller_update_protection_b(
        _config: &CatalogConfig,
    ) -> crate::error::Result<Arc<dyn ControllerUpdateProtection>> {
        Ok(Arc::new(TestControllerProtectionPlugin {
            plugin_type_id: PluginTypeId::from_static("__test_controller_protection_b"),
        }))
    }

    fn test_plugin_surface_registrations() -> Vec<surfaces::SurfaceRegistration> {
        vec![surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "plugin.test_provider".to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("plugin.test.surface").unwrap(),
                    label: "Test surface".to_string(),
                    priority: 100,
                    slot: surfaces::SLOT_SETTINGS_TABS.to_string(),
                    scope: surfaces::Scope::Global,
                    targeting: surfaces::Targeting::Universal,
                    required_permission: None,
                    provider_kind: surfaces::ProviderKind::Plugin,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    },
                },
                interactions: vec![],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }]
    }

    static TEST_SURFACE_OPS: SurfaceRegistrationOps = SurfaceRegistrationOps {
        registrations: test_plugin_surface_registrations,
    };

    static TEST_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
        type_id: "__test_surface_plugin",
        display_name: "Test Surface Plugin",
        family: PluginFamily::Software,
        config_model: ConfigModel::None,
        capabilities: &[],
        config: ConfigOps {
            validate: noop_validate,
            mask_secrets: noop_mask,
            restore_secrets: noop_restore,
            sample: noop_sample,
            form_schema: noop_form_schema,
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
            infra: None,
        },
        surface_actions: None,
        surfaces: Some(&TEST_SURFACE_OPS),
        type_settings: None,
        config_test: None,
        sudo: None,
        raw_settings_keys: &[],
        global_provider_consumers: &[],
        migrations: None,
    };

    static TEST_LIFECYCLE_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
        type_id: TEST_LIFECYCLE_PLUGIN_TYPE_ID,
        display_name: "Test Lifecycle Recording",
        family: PluginFamily::Enhancement,
        config_model: ConfigModel::None,
        capabilities: &[PluginCapability::SoftwareItemLifecycle],
        config: ConfigOps {
            validate: noop_validate,
            mask_secrets: noop_mask,
            restore_secrets: noop_restore,
            sample: noop_sample,
            form_schema: noop_form_schema,
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
            software_item_lifecycle: Some(create_recording_lifecycle),
            controller_update_protection: None,
            infra: None,
        },
        surface_actions: None,
        surfaces: None,
        type_settings: None,
        config_test: None,
        sudo: None,
        raw_settings_keys: &[],
        migrations: None,
    };

    static TEST_CONTROLLER_PROTECTION_DESCRIPTOR_A: PluginDescriptor = PluginDescriptor {
        type_id: "__test_controller_protection_a",
        display_name: "Test Controller Protection A",
        family: PluginFamily::Enhancement,
        config_model: ConfigModel::None,
        capabilities: &[],
        config: ConfigOps {
            validate: noop_validate,
            mask_secrets: noop_mask,
            restore_secrets: noop_restore,
            sample: noop_sample,
            form_schema: noop_form_schema,
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
            controller_update_protection: Some(create_controller_update_protection_a),
            infra: None,
        },
        surface_actions: None,
        surfaces: None,
        type_settings: None,
        config_test: None,
        sudo: None,
        raw_settings_keys: &[],
        migrations: None,
    };

    static TEST_CONTROLLER_PROTECTION_DESCRIPTOR_B: PluginDescriptor = PluginDescriptor {
        type_id: "__test_controller_protection_b",
        display_name: "Test Controller Protection B",
        family: PluginFamily::Enhancement,
        config_model: ConfigModel::None,
        capabilities: &[],
        config: ConfigOps {
            validate: noop_validate,
            mask_secrets: noop_mask,
            restore_secrets: noop_restore,
            sample: noop_sample,
            form_schema: noop_form_schema,
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
            controller_update_protection: Some(create_controller_update_protection_b),
            infra: None,
        },
        surface_actions: None,
        surfaces: None,
        type_settings: None,
        config_test: None,
        sudo: None,
        raw_settings_keys: &[],
        global_provider_consumers: &[],
        migrations: None,
    };

    static TEST_MULTI_PROVIDER_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
        type_id: "__test_multi_provider_plugin",
        display_name: "Test Multi Provider Consumer",
        family: PluginFamily::Enhancement,
        config_model: ConfigModel::None,
        capabilities: &[],
        config: ConfigOps {
            validate: noop_validate,
            mask_secrets: noop_mask,
            restore_secrets: noop_restore,
            sample: noop_sample,
            form_schema: noop_form_schema,
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
            infra: None,
        },
        surface_actions: None,
        surfaces: None,
        type_settings: None,
        config_test: None,
        sudo: None,
        raw_settings_keys: &[],
        global_provider_consumers: &[
            GlobalProviderConsumerDecl::new("github"),
            GlobalProviderConsumerDecl::new("gitlab"),
        ],
        migrations: None,
    };

    /// Empty catalog builds successfully.
    #[test]
    fn empty_catalog() {
        let catalog = PluginCatalog::new(vec![], &CatalogConfig::default()).unwrap();
        assert!(catalog.all().is_empty());
        assert!(catalog.known_type_ids().is_empty());
    }

    #[test]
    fn catalog_config_defaults_without_global_provider_lookup() {
        let config = CatalogConfig::default();
        assert!(config.global_provider_lookup.is_none());
    }

    #[test]
    fn empty_catalog_has_no_surface_registrations() {
        let catalog = PluginCatalog::new(vec![], &CatalogConfig::default()).unwrap();
        assert!(catalog.surface_registrations().is_empty());
    }

    #[test]
    fn catalog_collects_descriptor_surface_registrations() {
        let catalog =
            PluginCatalog::new(vec![&TEST_DESCRIPTOR], &CatalogConfig::default()).unwrap();
        let registrations = catalog.surface_registrations();
        assert_eq!(registrations.len(), 1);
        assert_eq!(
            registrations[0].provider.provider_id,
            "plugin.test_provider"
        );
        assert_eq!(
            registrations[0].surfaces[0].descriptor.surface_id.as_str(),
            "plugin.test.surface"
        );
    }

    #[test]
    fn descriptor_declares_global_provider_consumers() {
        assert_eq!(
            DESCRIPTOR.global_provider_consumers,
            &[GlobalProviderConsumerDecl::new("github")]
        );
    }

    #[test]
    fn descriptor_without_global_provider_consumers_defaults_to_empty_slice() {
        assert!(
            TEST_LIFECYCLE_DESCRIPTOR
                .global_provider_consumers
                .is_empty()
        );
    }

    #[test]
    fn descriptor_preserves_global_provider_consumer_order() {
        assert_eq!(
            TEST_MULTI_PROVIDER_DESCRIPTOR.global_provider_consumers,
            &[
                GlobalProviderConsumerDecl::new("github"),
                GlobalProviderConsumerDecl::new("gitlab"),
            ]
        );
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
    fn rejects_duplicate_controller_update_protection_singleton() {
        let result = PluginCatalog::new(
            vec![
                &TEST_CONTROLLER_PROTECTION_DESCRIPTOR_A,
                &TEST_CONTROLLER_PROTECTION_DESCRIPTOR_B,
            ],
            &CatalogConfig::default(),
        );
        let err = result
            .err()
            .expect("duplicate singleton role must fail catalog construction");

        assert!(
            err.to_string()
                .contains("duplicate controller update protection"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn registers_and_exposes_controller_update_protection_singleton() {
        let catalog = PluginCatalog::new(
            vec![&TEST_CONTROLLER_PROTECTION_DESCRIPTOR_A],
            &CatalogConfig::default(),
        )
        .expect("catalog should build");

        let direct = catalog
            .controller_update_protection()
            .expect("singleton should be present");
        assert_eq!(
            direct.plugin_type_id(),
            PluginTypeId::from_static(TEST_CONTROLLER_PROTECTION_DESCRIPTOR_A.type_id)
        );

        let ops: &dyn PluginOps = &catalog;
        let via_plugin_ops = ops
            .controller_update_protection()
            .expect("plugin ops view should expose singleton");
        assert_eq!(
            via_plugin_ops.plugin_type_id(),
            PluginTypeId::from_static(TEST_CONTROLLER_PROTECTION_DESCRIPTOR_A.type_id)
        );
    }

    #[test]
    fn controller_protection_test_descriptors_use_minimal_capabilities() {
        assert!(
            TEST_CONTROLLER_PROTECTION_DESCRIPTOR_A
                .capabilities
                .is_empty(),
            "fixture should not claim unrelated capabilities"
        );
        assert!(
            TEST_CONTROLLER_PROTECTION_DESCRIPTOR_B
                .capabilities
                .is_empty(),
            "fixture should not claim unrelated capabilities"
        );
    }
}
