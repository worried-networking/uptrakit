use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, DiscoveryTarget, HostCompatibility, PluginError, PluginRole, Result,
    plugin_ids,
};

use crate::plugin::{NpmPlugin, SYSTEM_NPM_PACKAGES};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for NpmPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering globally installed npm packages");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "npm",
                [
                    "list".to_string(),
                    "-g".to_string(),
                    "--depth=0".to_string(),
                    "--json".to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "npm list -g failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        // A zero-exit `npm list` must emit parseable json. A parse failure signals
        // a tool malfunction (truncated output, non-json noise), not an empty
        // install — surface it as an error rather than silently reporting zero
        // packages (discovery contract: failures set error, empty means empty).
        if serde_json::from_str::<serde_json::Value>(&cmd_output.output).is_err() {
            bail!(PluginError::PluginInternal(
                "npm list -g returned unparseable json output".to_string()
            ));
        }

        let all_packages = NpmPlugin::parse_npm_list_all(&cmd_output.output);

        let packages: Vec<DiscoveredSoftware> = all_packages
            .into_iter()
            .filter(|(name, _)| !SYSTEM_NPM_PACKAGES.contains(&name.as_str()))
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_NPM.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "npm".to_string(),
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

        tracing::debug!(count = packages.len(), "npm software discovery complete");
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["npm".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible("npm not found".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {

    use uptrakit_plugin_infrastructure_core::testing::{
        FixedOutputExecutor, test_runtime_with_executor,
    };
    use uptrakit_plugin_infrastructure_core::{Discoverer, plugin_ids};

    use crate::config::NpmConfig;
    use crate::plugin::NpmPlugin;

    // ── discover_software ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_always_emits_targets() {
        let json = r#"{"dependencies":{"n8n":{"version":"1.18.0"}}}"#;
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new(json, 0)),
        )
        .expect("create");
        let discovered = plugin.discover_software().await.expect("ok");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].targets.len(), 1);
        assert_eq!(
            discovered[0].targets[0].plugin_type,
            plugin_ids::PACKAGE_MANAGER_NPM.clone()
        );
        assert_eq!(discovered[0].targets[0].plugin_config_name, "npm");
        assert_eq!(discovered[0].targets[0].roles.len(), 3);
    }

    #[tokio::test]
    async fn discover_software_excludes_system_packages() {
        let json = r#"{"dependencies":{"npm":{"version":"10.0.0"},"n8n":{"version":"1.18.0"}}}"#;
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new(json, 0)),
        )
        .expect("create");
        let discovered = plugin.discover_software().await.expect("ok");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, "n8n");
    }

    #[tokio::test]
    async fn discover_software_unparseable_json_errors() {
        // zero exit but garbage output => tool malfunction, must surface an error
        // (not silently report an empty install).
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new("not json at all", 0)),
        )
        .expect("create");
        plugin.discover_software().await.unwrap_err();
    }

    #[tokio::test]
    async fn discover_software_empty_dependencies_is_empty() {
        // Valid json with no dependencies is a legitimate empty install, not an error.
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new(r#"{"dependencies":{}}"#, 0)),
        )
        .expect("create");
        let discovered = plugin.discover_software().await.expect("ok");
        assert!(discovered.is_empty());
    }
}
