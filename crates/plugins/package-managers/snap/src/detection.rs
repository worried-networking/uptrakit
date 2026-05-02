use std::collections::HashMap;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::helpers::execute_batch_detect_read_command;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, PluginError, Result, Version,
};

use crate::plugin::{SnapPlugin, parse_snap_list_line};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for SnapPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting Snap installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "snap",
                ["list".to_string(), package_identifier.to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "snap list failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {
                // Output: header line + one data line.
                // Parse the data line (second column is version).
                let version = cmd_output
                    .output
                    .lines()
                    .filter_map(parse_snap_list_line)
                    .next()
                    .map(|(_, v)| v);

                match version {
                    Some(v) if !v.is_empty() => {
                        tracing::debug!(version = %v, "Snap installed version detected");
                        Ok(Some(Version::new(&v)))
                    }
                    _ => Ok(None),
                }
            }
            // Exit code 1 means the snap was not found.
            1 => {
                tracing::debug!(package = %package_identifier, "snap not found in installed list");
                Ok(None)
            }
            code => bail!(PluginError::CommandFailed(code)),
        }
    }

    /// Detect installed versions for multiple packages using a single `snap list` call.
    ///
    /// Runs `snap list` (no arguments) to get all installed snaps, then looks up
    /// each requested package in the resulting map. Command invocation
    /// failures, including non-zero `execute_quiet` exits from the standard
    /// executor, are downgraded to per-item errors.
    #[tracing::instrument(skip_all)]
    async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            self.require_package_identifier(&item.package_identifier)?;
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting Snap installed versions"
        );

        // A single `snap list` (no args) returns all installed snaps.
        let stdout = match execute_batch_detect_read_command(
            self.executor.as_ref(),
            CommandSpec::exec("snap", ["list".to_string()]),
            items,
            "snap list",
        )
        .await
        {
            Ok(stdout) => stdout,
            Err(results) => return Ok(results),
        };

        // Build a name -> version map from the output.
        let installed: HashMap<String, String> =
            stdout.lines().filter_map(parse_snap_list_line).collect();

        Ok(items
            .iter()
            .map(|item| {
                let installed_version = installed.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use std::sync::Arc;

    use async_trait::async_trait;
    use uptrakit_plugin_infrastructure_core::command::{
        CommandExecutor, CommandOutput, CommandSpec,
    };
    use uptrakit_plugin_infrastructure_core::mpsc;
    use uptrakit_plugin_infrastructure_core::testing::FixedOutputExecutor as StandardFixedOutputExecutor;
    use uptrakit_plugin_infrastructure_core::{
        BatchDetectItem, HostCapabilities, HostRuntime, StandardHostRuntime, UpdateOutputLine,
        Version, VersionDetector,
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

    // ── detect_installed_version ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_found() {
        let output = "Name  Version  Rev   Tracking        Publisher  Notes\n\
                      vlc   3.0.20   2359  latest/stable   videolan   -\n";
        let plugin = make_plugin(SnapConfig::default(), output, 0);

        let result = plugin.detect_installed_version("vlc").await.unwrap();
        assert_eq!(result, Some(Version::new("3.0.20")));
    }

    #[tokio::test]
    async fn detect_installed_version_not_found() {
        let plugin = make_plugin(
            SnapConfig::default(),
            "error: snap \"vlc\" is not installed\n",
            1,
        );

        let result = plugin.detect_installed_version("vlc").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_installed_version_invalid_identifier_fails() {
        let plugin = make_plugin(SnapConfig::default(), "", 0);

        assert!(plugin.detect_installed_version("VLC").await.is_err());
        assert!(plugin.detect_installed_version("-invalid").await.is_err());
    }

    // ── batch_detect ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_basic() {
        let output = "Name    Version   Rev    Tracking         Publisher  Notes\n\
                      vlc     3.0.20    2359   latest/stable    videolan   -\n\
                      code    1.85.2    163351 latest/stable    vscode     -\n";

        let plugin = make_plugin(SnapConfig::default(), output, 0);

        let items = vec![
            BatchDetectItem::new("vlc".to_string()),
            BatchDetectItem::new("code".to_string()),
            BatchDetectItem::new("notinstalled".to_string()),
        ];

        let results = plugin.batch_detect(&items).await.unwrap();
        assert_eq!(results.len(), 3);

        let vlc = results
            .iter()
            .find(|r| r.package_identifier == "vlc")
            .unwrap();
        assert_eq!(vlc.installed_version, Some(Version::new("3.0.20")));

        let code = results
            .iter()
            .find(|r| r.package_identifier == "code")
            .unwrap();
        assert_eq!(code.installed_version, Some(Version::new("1.85.2")));

        let missing = results
            .iter()
            .find(|r| r.package_identifier == "notinstalled")
            .unwrap();
        assert!(missing.installed_version.is_none());
    }

    #[tokio::test]
    async fn batch_detect_empty_returns_empty() {
        let plugin = make_plugin(SnapConfig::default(), "", 0);

        let results = plugin.batch_detect(&[]).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_detect_nonzero_execute_quiet_maps_error_to_each_item() {
        let executor = StandardFixedOutputExecutor::failure(1);
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        let plugin = SnapPlugin::new(SnapConfig::default(), runtime).unwrap();
        let items = vec![
            BatchDetectItem::new("vlc".to_string()),
            BatchDetectItem::new("code".to_string()),
        ];

        let results = plugin.batch_detect(&items).await.expect("per-item errors");

        assert_eq!(results.len(), 2);
        for result in &results {
            assert!(result.installed_version.is_none());
            let error = result.error.as_deref().expect("error message");
            assert!(
                error.contains("snap list failed"),
                "unexpected error: {error}"
            );
        }
    }
}
