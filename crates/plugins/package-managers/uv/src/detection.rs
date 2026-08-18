use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::helpers::{
    execute_batch_detect_read_command, validation_error_message,
};
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, PluginError, Result, Version, execute_and_capture,
};

use crate::plugin::{UvPlugin, parse_uv_tool_list, validate_identifier};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for UvPlugin {
    /// Detect the installed version of a single uv tool.
    ///
    /// uv has no per-package query; the full `uv tool list` output is parsed
    /// and the matching row selected. An absent row is `Ok(None)`.
    #[tracing::instrument(skip_all, fields(package_identifier = %package_identifier))]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;

        let output = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("uv", ["tool".to_string(), "list".to_string()]),
            "uv tool list",
        )
        .await?;

        let installed = parse_uv_tool_list(&output);
        Ok(installed.get(package_identifier).map(Version::new))
    }

    /// Detect versions for multiple tools with a single `uv tool list` call.
    #[tracing::instrument(skip_all, fields(item_count = items.len()))]
    async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;
        }

        let output = match execute_batch_detect_read_command(
            self.executor.as_ref(),
            CommandSpec::exec("uv", ["tool".to_string(), "list".to_string()]),
            items,
            "uv tool list",
        )
        .await
        {
            Ok(output) => output,
            Err(results) => return Ok(results),
        };

        let installed = parse_uv_tool_list(&output);
        Ok(items
            .iter()
            .map(|item| {
                BatchDetectResult::new(
                    item.package_identifier.clone(),
                    installed.get(&item.package_identifier).map(Version::new),
                    None,
                )
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_plugin_infrastructure_core::testing::{
        FixedOutputExecutor, test_runtime_with_executor,
    };
    use uptrakit_plugin_infrastructure_core::{
        BatchDetectItem, PluginError, Version, VersionDetector,
    };

    use crate::config::UvConfig;
    use crate::plugin::UvPlugin;

    fn make_plugin(stdout: &str) -> UvPlugin {
        UvPlugin::new(
            UvConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::success(stdout)),
        )
        .expect("construct plugin")
    }

    fn make_failing_plugin(exit_code: i32) -> UvPlugin {
        UvPlugin::new(
            UvConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::failure(exit_code)),
        )
        .expect("construct plugin")
    }

    fn item(package_identifier: &str) -> BatchDetectItem {
        BatchDetectItem::new(package_identifier)
    }

    // ── detect_installed_version ────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_found() {
        let output = "ruff v0.6.8\n- ruff\nblack v24.4.2\n- black\n";
        let plugin = make_plugin(output);
        let version = plugin.detect_installed_version("ruff").await.unwrap();
        assert_eq!(version, Some(Version::new("0.6.8")));
    }

    #[tokio::test]
    async fn detect_installed_version_not_found_is_ok_none() {
        let output = "ruff v0.6.8\n- ruff\n";
        let plugin = make_plugin(output);
        let version = plugin.detect_installed_version("black").await.unwrap();
        assert_eq!(version, None);
    }

    #[tokio::test]
    async fn detect_installed_version_command_failure_is_plugin_internal() {
        let plugin = make_failing_plugin(1);
        let Err(err) = plugin.detect_installed_version("ruff").await else {
            panic!("expected detection to fail");
        };
        assert!(matches!(
            err.current_context(),
            PluginError::PluginInternal(_)
        ));
    }

    #[tokio::test]
    async fn detect_installed_version_invalid_identifier_fails() {
        let plugin = make_plugin("");
        let Err(err) = plugin.detect_installed_version("owner/pkg").await else {
            panic!("expected validation to fail");
        };
        assert!(matches!(
            err.current_context(),
            PluginError::Configuration(_)
        ));
    }

    // ── batch_detect ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_single_list_call() {
        let output = "ruff v0.6.8\n- ruff\nblack v24.4.2\n- black\n";
        let plugin = make_plugin(output);
        let items = vec![item("ruff"), item("black"), item("missing")];
        let results = plugin.batch_detect(&items).await.unwrap();
        assert_eq!(results.len(), 3);

        let ruff = results
            .iter()
            .find(|r| r.package_identifier == "ruff")
            .expect("ruff result");
        assert_eq!(ruff.installed_version, Some(Version::new("0.6.8")));
        assert!(ruff.error.is_none());

        let missing = results
            .iter()
            .find(|r| r.package_identifier == "missing")
            .expect("missing result");
        assert_eq!(missing.installed_version, None);
        assert!(missing.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_command_failure_fans_out_per_item_errors() {
        let plugin = make_failing_plugin(1);
        let items = vec![item("ruff"), item("black")];
        let results = plugin.batch_detect(&items).await.unwrap();
        assert_eq!(results.len(), 2);
        for result in &results {
            assert!(result.error.is_some(), "expected per-item error");
            assert!(result.installed_version.is_none());
        }
    }

    /// Drift output (e.g. a future uv release dropping the `v` prefix)
    /// parses to zero tools, but `batch_detect` has no drift guard of its
    /// own (unlike `discover_software`) — every requested item resolves to
    /// `installed_version: None` with no error, same as "tool not
    /// installed". This is the documented discovery/detection asymmetry;
    /// this test pins the existing behavior without changing it.
    #[tokio::test]
    async fn batch_detect_drift_output_returns_none_not_error() {
        let plugin = make_plugin("ruff 0.6.8\nblack 24.4.2\n");
        let items = vec![item("ruff"), item("black")];
        let results = plugin.batch_detect(&items).await.unwrap();
        assert_eq!(results.len(), 2);
        for result in &results {
            assert_eq!(result.installed_version, None);
            assert!(result.error.is_none());
        }
    }

    #[tokio::test]
    async fn batch_detect_empty_returns_empty() {
        let plugin = make_plugin("");
        let results = plugin.batch_detect(&[]).await.unwrap();
        assert!(results.is_empty());
    }

    /// The per-item identifier validation loop in `batch_detect` must reject
    /// an invalid identifier before ever issuing `uv tool list` — an invalid
    /// item anywhere in the batch fails the whole call.
    #[tokio::test]
    async fn batch_detect_invalid_identifier_fails() {
        let plugin = make_plugin("");
        let items = vec![item("owner/pkg")];
        let Err(err) = plugin.batch_detect(&items).await else {
            panic!("expected validation to fail");
        };
        assert!(matches!(
            err.current_context(),
            PluginError::Configuration(_)
        ));
    }
}
