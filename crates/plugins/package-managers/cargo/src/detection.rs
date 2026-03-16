use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, PluginError, Result, Version,
};

use crate::plugin::{CargoPlugin, parse_cargo_install_list, validate_identifier};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetectorPlugin for CargoPlugin {
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
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> Result<Vec<BatchDetectResult>> {
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
    use super::*;
    use uptrakit_plugin_infrastructure_core::VersionDetectorPlugin;
    use uptrakit_plugin_infrastructure_core::testing::FixedOutputExecutor;

    use crate::config::CargoConfig;

    // ── detect_installed_version ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_found() {
        let output = "bat v0.24.0:\n    bat\nripgrep v14.1.1:\n    rg\n";
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::success(output))
            .await
            .unwrap();

        let result = plugin.detect_installed_version("bat").await.unwrap();
        assert_eq!(result, Some(Version::new("0.24.0")));
    }

    #[tokio::test]
    async fn detect_installed_version_not_found() {
        let output = "bat v0.24.0:\n    bat\n";
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::success(output))
            .await
            .unwrap();

        let result = plugin.detect_installed_version("ripgrep").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_installed_version_invalid_identifier_fails() {
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::success(""))
            .await
            .unwrap();

        assert!(plugin.detect_installed_version("1invalid").await.is_err());
        assert!(plugin.detect_installed_version("owner/repo").await.is_err());
    }

    // ── batch_detect_installed_version ────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_installed_version_basic() {
        let output = "bat v0.24.0:\n    bat\nripgrep v14.1.1:\n    rg\n";
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::success(output))
            .await
            .unwrap();

        let items = vec![
            BatchDetectItem::new("bat".to_string()),
            BatchDetectItem::new("ripgrep".to_string()),
            BatchDetectItem::new("notinstalled".to_string()),
        ];

        let results = plugin.batch_detect_installed_version(&items).await.unwrap();
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
    async fn batch_detect_installed_version_empty_returns_empty() {
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::success(""))
            .await
            .unwrap();

        let results = plugin.batch_detect_installed_version(&[]).await.unwrap();
        assert!(results.is_empty());
    }
}
