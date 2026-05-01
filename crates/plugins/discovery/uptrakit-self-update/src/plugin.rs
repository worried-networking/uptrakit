use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, DiscoveredSoftware, HostCompatibility, HostRequirements,
    HostRuntime, PluginFamily, declare_plugin,
};

use crate::config::UptrakitSelfUpdateConfig;

/// Discovery plugin that discovers the running uptrakit service as a managed
/// software item.
///
/// When `config.enabled` is `false` (the default), `detect_host_compatibility`
/// returns `Incompatible` immediately — no I/O is performed. This allows
/// controller-standalone to ship with the plugin registered but inert unless
/// the operator explicitly opts in by setting `enabled = true`.
pub struct UptrakitSelfUpdatePlugin {
    config: UptrakitSelfUpdateConfig,
    _runtime: Arc<dyn HostRuntime>,
}

impl UptrakitSelfUpdatePlugin {
    /// Create a new instance.
    ///
    /// Construction is infallible — no I/O is performed here.
    pub fn new(
        config: UptrakitSelfUpdateConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        Ok(Self {
            config,
            _runtime: runtime,
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

// ── Discoverer implementation ────────────────────────────────────────────

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for UptrakitSelfUpdatePlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<HostCompatibility> {
        if !self.config.enabled {
            return Ok(HostCompatibility::Incompatible(
                "uptrakit self-update is disabled — set `enabled = true` to opt in".to_string(),
            ));
        }

        // When enabled, the plugin is always compatible — it runs on the controller
        // itself, not on a managed host.
        Ok(HostCompatibility::Compatible)
    }

    #[tracing::instrument(skip_all)]
    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<DiscoveredSoftware>> {
        // Full implementation comes in Task 17.
        // When disabled this path is unreachable (detect_host_compatibility returns
        // Incompatible), but we return empty rather than panicking so the stub is
        // safe to call in tests.
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{
        Discoverer, HostCapabilities, LocalCommandExecutor, PluginCapability, PluginMeta,
        StandardHostRuntime, command::CommandExecutor,
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
        matches!(result.unwrap(), HostCompatibility::Incompatible(_));
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

    #[tokio::test]
    async fn detect_host_compatibility_enabled_returns_compatible() {
        let config = UptrakitSelfUpdateConfig { enabled: true };
        let plugin = test_plugin_with_config(config);
        let result = plugin.detect_host_compatibility().await;
        assert!(result.is_ok());
        assert!(
            matches!(result.unwrap(), HostCompatibility::Compatible),
            "enabled plugin must report Compatible"
        );
    }

    // ── discovery ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_returns_empty_stub() {
        let plugin = test_plugin();
        let result = plugin.discover_software().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
