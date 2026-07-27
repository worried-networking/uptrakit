use std::sync::Arc;

use rootcause::prelude::*;
use uptrakit_global_github_provider::GitHubProviderClient;
#[cfg(feature = "catalog")]
use uptrakit_global_github_provider::GitHubProviderHandle;
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, HostRequirements, HostRuntime, PluginCapability, PluginConfigValidationError,
    PluginError, PluginFamily, ReleaseFetcher, Result, command::CommandSpec, declare_plugin,
    roles::ReleaseFetchContext,
};

use crate::config::SkillsConfig;

/// Shell script: exit 44 = lock file absent; exit 0 = content on stdout; other = unreadable.
pub(crate) const READ_LOCK_SCRIPT: &str =
    r#"f="$HOME/.agents/.skill-lock.json"; if [ ! -f "$f" ]; then exit 44; fi; cat -- "$f""#;

/// Sentinel exit code meaning the skill lock file does not exist.
pub(crate) const LOCK_ABSENT_EXIT: i32 = 44;

/// Agent Skills package-manager plugin.
///
/// Manages LLM-agent skills installed via the `skills` CLI (`npx skills@<version>`).
/// Each skill is identified by a `<source_url>#<skill_path>` composite key parsed
/// by [`crate::lock::parse_skill_identifier`].
///
/// Controller-side roles: `ReleaseFetcher` (latest commit-date display via GitHub Trees +
/// commits-by-path) and `InstalledVersionEnricher` (installed commit-date display via the
/// same primitive — keyed by tree-at-path SHA reported by the agent).
#[non_exhaustive]
pub struct SkillsPlugin {
    pub(crate) config: SkillsConfig,
    pub(crate) executor: Arc<dyn uptrakit_plugin_infrastructure_core::CommandExecutor>,
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

    /// Read the skill lock file from the agent host.
    ///
    /// Returns `Ok(None)` when the lock file is absent (sentinel exit `44`),
    /// `Ok(Some(content))` when it was read successfully, or `Err` when the
    /// file exists but could not be read (permission denied, executor failure,
    /// etc.).
    ///
    /// # Errors
    ///
    /// Returns `Err` for any failure other than the file-absent sentinel.
    pub(crate) async fn read_lock_file(&self) -> Result<Option<String>> {
        let cmd = CommandSpec::exec("sh", ["-c".to_string(), READ_LOCK_SCRIPT.to_string()]);
        match self.executor.execute_quiet(&cmd).await {
            Ok(out) if out.exit_code == 0 => Ok(Some(out.output)),
            Ok(out) if out.exit_code == LOCK_ABSENT_EXIT => {
                tracing::debug!("skill lock file absent (sentinel exit {LOCK_ABSENT_EXIT})");
                Ok(None)
            }
            Ok(out) => {
                bail!(PluginError::PluginInternal(format!(
                    "skill lock file exists but is unreadable (exit {})",
                    out.exit_code
                )))
            }
            Err(e) => {
                if let uptrakit_command::CommandError::CommandFailed(c) = e.current_context()
                    && *c == LOCK_ABSENT_EXIT
                {
                    tracing::debug!("skill lock file absent (sentinel exit {LOCK_ABSENT_EXIT})");
                    return Ok(None);
                }
                bail!(PluginError::PluginInternal(format!(
                    "failed to read skill lock file: {e}"
                )))
            }
        }
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
        Arc::downcast::<GitHubProviderHandle>(handle)
            .ok()
            .map(|h| h.client())
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

/// Custom `InstalledVersionEnricher` factory that injects the GitHub provider from context.
///
/// Used as `installed_version_enricher_create` in `declare_plugin!` so the controller-side
/// enricher can reach the global GitHub provider through
/// [`uptrakit_plugin_infrastructure_core::InstalledVersionEnrichmentContext`].
pub(crate) fn create_installed_version_enricher_skills(
    config: &serde_json::Value,
    runtime: Arc<dyn HostRuntime>,
    ctx: &uptrakit_plugin_infrastructure_core::InstalledVersionEnrichmentContext,
) -> uptrakit_plugin_infrastructure_core::error::Result<
    Box<dyn uptrakit_plugin_infrastructure_core::InstalledVersionEnricher>,
> {
    let cfg: SkillsConfig = serde_json::from_value(config.clone()).map_err(|e| {
        report!(PluginError::Configuration(format!(
            "failed to parse skills config: {e}"
        )))
    })?;
    let provider = lookup_github_provider_from_enrichment_ctx(ctx);
    Ok(Box::new(SkillsPlugin {
        config: cfg,
        executor: runtime.executor(),
        provider,
    }))
}

/// Extract the GitHub provider client from the installed-version-enrichment context.
///
/// Returns `None` in standalone-scheduler deployments or when the `catalog` feature is
/// not active. Uses only a positive `#[cfg(feature = "catalog")]` block; the implicit
/// "else" is the trailing `None` literal, satisfying the additive-only feature-flag rule.
fn lookup_github_provider_from_enrichment_ctx(
    ctx: &uptrakit_plugin_infrastructure_core::InstalledVersionEnrichmentContext,
) -> Option<Arc<dyn GitHubProviderClient>> {
    let _ = ctx;
    #[cfg(feature = "catalog")]
    {
        if let Some(lookup) = ctx.global_provider_lookup.as_ref()
            && let Some(handle) = lookup.lookup("github")
        {
            return Arc::downcast::<GitHubProviderHandle>(handle)
                .ok()
                .map(|h| h.client());
        }
    }
    None
}

// `ReleaseFetcher` listed in `roles` ensures `PluginCapability::ReleaseFetching` is included
// and `SkillsPlugin: ReleaseFetcher` is compile-checked. The auto-generated factory is
// replaced by `create_release_fetcher_skills`, which injects the GitHub provider.
// `InstalledVersionEnricher` is handled the same way via `create_installed_version_enricher_skills`.
declare_plugin!(SkillsPlugin, SkillsConfig, "package-manager.skills", {
    display_name: "Agent Skills",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    roles: [
        Discoverer,
        VersionDetector,
        ReleaseFetcher { host_requirements: HostRequirements::CONTROLLER_ONLY },
        InstalledVersionEnricher { host_requirements: HostRequirements::CONTROLLER_ONLY },
        UpdateExecutor,
    ],
    extra_capabilities: [
        PluginCapability::ControllerSideFetchReleases,
        PluginCapability::EnrichInstalledVersion,
    ],
    release_fetcher_create: {
        create: create_release_fetcher_skills,
        host_requirements: HostRequirements::CONTROLLER_ONLY,
    },
    installed_version_enricher_create: {
        create: create_installed_version_enricher_skills,
        host_requirements: HostRequirements::CONTROLLER_ONLY,
    },
});

// ── Role implementations ──────────────────────────────────────────────────────
// Discoverer is implemented in discovery.rs (Task 6).
// VersionDetector is implemented in detection.rs (Task 7).
// ReleaseFetcher is implemented in releases.rs (Task 8).
// UpdateExecutor is implemented in update.rs (Task 9).

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::testing::test_runtime;
    use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginMeta};

    #[test]
    fn descriptor_type_id() {
        let plugin = SkillsPlugin::new(SkillsConfig::default(), test_runtime()).expect("create");
        assert_eq!(plugin.plugin_type_id().as_str(), "package-manager.skills");
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
                .contains(&PluginCapability::ControllerSideFetchReleases)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::EnrichInstalledVersion)
        );
        assert!(
            DESCRIPTOR.roles.installed_version_enricher.is_some(),
            "Skills must register an InstalledVersionEnricher slot"
        );
        assert!(
            DESCRIPTOR
                .roles
                .installed_version_enricher
                .as_ref()
                .unwrap()
                .host_requirements
                .controller_only
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
