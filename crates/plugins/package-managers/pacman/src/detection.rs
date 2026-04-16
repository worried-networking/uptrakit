use std::collections::HashMap;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, PluginError, Result, Version,
};

use crate::plugin::{PacmanPlugin, validate_identifier};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting Pacman installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "pacman",
                ["-Q".to_string(), package_identifier.to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pacman -Q failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {
                // Output is "name version\n".
                let version = cmd_output.output.split_whitespace().nth(1).map(|v| {
                    tracing::debug!(version = %v, "Pacman installed version detected");
                    Version::new(v)
                });
                Ok(version)
            }
            // Exit code 1 means the package was not found.
            1 => {
                tracing::debug!(
                    package = %package_identifier,
                    "package not found in pacman database"
                );
                Ok(None)
            }
            code => bail!(PluginError::CommandFailed(code)),
        }
    }

    /// Detect installed versions for multiple packages using a single
    /// `pacman -Q` call.
    ///
    /// Runs:
    /// ```text
    /// pacman -Q pkg1 pkg2 pkg3
    /// ```
    ///
    /// The exit code is intentionally ignored: `pacman -Q` exits non-zero when
    /// any requested package is not installed, but packages that *are* installed
    /// still appear in stdout. Packages absent from stdout are treated as not
    /// installed (`None` with no error).
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

        let mut args = vec!["-Q".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting Pacman installed versions"
        );

        // Non-zero exit is expected when any package is unknown; ignore it.
        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec("pacman", args))
            .await
        {
            Ok(o) => o.output,
            Err(e) => {
                // pacman completely failed (e.g., not found on PATH).
                let error_str = format!("pacman -Q failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        // Parse output into a map for O(1) lookup.
        let query_map: HashMap<String, String> = PacmanPlugin::parse_query_output(&stdout)
            .into_iter()
            .collect();

        let results = items
            .iter()
            .map(|item| {
                let installed_version = query_map.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(
            count = items.len(),
            "Pacman batch version detection complete"
        );
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
    use uptrakit_plugin_infrastructure_core::testing::RoutedOutputExecutor;
    use uptrakit_plugin_infrastructure_core::{
        BatchDetectItem, HostCapabilities, HostRuntime, LocalCommandExecutor, StandardHostRuntime,
        Version, VersionDetector,
    };

    use crate::config::PacmanConfig;
    use crate::plugin::PacmanPlugin;

    fn test_plugin_with_executor(
        config: PacmanConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> PacmanPlugin {
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        PacmanPlugin::new(config, runtime).unwrap()
    }

    // ── batch_detect ───────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_found_packages() {
        let executor =
            RoutedOutputExecutor::success([("pacman", "nginx 1.26.3-1\npython 3.12.4-1\n")]);
        let plugin = test_plugin_with_executor(PacmanConfig::default(), executor);

        let items = vec![
            BatchDetectItem::new("nginx".to_string()),
            BatchDetectItem::new("python".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.expect("ok");

        assert_eq!(results.len(), 2);
        let nginx = results
            .iter()
            .find(|r| r.package_identifier == "nginx")
            .unwrap();
        assert_eq!(nginx.installed_version, Some(Version::new("1.26.3-1")));
        assert!(nginx.error.is_none());

        let python = results
            .iter()
            .find(|r| r.package_identifier == "python")
            .unwrap();
        assert_eq!(python.installed_version, Some(Version::new("3.12.4-1")));
        assert!(python.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_package_not_in_output_is_not_installed() {
        let executor = RoutedOutputExecutor::success([("pacman", "nginx 1.26.3-1\n")]);
        let plugin = test_plugin_with_executor(PacmanConfig::default(), executor);

        let items = vec![
            BatchDetectItem::new("nginx".to_string()),
            BatchDetectItem::new("curl".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.expect("ok");

        assert_eq!(results.len(), 2);
        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert!(curl.installed_version.is_none());
        assert!(curl.error.is_none(), "absent package is not an error");
    }

    #[tokio::test]
    async fn batch_detect_empty_items_returns_empty() {
        let plugin = test_plugin_with_executor(
            PacmanConfig::default(),
            Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>,
        );
        let results = plugin.batch_detect(&[]).await.expect("ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_detect_invalid_identifier_fails() {
        let plugin = test_plugin_with_executor(
            PacmanConfig::default(),
            Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>,
        );
        let items = vec![BatchDetectItem::new("INVALID_UPPERCASE".to_string())];
        let result = plugin.batch_detect(&items).await;
        assert!(result.is_err());
    }
}
