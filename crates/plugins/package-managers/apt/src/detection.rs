use std::collections::HashMap;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, PluginError, Result, Version,
};

use crate::discovery::parse_dpkg_output;
use crate::plugin::{AptPlugin, validate_identifier};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for AptPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting APT installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "dpkg-query",
                [
                    "--show".to_string(),
                    "--showformat=${Version}\\n".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "dpkg-query failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {
                let version = cmd_output.output.trim().to_string();
                if version.is_empty() {
                    return Ok(None);
                }
                tracing::debug!(version = %version, "APT installed version detected");
                Ok(Some(Version::new(&version)))
            }
            // Exit code 1 means the package was not found.
            1 => {
                tracing::debug!(
                    package = %package_identifier,
                    "package not found in dpkg database"
                );
                Ok(None)
            }
            code => bail!(PluginError::CommandFailed(code)),
        }
    }

    /// Detect installed versions for multiple packages using a single `dpkg-query` call.
    ///
    /// Runs:
    /// ```text
    /// dpkg-query --show --showformat='${Package}\t${Version}\n' pkg1 pkg2 pkg3
    /// ```
    ///
    /// The exit code is intentionally ignored: `dpkg-query` exits non-zero when any
    /// requested package is unknown, but packages that *are* found still appear in
    /// stdout. Packages absent from stdout are treated as not installed (`None` with
    /// no error).
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

        let mut args = vec![
            "--show".to_string(),
            "--showformat=${Package}\\t${Version}\\n".to_string(),
        ];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting APT installed versions"
        );

        // Non-zero exit is expected when any package is unknown; ignore it.
        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec("dpkg-query", args))
            .await
        {
            Ok(o) => o.output,
            Err(e) => {
                // dpkg-query completely failed (e.g., not found on PATH).
                let error_str = format!("dpkg-query failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        // Parse output into a map for O(1) lookup.
        let dpkg_map: HashMap<String, String> = parse_dpkg_output(&stdout).into_iter().collect();

        let results = items
            .iter()
            .map(|item| {
                let installed_version = dpkg_map.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(count = items.len(), "APT batch version detection complete");
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
    use uptrakit_plugin_infrastructure_core::testing::RoutedOutputExecutor;
    use uptrakit_plugin_infrastructure_core::{
        BatchDetectItem, HostCapabilities, HostRuntime, LocalCommandExecutor, PosixHostRuntime,
        Version, VersionDetector,
    };

    use crate::config::AptConfig;

    /// Helper to create an `AptPlugin` from a mock executor for testing.
    fn test_plugin_with_executor(
        config: AptConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> AptPlugin {
        let caps = HostCapabilities::default();
        let runtime = Arc::new(PosixHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        AptPlugin::new(config, runtime).unwrap()
    }

    #[tokio::test]
    async fn batch_detect_found_packages() {
        let executor = RoutedOutputExecutor::success([(
            "dpkg-query",
            "nginx\t1.24.0-2ubuntu7.3\npython3\t3.11.0-5ubuntu2\n",
        )]);
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);

        let items = vec![
            BatchDetectItem::new("nginx".to_string()),
            BatchDetectItem::new("python3".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.expect("ok");

        assert_eq!(results.len(), 2);
        let nginx = results
            .iter()
            .find(|r| r.package_identifier == "nginx")
            .unwrap();
        assert_eq!(
            nginx.installed_version,
            Some(Version::new("1.24.0-2ubuntu7.3"))
        );
        assert!(nginx.error.is_none());

        let python3 = results
            .iter()
            .find(|r| r.package_identifier == "python3")
            .unwrap();
        assert_eq!(
            python3.installed_version,
            Some(Version::new("3.11.0-5ubuntu2"))
        );
        assert!(python3.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_package_not_in_output_is_not_installed() {
        // dpkg-query returns output for nginx only; curl is absent (not installed).
        let executor = RoutedOutputExecutor::success([("dpkg-query", "nginx\t1.24.0\n")]);
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);

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
        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);
        let results = plugin.batch_detect(&[]).await.expect("ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_detect_invalid_identifier_fails() {
        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);
        let items = vec![BatchDetectItem::new("INVALID_UPPERCASE".to_string())];
        let result = plugin.batch_detect(&items).await;
        assert!(result.is_err());
    }
}
