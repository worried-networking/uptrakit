use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_global_github_provider::GitHubProviderClient;
#[cfg(feature = "catalog")]
use uptrakit_global_github_provider::GitHubProviderHandle;
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, DiscoveredSoftware, ExecuteUpdateResult, HostCompatibility, HostRequirements,
    HostRuntime, PluginConfigValidationError, PluginError, PluginFamily, ReleaseFetcher,
    ReleaseInfo, Result, UpdateOutputSender, UpstreamRelease, Version, VersionDetector,
    declare_plugin, roles::ReleaseFetchContext,
};

use crate::config::SkillsConfig;

/// Agent Skills package-manager plugin.
///
/// Manages LLM-agent skills installed via the `skills` CLI (`npx skills@<version>`).
/// Each skill is identified by a `<source_url>#<skill_path>` composite key parsed
/// by [`crate::lock::parse_skill_identifier`].
#[non_exhaustive]
pub struct SkillsPlugin {
    #[expect(dead_code, reason = "read by executor modules landing in Tasks 6–9")]
    pub(crate) config: SkillsConfig,
    #[expect(dead_code, reason = "read by executor modules landing in Tasks 6–9")]
    pub(crate) executor: Arc<dyn uptrakit_plugin_infrastructure_core::CommandExecutor>,
    #[expect(dead_code, reason = "read by release-fetch module landing in Task 8")]
    pub(crate) provider: Option<Arc<dyn GitHubProviderClient>>,
}

impl SkillsPlugin {
    /// Create a new skills plugin with the given configuration and host runtime.
    pub fn new(config: SkillsConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self> {
        Ok(Self {
            config,
            executor: runtime.executor(),
            provider: None,
        })
    }
}

/// Validate a skill identifier (`<source_url>#<skill_path>`).
///
/// Delegates to [`crate::lock::parse_skill_identifier`] and maps parse errors to
/// [`PluginConfigValidationError::InvalidIdentifier`].
pub fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
    crate::lock::parse_skill_identifier(value)
        .map(|_| ())
        .map_err(|e| PluginConfigValidationError::InvalidIdentifier(e.to_string()))
}

/// Extract the GitHub provider client from the release-fetch context, if available.
///
/// Returns `None` in standalone-scheduler deployments or when the `catalog`
/// feature is not active (the `global_provider_lookup` field is gated on that feature).
fn lookup_github_provider_from_ctx(
    ctx: &ReleaseFetchContext,
) -> Option<Arc<dyn GitHubProviderClient>> {
    #[cfg(feature = "catalog")]
    {
        let lookup = ctx.global_provider_lookup.as_ref()?;
        let handle = lookup.lookup("github")?;
        return Arc::downcast::<GitHubProviderHandle>(handle)
            .ok()
            .map(|h| h.client());
    }
    #[cfg(not(feature = "catalog"))]
    {
        let _ = ctx;
        None
    }
}

/// Custom `ReleaseFetcher` factory that injects the GitHub provider from context.
///
/// Used as `release_fetcher_create` in `declare_plugin!` so the controller-side
/// release fetcher can reach the global GitHub provider through
/// [`ReleaseFetchContext`].
pub(crate) fn create_release_fetcher_skills(
    config: &serde_json::Value,
    runtime: Arc<dyn HostRuntime>,
    ctx: &ReleaseFetchContext,
) -> uptrakit_plugin_infrastructure_core::error::Result<Box<dyn ReleaseFetcher>> {
    let cfg: SkillsConfig = serde_json::from_value(config.clone()).map_err(|e| {
        report!(PluginError::Configuration(format!(
            "failed to parse skills config: {e}"
        )))
    })?;
    let provider = lookup_github_provider_from_ctx(ctx);
    Ok(Box::new(SkillsPlugin {
        config: cfg,
        executor: runtime.executor(),
        provider,
    }))
}

// `ReleaseFetcher` listed in `roles` ensures `PluginCapability::ReleaseFetching` is included
// and `SkillsPlugin: ReleaseFetcher` is compile-checked. The auto-generated factory is
// replaced by `create_release_fetcher_skills`, which injects the GitHub provider.
declare_plugin!(SkillsPlugin, SkillsConfig, "package_manager_skills", {
    display_name: "Agent Skills",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    roles: [
        Discoverer,
        VersionDetector,
        ReleaseFetcher { host_requirements: HostRequirements::CONTROLLER_ONLY },
        UpdateExecutor,
    ],
    release_fetcher_create: {
        create: create_release_fetcher_skills,
        host_requirements: HostRequirements::CONTROLLER_ONLY,
    },
    global_provider_consumers: ["github"],
});

// ── Temporary role stubs ─────────────────────────────────────────────────────
// Each stub will be replaced by a real implementation in Tasks 6–9.

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for SkillsPlugin {
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        Ok(vec![])
    }

    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        Ok(HostCompatibility::Compatible)
    }
}

#[async_trait]
impl VersionDetector for SkillsPlugin {
    async fn detect_installed_version(&self, _id: &str) -> Result<Option<Version>> {
        Ok(None)
    }
}

#[async_trait]
impl ReleaseFetcher for SkillsPlugin {
    async fn fetch_releases(&self, _id: &str) -> Result<Vec<UpstreamRelease>> {
        Ok(vec![])
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for SkillsPlugin {
    async fn execute_update(
        &self,
        _id: &str,
        _ver: &str,
        _release_info: Option<&ReleaseInfo>,
        _tx: &UpdateOutputSender,
    ) -> Result<ExecuteUpdateResult> {
        Ok(ExecuteUpdateResult::new(String::new(), false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::testing::test_runtime;
    use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginMeta};

    #[test]
    fn descriptor_type_id() {
        let plugin = SkillsPlugin::new(SkillsConfig::default(), test_runtime()).expect("create");
        assert_eq!(plugin.plugin_type_id().as_str(), "package_manager_skills");
    }

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
                .contains(&PluginCapability::VersionDetection)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ReleaseFetching)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::UpdateExecution)
        );
        assert!(
            !DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::RefreshPackageIndex)
        );
        assert!(
            !DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ConfigTest)
        );
    }

    #[test]
    fn descriptor_release_fetcher_is_controller_only() {
        let slot = DESCRIPTOR
            .roles
            .release_fetcher
            .as_ref()
            .expect("slot present");
        assert!(slot.host_requirements.controller_only);
    }

    #[test]
    fn descriptor_has_expected_roles() {
        assert!(DESCRIPTOR.roles.discoverer.is_some());
        assert!(DESCRIPTOR.roles.version_detector.is_some());
        assert!(DESCRIPTOR.roles.release_fetcher.is_some());
        assert!(DESCRIPTOR.roles.update_executor.is_some());
        assert!(DESCRIPTOR.roles.package_indexer.is_none());
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_none());
    }
}
