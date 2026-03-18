use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::{
    BatchUpdateItem, BatchUpdateResult, OutputStreamType, PluginError, ReleaseInfo, Result,
    UpdateOutputSender,
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

        let pkg_version = format!("{package_identifier}@{to_version}");
        let args = vec!["install".to_string(), "-g".to_string(), pkg_version];

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running npm install -g"
        );

        let display_args = std::iter::once("npm")
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
            .execute(&CommandSpec::exec("npm", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "npm install -g failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }

    /// Execute batch updates using a single `npm install -g pkg1@v1 pkg2@v2 ...` command.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &UpdateOutputSender,
    ) -> Result<Vec<BatchUpdateResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        for item in items {
            self.require_package_identifier(&item.package_identifier)?;
            validate_version(&item.to_version)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["install".to_string(), "-g".to_string()];
        for item in items {
            args.push(format!("{}@{}", item.package_identifier, item.to_version));
        }

        let display_args = std::iter::once("npm")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        send_output(
            output_tx,
            &format!(
                "Batch updating {} packages\nRunning: {display_args}",
                items.len()
            ),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        tracing::debug!(count = items.len(), "running npm batch install -g");

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("npm", args).privileged(), output_tx)
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
