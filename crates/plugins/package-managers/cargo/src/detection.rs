use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, PluginError, Result, Version,
};

use crate::plugin::{CargoPlugin, parse_cargo_install_list, validate_identifier};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for CargoPlugin {
    /// Detect the installed version of a single crate.
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting cargo-installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "cargo",
                ["install".to_string(), "--list".to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "cargo install --list failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let installed = parse_cargo_install_list(&cmd_output.output);
        let version = installed.get(package_identifier).map(Version::new);

        if let Some(ref v) = version {
            tracing::debug!(version = %v, "cargo installed version detected");
        } else {
            tracing::debug!(package = %package_identifier, "crate not found in cargo install list");
        }

        Ok(version)
    }

    /// Detect installed versions for multiple crates using a single `cargo install --list` call.
    #[tracing::instrument(skip_all)]
    async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting cargo-installed versions"
        );

        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "cargo",
                ["install".to_string(), "--list".to_string()],
            ))
            .await
        {
            Ok(o) => {
                if o.exit_code != 0 {
                    let error_str =
                        format!("cargo install --list failed with exit code {}", o.exit_code);
                    return Ok(items
                        .iter()
                        .map(|item| {
                            BatchDetectResult::error(
                                item.package_identifier.clone(),
                                error_str.clone(),
                            )
                        })
                        .collect());
                }
                o.output
            }
            Err(e) => {
                let error_str = format!("cargo install --list failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        let installed = parse_cargo_install_list(&stdout);

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
    use std::sync::Arc;

    use async_trait::async_trait;
    use uptrakit_plugin_infrastructure_core::command::{
        CommandExecutor, CommandOutput, CommandSpec,
    };
    use uptrakit_plugin_infrastructure_core::mpsc;
    use uptrakit_plugin_infrastructure_core::{
        BatchDetectItem, HostCapabilities, HostRuntime, StandardHostRuntime, UpdateOutputLine,
        Version, VersionDetector,
    };

    use crate::config::CargoConfig;
    use crate::plugin::CargoPlugin;

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

    fn make_plugin(config: CargoConfig, stdout: &str, exit_code: i32) -> CargoPlugin {
        let executor = Arc::new(FixedOutputExecutor {
            output: stdout.to_string(),
            exit_code,
        }) as Arc<dyn CommandExecutor>;
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        CargoPlugin::new(config, runtime).unwrap()
    }

    // ── detect_installed_version ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_found() {
        let output = "bat v0.24.0:\n    bat\nripgrep v14.1.1:\n    rg\n";
        let plugin = make_plugin(CargoConfig::default(), output, 0);

        let result = plugin.detect_installed_version("bat").await.unwrap();
        assert_eq!(result, Some(Version::new("0.24.0")));
    }

    #[tokio::test]
    async fn detect_installed_version_not_found() {
        let output = "bat v0.24.0:\n    bat\n";
        let plugin = make_plugin(CargoConfig::default(), output, 0);

        let result = plugin.detect_installed_version("ripgrep").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_installed_version_invalid_identifier_fails() {
        let plugin = make_plugin(CargoConfig::default(), "", 0);

        assert!(plugin.detect_installed_version("1invalid").await.is_err());
        assert!(plugin.detect_installed_version("owner/repo").await.is_err());
    }

    // ── batch_detect ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_basic() {
        let output = "bat v0.24.0:\n    bat\nripgrep v14.1.1:\n    rg\n";
        let plugin = make_plugin(CargoConfig::default(), output, 0);

        let items = vec![
            BatchDetectItem::new("bat".to_string()),
            BatchDetectItem::new("ripgrep".to_string()),
            BatchDetectItem::new("notinstalled".to_string()),
        ];

        let results = plugin.batch_detect(&items).await.unwrap();
        assert_eq!(results.len(), 3);

        let bat = results
            .iter()
            .find(|r| r.package_identifier == "bat")
            .unwrap();
        assert_eq!(bat.installed_version, Some(Version::new("0.24.0")));

        let rg = results
            .iter()
            .find(|r| r.package_identifier == "ripgrep")
            .unwrap();
        assert_eq!(rg.installed_version, Some(Version::new("14.1.1")));

        let missing = results
            .iter()
            .find(|r| r.package_identifier == "notinstalled")
            .unwrap();
        assert!(missing.installed_version.is_none());
    }

    #[tokio::test]
    async fn batch_detect_empty_returns_empty() {
        let plugin = make_plugin(CargoConfig::default(), "", 0);

        let results = plugin.batch_detect(&[]).await.unwrap();
        assert!(results.is_empty());
    }
}
