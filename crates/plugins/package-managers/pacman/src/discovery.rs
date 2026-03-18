use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, DiscoveryTarget, HostCompatibility, PluginRole, Result,
    execute_and_capture, plugin_ids,
};

use crate::config::PacmanDiscoveryFilter;
use crate::plugin::PacmanPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering Pacman-managed software");

        // Choose command based on effective filter.
        let args = match self.config.effective_filter() {
            PacmanDiscoveryFilter::Explicit => vec!["-Qe".to_string()],
            PacmanDiscoveryFilter::All => vec!["-Q".to_string()],
        };

        let stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("pacman", args),
            "pacman",
        )
        .await?;

        let all_packages = PacmanPlugin::parse_query_output(&stdout);

        let packages: Vec<DiscoveredSoftware> = all_packages
            .into_iter()
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_PACMAN.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "Pacman".to_string(),
                    roles: vec![
                        PluginRole::DetectVersion,
                        PluginRole::FetchReleases,
                        PluginRole::ExecuteUpdate,
                    ],
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }];
                DiscoveredSoftware {
                    package_identifier: name.clone(),
                    name,
                    installed_version: version,
                    targets,
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: false,
                    installed_display_version: None,
                }
            })
            .collect();

        tracing::debug!(count = packages.len(), "Pacman software discovery complete");
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["pacman".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "pacman not found".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
    use uptrakit_plugin_infrastructure_core::testing::RoutedOutputExecutor;
    use uptrakit_plugin_infrastructure_core::{
        Discoverer, HostCapabilities, HostRuntime, PluginRole, PosixHostRuntime, plugin_ids,
    };

    use crate::config::{PacmanConfig, PacmanDiscoveryFilter};
    use crate::plugin::PacmanPlugin;

    fn test_plugin_with_executor(
        config: PacmanConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> PacmanPlugin {
        let caps = HostCapabilities::default();
        let runtime = Arc::new(PosixHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        PacmanPlugin::new(config, runtime).unwrap()
    }

    // ── discover_software target emission ────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_targets() {
        // Targets are always emitted regardless of filter.
        let executor = RoutedOutputExecutor::success([("pacman", "nginx 1.26.3-1\n")]);
        let plugin = test_plugin_with_executor(PacmanConfig::default(), executor);

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].targets.len(), 1);

        let target = &discoveries[0].targets[0];
        assert_eq!(
            target.plugin_type,
            plugin_ids::PACKAGE_MANAGER_PACMAN.clone()
        );
        assert_eq!(target.plugin_config_name, "Pacman");
        assert_eq!(target.plugin_config, serde_json::json!({}));
        assert!(target.roles.contains(&PluginRole::DetectVersion));
        assert!(target.roles.contains(&PluginRole::FetchReleases));
        assert!(target.roles.contains(&PluginRole::ExecuteUpdate));
    }

    #[tokio::test]
    async fn discover_software_default_config_discovers_all_packages() {
        let executor =
            RoutedOutputExecutor::success([("pacman", "nginx 1.26.3-1\npython 3.12.4-1\n")]);
        let plugin = test_plugin_with_executor(PacmanConfig::default(), executor);

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 2, "all packages must be discovered");
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_explicit_all_filter() {
        let executor = RoutedOutputExecutor::success([("pacman", "nginx 1.26.3-1\n")]);
        let plugin = test_plugin_with_executor(
            PacmanConfig {
                discovery_filter: PacmanDiscoveryFilter::All,
            },
            executor,
        );

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(
            discoveries[0].targets.len(),
            1,
            "explicit All filter must still emit targets"
        );
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_explicit_filter() {
        let executor = RoutedOutputExecutor::success([("pacman", "nginx 1.26.3-1\n")]);
        let plugin = test_plugin_with_executor(
            PacmanConfig {
                discovery_filter: PacmanDiscoveryFilter::Explicit,
            },
            executor,
        );

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].package_identifier, "nginx");
        assert_eq!(
            discoveries[0].targets.len(),
            1,
            "explicit filter must still emit targets"
        );
    }
}
