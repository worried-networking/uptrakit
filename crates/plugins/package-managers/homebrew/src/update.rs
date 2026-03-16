use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchUpdateItem, BatchUpdateResult, OutputStreamType, PluginError, ReleaseInfo, Result,
    UpdateOutputLine,
};

use crate::plugin::HomebrewPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PackageIndexPlugin for HomebrewPlugin {
    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing Homebrew package index");
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("brew", ["update".to_string()]))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "brew update failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        tracing::info!("Homebrew package index refreshed");
        Ok(())
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin for HomebrewPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;
        let pkg = package_identifier;
        let mut output = String::new();

        let args: Vec<String> = if self.is_cask() {
            vec!["upgrade".to_string(), "--cask".to_string(), pkg.to_string()]
        } else {
            vec!["upgrade".to_string(), pkg.to_string()]
        };

        tracing::debug!(package = %pkg, "running brew upgrade");
        send_output(
            output_tx,
            &format!("Running: brew {}", args.join(" ")),
            OutputStreamType::Stdout,
        )
        .await;
        output.push_str(&format!("Running: brew {}\n", args.join(" ")));

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("brew", args), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output.output);

        Ok(output)
    }

    /// Execute batch updates using a single `brew upgrade pkg1 pkg2 ...` command.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        for item in items {
            self.require_package_identifier(&item.package_identifier)?;
        }

        let mut args: Vec<String> = vec!["upgrade".to_string()];
        if self.is_cask() {
            args.push("--cask".to_string());
        }
        for item in items {
            args.push(item.package_identifier.clone());
        }

        let display_cmd = format!("brew {}", args.join(" "));
        send_output(
            output_tx,
            &format!(
                "Batch updating {} packages\nRunning: {display_cmd}",
                items.len()
            ),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_cmd}\n");

        tracing::debug!(count = items.len(), "running brew batch upgrade");

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("brew", args), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output.output);

        let success = cmd_output.exit_code == 0;
        let results = items
            .iter()
            .map(|item| {
                BatchUpdateResult::new(item.package_identifier.clone(), success, output.clone())
            })
            .collect();

        Ok(results)
    }
}
