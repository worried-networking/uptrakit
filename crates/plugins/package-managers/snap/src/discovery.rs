use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, DiscoveryTarget, HostCompatibility, PluginRole, Result,
    execute_and_capture, plugin_ids,
};

use crate::plugin::{SYSTEM_SNAPS, SnapPlugin, parse_snap_list_line};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for SnapPlugin {
    /// Discover Snap packages installed on the local system.
    ///
    /// Runs `snap list` and returns all user-installed snaps, excluding known
    /// system/infrastructure snaps (`core*`, `snapd`, `bare`).
    ///
    /// Always emits one [`DiscoveryTarget`] per snap so the controller can
    /// find-or-create a Snap plugin config and role assignments.
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering Snap-managed software");

        let stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("snap", ["list".to_string()]),
            "snap list",
        )
        .await?;

        let packages: Vec<DiscoveredSoftware> = stdout
            .lines()
            .filter_map(parse_snap_list_line)
            .filter(|(name, _)| !SYSTEM_SNAPS.contains(&name.as_str()))
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_SNAP.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "Snap".to_string(),
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

        tracing::debug!(count = packages.len(), "Snap software discovery complete");
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["snap".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "snap not found".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use uptrakit_plugin_infrastructure_core::command::{
        CommandExecutor, CommandOutput, CommandSpec,
    };
    use uptrakit_plugin_infrastructure_core::mpsc;
    use uptrakit_plugin_infrastructure_core::{
        Discoverer, HostCapabilities, HostRuntime, StandardHostRuntime, UpdateOutputLine,
        plugin_ids,
    };

    use crate::config::SnapConfig;
    use crate::plugin::SnapPlugin;

    /// Mock executor that always returns Ok (even for non-zero exit codes).
    struct FixedOutputExecutor {
        output: String,
        exit_code: i32,
    }

    #[async_trait]
    impl CommandExecutor for FixedOutputExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: self.exit_code,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: self.exit_code,
            })
        }
    }

    fn make_plugin(config: SnapConfig, stdout: &str, exit_code: i32) -> SnapPlugin {
        let executor = Arc::new(FixedOutputExecutor {
            output: stdout.to_string(),
            exit_code,
        }) as Arc<dyn CommandExecutor>;
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        SnapPlugin::new(config, runtime).unwrap()
    }

    // ── discover_software: system snap exclusion ──────────────────────────────

    #[tokio::test]
    async fn discover_software_excludes_system_snaps() {
        let snap_list_output = "Name    Version   Rev    Tracking         Publisher  Notes\n\
                                snapd   2.61.3    21759  latest/stable    canonical  snapd\n\
                                core20  20231212  2105   latest/stable    canonical  base\n\
                                core22  20231201  1234   latest/stable    canonical  base\n\
                                bare    1.0       5      latest/stable    canonical  base\n\
                                vlc     3.0.20    2359   latest/stable    videolan   -\n\
                                code    1.85.2    163351 latest/stable    vscode     -\n";

        let plugin = make_plugin(SnapConfig::default(), snap_list_output, 0);

        let discovered = plugin.discover_software().await.unwrap();

        // Only vlc and code should be discovered; system snaps excluded.
        assert_eq!(discovered.len(), 2);
        let names: Vec<&str> = discovered.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"vlc"));
        assert!(names.contains(&"code"));
        assert!(!names.contains(&"snapd"));
        assert!(!names.contains(&"core20"));
        assert!(!names.contains(&"core22"));
        assert!(!names.contains(&"bare"));
    }

    #[tokio::test]
    async fn discover_software_always_emits_targets() {
        let snap_list_output = "Name    Version   Rev    Tracking         Publisher  Notes\n\
                                vlc     3.0.20    2359   latest/stable    videolan   -\n";

        let plugin = make_plugin(SnapConfig::default(), snap_list_output, 0);

        let discovered = plugin.discover_software().await.unwrap();
        assert_eq!(discovered.len(), 1);
        assert!(!discovered[0].targets.is_empty());
        assert_eq!(
            discovered[0].targets[0].plugin_type,
            plugin_ids::PACKAGE_MANAGER_SNAP.clone()
        );
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_explicit_config() {
        let snap_list_output = "Name    Version   Rev    Tracking         Publisher  Notes\n\
                                vlc     3.0.20    2359   latest/stable    videolan   -\n";

        let plugin = make_plugin(
            SnapConfig {
                channel: Some("latest/stable".to_string()),
            },
            snap_list_output,
            0,
        );

        let discovered = plugin.discover_software().await.unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].targets.len(), 1);
    }
}
