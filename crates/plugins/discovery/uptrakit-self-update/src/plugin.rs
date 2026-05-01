use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, HostRequirements, HostRuntime, PluginFamily,
    ServiceMetadataProvider, declare_plugin,
};

use crate::config::UptrakitSelfUpdateConfig;

/// Discovery plugin that discovers the running uptrakit service as a managed
/// software item.
///
/// When `config.enabled` is `false` (the default), `detect_host_compatibility`
/// returns `Incompatible` immediately — no I/O is performed. This allows
/// controller-standalone to ship with the plugin registered but inert unless
/// the operator explicitly opts in by setting `enabled = true`.
///
/// When `enabled` is `true` but the plugin is not constructed with a
/// `metadata_provider` (i.e., not running as the embedded agent inside a
/// controller-standalone), `detect_host_compatibility` returns `Incompatible`
/// with a reason explaining that a controller is required.
pub struct UptrakitSelfUpdatePlugin {
    pub(crate) config: UptrakitSelfUpdateConfig,
    pub(crate) metadata_provider: Option<Arc<dyn ServiceMetadataProvider>>,
}

impl UptrakitSelfUpdatePlugin {
    /// Create a new instance.
    ///
    /// Construction never fails — no I/O is performed here. The `Result` return type
    /// satisfies the `declare_plugin!` macro contract; the `Err` variant is never produced.
    pub fn new(
        config: UptrakitSelfUpdateConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let metadata_provider = runtime.metadata_provider();
        Ok(Self {
            config,
            metadata_provider,
        })
    }
}

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(UptrakitSelfUpdatePlugin, UptrakitSelfUpdateConfig, "discovery_uptrakit_self_update", {
    display_name: "Uptrakit Self-Update",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::CONTROLLER_ONLY,
    config_test: [ConfigTestKind::VersionDetection],
    roles: [Discoverer],
});

// ── Discoverer implementation is in discovery.rs ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uptrakit_plugin_infrastructure_core::{
        DeploymentTopology, Discoverer, HostCapabilities, HostCompatibility, LocalCommandExecutor,
        PluginCapability, PluginMeta, ServiceMetadata, StandardHostRuntime,
        command::CommandExecutor,
    };

    fn test_plugin_with_config(config: UptrakitSelfUpdateConfig) -> UptrakitSelfUpdatePlugin {
        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        UptrakitSelfUpdatePlugin::new(config, runtime).expect("create plugin")
    }

    fn test_plugin() -> UptrakitSelfUpdatePlugin {
        test_plugin_with_config(UptrakitSelfUpdateConfig::default())
    }

    /// A fake `ServiceMetadataProvider` for unit tests.
    struct FakeMetadataProvider {
        service_name: String,
        binary_path: Option<PathBuf>,
        version: String,
        reuseport_configured: bool,
        pid_file: Option<PathBuf>,
    }

    impl uptrakit_plugin_infrastructure_core::ServiceMetadataProvider for FakeMetadataProvider {
        fn get_metadata(&self) -> ServiceMetadata {
            ServiceMetadata::new(
                self.service_name.clone(),
                self.binary_path.clone(),
                self.version.clone(),
                DeploymentTopology::UnixBinary,
                self.reuseport_configured,
                self.pid_file.clone(),
            )
        }
    }

    fn make_test_metadata_provider(version: &str) -> Arc<dyn ServiceMetadataProvider> {
        Arc::new(FakeMetadataProvider {
            service_name: "uptrakit-controller".to_string(),
            binary_path: Some(PathBuf::from("/usr/bin/uptrakit")),
            version: version.to_string(),
            reuseport_configured: false,
            pid_file: None,
        })
    }

    fn test_plugin_with_provider(
        config: UptrakitSelfUpdateConfig,
        provider: Arc<dyn ServiceMetadataProvider>,
    ) -> UptrakitSelfUpdatePlugin {
        UptrakitSelfUpdatePlugin {
            config,
            metadata_provider: Some(provider),
        }
    }

    // ── config ──────────────────────────────────────────────────────────

    #[test]
    fn test_detect_host_compatibility_disabled_by_default() {
        let config = super::super::config::UptrakitSelfUpdateConfig::default();
        assert!(!config.enabled);
    }

    // ── plugin type id ──────────────────────────────────────────────────

    #[test]
    fn plugin_type_id_is_discovery_uptrakit_self_update() {
        let plugin = test_plugin();
        assert_eq!(
            plugin.plugin_type_id().as_str(),
            "discovery_uptrakit_self_update"
        );
    }

    // ── descriptor capabilities ─────────────────────────────────────────

    #[test]
    fn descriptor_capabilities() {
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::DiscoverLocalSoftware)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::DetectHostCompatibility)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ConfigTest)
        );
    }

    // ── descriptor roles ────────────────────────────────────────────────

    #[test]
    fn descriptor_has_discoverer_role_only() {
        assert!(DESCRIPTOR.roles.discoverer.is_some());
        assert!(DESCRIPTOR.roles.version_detector.is_none());
        assert!(DESCRIPTOR.roles.release_fetcher.is_none());
        assert!(DESCRIPTOR.roles.package_indexer.is_none());
        assert!(DESCRIPTOR.roles.update_executor.is_none());
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_none());
    }

    // ── descriptor sudo ─────────────────────────────────────────────────

    #[test]
    fn descriptor_has_no_sudo() {
        assert!(DESCRIPTOR.sudo.is_none());
    }

    // ── host compatibility ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_disabled_returns_incompatible() {
        let plugin = test_plugin(); // default: enabled = false
        let result = plugin.detect_host_compatibility().await;
        assert!(result.is_ok(), "must not error");
        assert!(matches!(
            result.unwrap(),
            HostCompatibility::Incompatible(_)
        ));
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_carries_hint() {
        let plugin = test_plugin();
        if let Ok(HostCompatibility::Incompatible(reason)) =
            plugin.detect_host_compatibility().await
        {
            assert!(
                reason.contains("enabled"),
                "incompatible reason should mention 'enabled': {reason}"
            );
        }
    }

    /// When `enabled = true` but `metadata_provider` is `None` (i.e., not
    /// running as embedded agent inside controller-standalone), the plugin
    /// must return `Incompatible`.
    #[tokio::test]
    async fn detect_host_compatibility_enabled_no_provider_returns_incompatible() {
        let config = UptrakitSelfUpdateConfig { enabled: true };
        let plugin = test_plugin_with_config(config); // StandardHostRuntime → None provider
        let result = plugin.detect_host_compatibility().await;
        assert!(result.is_ok());
        assert!(
            matches!(result.unwrap(), HostCompatibility::Incompatible(_)),
            "enabled plugin without metadata provider must report Incompatible"
        );
    }

    /// When `enabled = true` AND `metadata_provider` is set, the plugin must
    /// report `Compatible`.
    #[tokio::test]
    async fn detect_host_compatibility_enabled_with_provider_returns_compatible() {
        let config = UptrakitSelfUpdateConfig { enabled: true };
        let provider = make_test_metadata_provider("1.0.0");
        let plugin = test_plugin_with_provider(config, provider);
        let result = plugin.detect_host_compatibility().await;
        assert!(result.is_ok());
        assert!(
            matches!(result.unwrap(), HostCompatibility::Compatible),
            "enabled plugin with metadata provider must report Compatible"
        );
    }

    // ── discovery ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_disabled_returns_empty() {
        let plugin = test_plugin(); // enabled = false
        let result = plugin.discover_software().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn discover_software_enabled_no_provider_returns_empty() {
        let config = UptrakitSelfUpdateConfig { enabled: true };
        let plugin = test_plugin_with_config(config); // no metadata_provider
        let result = plugin.discover_software().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    // ── new discovery tests (Task 17) ────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_disabled() {
        let config = UptrakitSelfUpdateConfig { enabled: false };
        let provider = make_test_metadata_provider("1.0.0");
        let plugin = test_plugin_with_provider(config, provider);
        let result = plugin.detect_host_compatibility().await;
        assert!(result.is_ok());
        assert!(
            matches!(result.unwrap(), HostCompatibility::Incompatible(_)),
            "disabled plugin must return Incompatible"
        );
    }

    #[tokio::test]
    async fn detect_host_compatibility_no_metadata_provider() {
        let config = UptrakitSelfUpdateConfig { enabled: true };
        let plugin = UptrakitSelfUpdatePlugin {
            config,
            metadata_provider: None,
        };
        let result = plugin.detect_host_compatibility().await;
        assert!(result.is_ok());
        if let Ok(HostCompatibility::Incompatible(reason)) = result {
            assert!(
                reason.contains("controller") || reason.contains("embedded"),
                "reason should mention 'controller' or 'embedded': {reason}"
            );
        } else {
            panic!("expected Incompatible when no metadata_provider");
        }
    }

    #[tokio::test]
    async fn build_software_item_sets_tag_strip_prefix() {
        let config = UptrakitSelfUpdateConfig { enabled: true };
        let provider = make_test_metadata_provider("1.2.3");
        let plugin = test_plugin_with_provider(config, provider);
        let result = plugin.discover_software().await.expect("discover_software");
        assert_eq!(
            result.len(),
            1,
            "expected exactly one discovered software item"
        );
        let item = &result[0];
        let github_target = item
            .targets
            .iter()
            .find(|t| t.plugin_type.as_str() == "releases_github");
        let github_target = github_target.expect("releases_github target must be present");
        let strip_prefix = github_target
            .plugin_config
            .get("tag_strip_prefix")
            .and_then(|v| v.as_str());
        assert_eq!(strip_prefix, Some("v"), "tag_strip_prefix must be 'v'");
    }

    #[tokio::test]
    async fn build_software_item_awaiting_restart_timeout_is_120() {
        let config = UptrakitSelfUpdateConfig { enabled: true };
        let provider = make_test_metadata_provider("1.2.3");
        let plugin = test_plugin_with_provider(config, provider);
        let result = plugin.discover_software().await.expect("discover_software");
        assert_eq!(
            result.len(),
            1,
            "expected exactly one discovered software item"
        );
        let item = &result[0];
        let timeout = item
            .extra
            .as_ref()
            .and_then(|e| e.get("awaiting_restart_timeout"))
            .and_then(|v| v.as_u64());
        assert_eq!(timeout, Some(120), "awaiting_restart_timeout must be 120");
    }

    /// Integration smoke test: the plugin, constructed via the real registry path
    /// (MetadataAwareHostRuntime + ControllerMetadataProvider), discovers exactly
    /// one software item representing the running test binary.
    ///
    /// This test is `#[ignore]` because it reads `std::env::current_exe()` (live
    /// filesystem) and is intended for explicit CI verification rather than the
    /// fast unit-test loop.
    #[tokio::test]
    #[ignore]
    async fn test_self_update_plugin_discovers_running_controller() {
        use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
        use uptrakit_plugin_infrastructure_core::service_metadata::{
            DeploymentTopology, ServiceMetadata,
        };
        use uptrakit_plugin_infrastructure_core::{HostCapabilities, LocalCommandExecutor};
        use uptrakit_plugin_infrastructure_core::{
            MetadataAwareHostRuntime, ServiceMetadataProvider, construct_host_runtime,
        };

        struct TestMetadataProvider {
            version: String,
        }
        impl ServiceMetadataProvider for TestMetadataProvider {
            fn get_metadata(&self) -> ServiceMetadata {
                ServiceMetadata::new(
                    "uptrakit-controller-standalone".to_string(),
                    std::env::current_exe().ok(),
                    self.version.clone(),
                    DeploymentTopology::UnixBinary,
                    false,
                    None,
                )
            }
        }

        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let base_runtime = construct_host_runtime(executor, HostCapabilities::default());
        let provider = Arc::new(TestMetadataProvider {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }) as Arc<dyn ServiceMetadataProvider>;
        let runtime = MetadataAwareHostRuntime::new(base_runtime, provider);

        let config = UptrakitSelfUpdateConfig { enabled: true };
        let plugin =
            UptrakitSelfUpdatePlugin::new(config, runtime).expect("plugin creation must not fail");

        let compat = plugin
            .detect_host_compatibility()
            .await
            .expect("detect_host_compatibility must not error");
        assert!(
            matches!(compat, HostCompatibility::Compatible),
            "plugin must be Compatible when MetadataAwareHostRuntime is used: {compat:?}"
        );

        let items = plugin
            .discover_software()
            .await
            .expect("discover_software must not error");
        assert_eq!(items.len(), 1, "must discover exactly one software item");

        let item = &items[0];
        assert_eq!(
            item.name, "uptrakit-controller-standalone",
            "software item name must match service name"
        );
        assert!(
            !item.installed_version.is_empty(),
            "discovered item must carry installed version"
        );
    }
}
