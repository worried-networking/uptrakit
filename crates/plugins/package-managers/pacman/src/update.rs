use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchUpdateItem, BatchUpdateResult, PluginError, ReleaseInfo, Result, UpdateOutputLine,
};

use crate::plugin::{PacmanPlugin, validate_identifier, validate_version};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;
        validate_version(to_version).map_err(|e| report!(PluginError::Configuration(e)))?;

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running pacman -S --noconfirm"
        );

        uptrakit_plugin_infrastructure_core::execute_command_update(
            uptrakit_plugin_infrastructure_core::CommandUpdateParams {
                executor: self.executor.as_ref(),
                binary: "pacman",
                args: vec![
                    "-S".to_string(),
                    "--noconfirm".to_string(),
                    package_identifier.to_string(),
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

    /// Execute batch updates by installing all targeted packages in a single
    /// `pacman -S --noconfirm` invocation.
    ///
    /// All packages are installed or none — pacman treats the batch as a
    /// single transaction.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        let pkg_list: Vec<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();
        if !items.is_empty() {
            uptrakit_plugin_infrastructure_core::command::send_output(
                output_tx,
                &format!(
                    "Batch updating {} packages: {}",
                    items.len(),
                    pkg_list.join(", ")
                ),
                uptrakit_plugin_infrastructure_core::OutputStreamType::Stdout,
            )
            .await;
        }
        tracing::debug!(
            count = items.len(),
            packages = ?pkg_list,
            "running pacman batch install"
        );
        uptrakit_plugin_infrastructure_core::execute_batch_names_command(
            uptrakit_plugin_infrastructure_core::BatchNamesParams {
                executor: self.executor.as_ref(),
                binary: "pacman",
                prefix_args: vec!["-S".to_string(), "--noconfirm".to_string()],
                privileged: true,
                suffix_args: vec![],
                validate_identifier,
                validate_version: Some(validate_version),
            },
            items,
            output_tx,
        )
        .await
    }
}
