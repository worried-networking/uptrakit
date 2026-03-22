use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::{
    BatchUpdateItem, BatchUpdateResult, PluginError, ReleaseInfo, Result, UpdateOutputSender,
};

use crate::plugin::{NpmPlugin, validate_version};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for NpmPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &UpdateOutputSender,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;
        validate_version(to_version).map_err(|e| report!(PluginError::Configuration(e)))?;

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running npm install -g"
        );

        uptrakit_plugin_infrastructure_core::execute_command_update(
            uptrakit_plugin_infrastructure_core::CommandUpdateParams {
                executor: self.executor.as_ref(),
                binary: "npm",
                args: vec![
                    "install".to_string(),
                    "-g".to_string(),
                    format!("{package_identifier}@{to_version}"),
                ],
                privileged: true,
                spec_modifier: None,
                exit_code_success: None,
                exit_code_error: None,
            },
            output_tx,
        )
        .await
    }

    /// Execute batch updates using a single `npm install -g pkg1@v1 pkg2@v2 ...` command.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &UpdateOutputSender,
    ) -> Result<Vec<BatchUpdateResult>> {
        let context_prefix = if items.is_empty() {
            None
        } else {
            Some(format!("Batch updating {} packages", items.len()))
        };
        tracing::debug!(count = items.len(), "running npm batch install -g");
        uptrakit_plugin_infrastructure_core::execute_batch_versioned_command(
            uptrakit_plugin_infrastructure_core::BatchVersionedParams {
                executor: self.executor.as_ref(),
                binary: "npm",
                prefix_args: vec!["install".to_string(), "-g".to_string()],
                privileged: true,
                format_item: |id, ver| format!("{id}@{ver}"),
                validate_identifier: crate::plugin::validate_identifier,
                validate_version,
                context_prefix,
            },
            items,
            output_tx,
        )
        .await
    }
}
