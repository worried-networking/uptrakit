use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    BatchUpdateItem, BatchUpdateResult, ExecuteUpdateResult, PluginError, ReleaseInfo, Result,
    UpdateOutputSender,
};

use crate::plugin::HomebrewPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PackageIndexer for HomebrewPlugin {
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
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for HomebrewPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &UpdateOutputSender,
    ) -> Result<ExecuteUpdateResult> {
        self.require_package_identifier(package_identifier)?;

        let args: Vec<String> = if self.is_cask() {
            vec![
                "upgrade".to_string(),
                "--cask".to_string(),
                package_identifier.to_string(),
            ]
        } else {
            vec!["upgrade".to_string(), package_identifier.to_string()]
        };

        tracing::debug!(package = %package_identifier, "running brew upgrade");

        let output = uptrakit_plugin_infrastructure_core::execute_command_update(
            uptrakit_plugin_infrastructure_core::CommandUpdateParams {
                executor: self.executor.as_ref(),
                binary: "brew",
                args,
                privileged: false,
                spec_modifier: None,
                exit_code_success: Some(|_| true),
                exit_code_error: None,
            },
            output_tx,
        )
        .await?;
        Ok(ExecuteUpdateResult::new(output, false))
    }

    /// Execute batch updates using a single `brew upgrade pkg1 pkg2 ...` command.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &UpdateOutputSender,
    ) -> Result<Vec<BatchUpdateResult>> {
        if !items.is_empty() {
            uptrakit_plugin_infrastructure_core::command::send_output(
                output_tx,
                &format!("Batch updating {} packages", items.len()),
                uptrakit_plugin_infrastructure_core::OutputStreamType::Stdout,
            )
            .await;
        }
        let prefix_args: Vec<String> = if self.is_cask() {
            vec!["upgrade".to_string(), "--cask".to_string()]
        } else {
            vec!["upgrade".to_string()]
        };
        tracing::debug!(count = items.len(), "running brew batch upgrade");
        uptrakit_plugin_infrastructure_core::execute_batch_names_command(
            uptrakit_plugin_infrastructure_core::BatchNamesParams {
                executor: self.executor.as_ref(),
                binary: "brew",
                prefix_args,
                privileged: false,
                suffix_args: vec![],
                validate_identifier: crate::plugin::validate_identifier_nonempty,
                validate_version: None,
            },
            items,
            output_tx,
        )
        .await
    }
}
