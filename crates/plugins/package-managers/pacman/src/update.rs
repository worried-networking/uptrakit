use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchUpdateItem, BatchUpdateResult, OutputStreamType, PluginError, ReleaseInfo, Result,
    UpdateOutputLine,
};

use crate::plugin::{PacmanPlugin, validate_identifier, validate_version};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin for PacmanPlugin {
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

        // Pacman always installs the latest version from the repository;
        // version pinning is not supported. The `to_version` argument is
        // validated for safety but not passed to the command.
        let args = vec![
            "-S".to_string(),
            "--noconfirm".to_string(),
            package_identifier.to_string(),
        ];

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running pacman -S --noconfirm"
        );

        let display_args = std::iter::once("pacman")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        send_output(
            output_tx,
            &format!("Running: {display_args}"),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("pacman", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "pacman -S failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
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
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all package identifiers and versions up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
            validate_version(&item.to_version)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["-S".to_string(), "--noconfirm".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        let display_args = std::iter::once("pacman")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        let pkg_list: Vec<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();
        send_output(
            output_tx,
            &format!(
                "Batch updating {} packages: {}\nRunning: {display_args}",
                items.len(),
                pkg_list.join(", ")
            ),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        tracing::debug!(
            count = items.len(),
            packages = ?pkg_list,
            "running pacman batch install"
        );

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("pacman", args).privileged(), output_tx)
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
