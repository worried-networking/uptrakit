use std::collections::HashSet;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, Discoverer, DiscoveryTarget, HostCompatibility, PluginRole, Result,
    execute_and_capture, plugin_ids,
};

use crate::config::AptDiscoveryFilter;
use crate::plugin::AptPlugin;

/// Parse `dpkg-query --show --showformat=${Package}\t${Version}\n` output.
///
/// Each line is a tab-separated `package\tversion` pair. Lines with an
/// empty version are skipped.
pub(crate) fn parse_dpkg_output(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let name = parts.next()?.trim();
            let version = parts.next()?.trim();
            if name.is_empty() || version.is_empty() {
                None
            } else {
                Some((name.to_string(), version.to_string()))
            }
        })
        .collect()
}

#[async_trait]
impl Discoverer for AptPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering APT-managed software");

        // Step 1: Query all installed packages from dpkg.
        let dpkg_stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec(
                "dpkg-query",
                [
                    "--show".to_string(),
                    "--showformat=${Package}\\t${Version}\\n".to_string(),
                ],
            ),
            "dpkg-query",
        )
        .await?;

        let all_packages = parse_dpkg_output(&dpkg_stdout);

        // Step 2: For the Manual filter, build a set of manually-installed packages.
        let manual_set: Option<HashSet<String>> = match self.config.effective_filter() {
            AptDiscoveryFilter::Manual => {
                let mark_stdout = execute_and_capture(
                    self.executor.as_ref(),
                    CommandSpec::exec("apt-mark", ["showmanual".to_string()]),
                    "apt-mark showmanual",
                )
                .await?;

                let set: HashSet<String> = mark_stdout
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                Some(set)
            }
            AptDiscoveryFilter::All => None,
        };

        // Step 3: Filter by the manual set (if applicable) and build results.
        let packages: Vec<DiscoveredSoftware> = all_packages
            .into_iter()
            .filter(|(name, _)| {
                manual_set
                    .as_ref()
                    .is_none_or(|set| set.contains(name.as_str()))
            })
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_APT.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "APT".to_string(),
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

        tracing::debug!(count = packages.len(), "APT software discovery complete");
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["apt-get".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "apt-get not found".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
    use uptrakit_plugin_infrastructure_core::testing::{FixedOutputExecutor, RoutedOutputExecutor};
    use uptrakit_plugin_infrastructure_core::{
        Discoverer, HostCapabilities, HostCompatibility, HostRuntime, LocalCommandExecutor,
        PluginRole, ReleaseFetcher, StandardHostRuntime, VersionDetector, plugin_ids,
    };

    use crate::config::{AptConfig, AptDiscoveryFilter};

    /// Helper to create an `AptPlugin` from a mock executor for testing.
    fn test_plugin_with_executor(
        config: AptConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> AptPlugin {
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        AptPlugin::new(config, runtime).unwrap()
    }

    // ── parse_dpkg_output ───────────────────────────────────────────────

    #[test]
    fn parse_dpkg_output_normal() {
        let output = "nginx\t1.24.0-2ubuntu7.3\npython3\t3.11.0-5ubuntu2\n";
        let result = parse_dpkg_output(output);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            ("nginx".to_string(), "1.24.0-2ubuntu7.3".to_string())
        );
        assert_eq!(
            result[1],
            ("python3".to_string(), "3.11.0-5ubuntu2".to_string())
        );
    }

    #[test]
    fn parse_dpkg_output_empty_version_skipped() {
        let output = "nginx\t\npython3\t3.11.0\n";
        let result = parse_dpkg_output(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "python3");
        assert_eq!(result[0].1, "3.11.0");
    }

    #[test]
    fn parse_dpkg_output_empty_input() {
        let result = parse_dpkg_output("");
        assert!(result.is_empty());
    }

    // ── discover_software target emission ─────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_targets() {
        // Targets are always emitted regardless of filter.
        let executor = RoutedOutputExecutor::success([("dpkg-query", "nginx\t1.24.0\n")]);
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].targets.len(), 1);

        let target = &discoveries[0].targets[0];
        assert_eq!(target.plugin_type, plugin_ids::PACKAGE_MANAGER_APT.clone());
        assert_eq!(target.plugin_config_name, "APT");
        assert_eq!(target.plugin_config, serde_json::json!({}));
        assert!(target.roles.contains(&PluginRole::DetectVersion));
        assert!(target.roles.contains(&PluginRole::FetchReleases));
        assert!(target.roles.contains(&PluginRole::ExecuteUpdate));
    }

    #[tokio::test]
    async fn discover_software_default_config_discovers_all_packages() {
        // Default config -> effective filter All -> all dpkg packages discovered.
        let executor =
            RoutedOutputExecutor::success([("dpkg-query", "nginx\t1.24.0\npython3\t3.11.0\n")]);
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 2, "all dpkg packages must be discovered");
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_explicit_all_filter() {
        // discovery_filter: All -> targets always emitted.
        let executor = RoutedOutputExecutor::success([("dpkg-query", "nginx\t1.24.0\n")]);
        let plugin = test_plugin_with_executor(
            AptConfig {
                discovery_filter: AptDiscoveryFilter::All,
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
    async fn discover_software_emits_targets_with_manual_filter() {
        // discovery_filter: Manual -> apt-mark narrows packages; targets always emitted.
        let executor = RoutedOutputExecutor::success([
            ("dpkg-query", "nginx\t1.24.0\npython3\t3.11.0\n"),
            ("apt-mark", "nginx\n"), // only nginx is manually installed
        ]);
        let plugin = test_plugin_with_executor(
            AptConfig {
                discovery_filter: AptDiscoveryFilter::Manual,
            },
            executor,
        );

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].package_identifier, "nginx");
        assert_eq!(
            discoveries[0].targets.len(),
            1,
            "manual filter must still emit targets"
        );
    }

    // ── detect_host_compatibility ────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_which_exits_zero() {
        let plugin =
            test_plugin_with_executor(AptConfig::default(), FixedOutputExecutor::failure(0));
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_which_exits_nonzero() {
        let plugin =
            test_plugin_with_executor(AptConfig::default(), FixedOutputExecutor::failure(1));
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert_eq!(msg, "apt-get not found");
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }

    // ── empty identifier guards ──────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_empty_identifier_fails() {
        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);
        let result = plugin.detect_installed_version("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_releases_empty_identifier_fails() {
        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);
        let result = plugin.fetch_releases("").await;
        assert!(result.is_err());
    }
}
