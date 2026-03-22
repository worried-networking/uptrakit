use std::collections::HashMap;

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{BatchDetectItem, BatchDetectResult, Result, Version};

use crate::plugin::NpmPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for NpmPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting npm installed version");

        // npm exits non-zero when a package is not found; treat any non-zero
        // (Err from execute_quiet) as not installed.
        let cmd_output = match self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "npm",
                [
                    "list".to_string(),
                    "-g".to_string(),
                    package_identifier.to_string(),
                    "--depth=0".to_string(),
                    "--json".to_string(),
                ],
            ))
            .await
        {
            Ok(output) => output,
            Err(_) => {
                tracing::debug!(
                    package = %package_identifier,
                    "npm list returned non-zero; package not installed"
                );
                return Ok(None);
            }
        };

        let version = NpmPlugin::parse_npm_list_version(&cmd_output.output, package_identifier);
        tracing::debug!(package = %package_identifier, version = ?version, "npm installed version");
        Ok(version.map(|v| Version::new(&v)))
    }

    /// Detect installed versions for multiple packages using a single `npm list -g` call.
    ///
    /// Runs:
    /// ```text
    /// npm list -g --depth=0 --json
    /// ```
    ///
    /// Fetches all globally installed packages in one subprocess call and filters
    /// the results in memory. This is more efficient than per-package calls when
    /// checking many packages.
    ///
    /// If the command fails (non-zero exit or process error), all items are treated
    /// as not installed rather than erroring — consistent with the single-item
    /// `detect_installed_version` behaviour.
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
            "batch detecting npm installed versions"
        );

        // Run a single `npm list -g --depth=0 --json` without a package filter.
        // npm exits non-zero when there are peer-dep issues; treat any failure as
        // "not installed" for all items (consistent with the single-item behaviour).
        let all_packages: HashMap<String, String> = match self
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
        {
            Ok(output) => NpmPlugin::parse_npm_list_all(&output.output)
                .into_iter()
                .collect(),
            Err(_) => {
                tracing::debug!(
                    "npm list -g returned non-zero; treating all packages as not installed"
                );
                HashMap::new()
            }
        };

        let results = items
            .iter()
            .map(|item| {
                BatchDetectResult::new(
                    item.package_identifier.clone(),
                    all_packages.get(&item.package_identifier).map(Version::new),
                    None,
                )
            })
            .collect();

        tracing::debug!(count = items.len(), "npm batch version detection complete");
        Ok(results)
    }
}

#[cfg(test)]
mod tests {

    use uptrakit_plugin_infrastructure_core::testing::{
        FixedOutputExecutor, test_runtime, test_runtime_with_executor,
    };
    use uptrakit_plugin_infrastructure_core::{BatchDetectItem, Version, VersionDetector};

    use crate::config::NpmConfig;
    use crate::plugin::NpmPlugin;

    // ── detect_installed_version ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_found() {
        let json = r#"{"dependencies":{"n8n":{"version":"1.18.0"}}}"#;
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new(json, 0)),
        )
        .expect("create");
        let result = plugin.detect_installed_version("n8n").await.expect("ok");
        assert_eq!(result, Some(Version::new("1.18.0")));
    }

    #[tokio::test]
    async fn detect_installed_version_not_installed() {
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new("", 1)),
        )
        .expect("create");
        let result = plugin.detect_installed_version("n8n").await.expect("ok");
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn detect_installed_version_empty_identifier_fails() {
        let plugin = NpmPlugin::new(NpmConfig::default(), test_runtime()).expect("create");
        let result = plugin.detect_installed_version("").await;
        assert!(result.is_err());
    }

    // ── batch_detect ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_empty_returns_empty() {
        let plugin = NpmPlugin::new(NpmConfig::default(), test_runtime()).expect("create");
        let result = plugin.batch_detect(&[]).await.expect("ok");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn batch_detect_found() {
        let json = r#"{"dependencies":{"n8n":{"version":"1.18.0"},"pm2":{"version":"5.3.0"}}}"#;
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new(json, 0)),
        )
        .expect("create");
        let items = vec![
            BatchDetectItem::new("n8n".to_string()),
            BatchDetectItem::new("pm2".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.expect("ok");
        assert_eq!(results.len(), 2);
        let n8n = results
            .iter()
            .find(|r| r.package_identifier == "n8n")
            .expect("n8n");
        assert_eq!(n8n.installed_version, Some(Version::new("1.18.0")));
        assert!(n8n.error.is_none());
        let pm2 = results
            .iter()
            .find(|r| r.package_identifier == "pm2")
            .expect("pm2");
        assert_eq!(pm2.installed_version, Some(Version::new("5.3.0")));
        assert!(pm2.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_not_installed_package() {
        // The package "missing" is not in the npm list output.
        let json = r#"{"dependencies":{"n8n":{"version":"1.18.0"}}}"#;
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new(json, 0)),
        )
        .expect("create");
        let items = vec![
            BatchDetectItem::new("n8n".to_string()),
            BatchDetectItem::new("missing".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.expect("ok");
        assert_eq!(results.len(), 2);
        let found = results
            .iter()
            .find(|r| r.package_identifier == "n8n")
            .expect("n8n");
        assert_eq!(found.installed_version, Some(Version::new("1.18.0")));
        assert!(found.error.is_none());
        let missing = results
            .iter()
            .find(|r| r.package_identifier == "missing")
            .expect("missing");
        assert_eq!(missing.installed_version, None);
        assert!(missing.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_command_fails_all_not_installed() {
        // When npm list -g exits non-zero, all items are treated as not installed.
        let plugin = NpmPlugin::new(
            NpmConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new("", 1)),
        )
        .expect("create");
        let items = vec![
            BatchDetectItem::new("n8n".to_string()),
            BatchDetectItem::new("pm2".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.expect("ok");
        assert_eq!(results.len(), 2);
        for r in &results {
            assert_eq!(r.installed_version, None);
            assert!(r.error.is_none());
        }
    }

    #[tokio::test]
    async fn batch_detect_invalid_identifier_fails() {
        let plugin = NpmPlugin::new(NpmConfig::default(), test_runtime()).expect("create");
        let items = vec![
            BatchDetectItem::new("valid".to_string()),
            BatchDetectItem::new("Invalid Package!".to_string()),
        ];
        let result = plugin.batch_detect(&items).await;
        assert!(result.is_err());
    }
}
