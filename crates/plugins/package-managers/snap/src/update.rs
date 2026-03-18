use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchUpdateItem, BatchUpdateResult, OutputStreamType, PluginError, ReleaseInfo, Result,
    UpdateOutputLine,
};

use crate::plugin::{SnapPlugin, validate_identifier};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for SnapPlugin {
    /// Execute a single Snap package update via `snap refresh`.
    ///
    /// Runs `snap refresh <name>` with an optional `--channel=<channel>` argument
    /// when a channel is explicitly configured. Snap tracks channels rather than
    /// pinned version strings; the `to_version` parameter is used only for the
    /// display message prefix.
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;

        let mut args = vec!["refresh".to_string(), package_identifier.to_string()];
        if let Some(channel) = &self.config.channel {
            args.push(format!("--channel={channel}"));
        }

        tracing::debug!(
            package = %package_identifier,
            to_version = %to_version,
            channel = ?self.config.channel,
            "running snap refresh"
        );

        let display_args = std::iter::once("snap")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        send_output(
            output_tx,
            &format!("Updating {package_identifier} to {to_version}\nRunning: {display_args}"),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("snap", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "snap refresh failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }

    /// Execute batch Snap package updates using a single `snap refresh` invocation.
    ///
    /// Snap natively supports refreshing multiple packages in a single call:
    /// `snap refresh name1 name2 ...`. All items share the same success/failure
    /// status and output, since `snap refresh` handles them atomically.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all package identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["refresh".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }
        if let Some(channel) = &self.config.channel {
            args.push(format!("--channel={channel}"));
        }

        let display_args = std::iter::once("snap")
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
                "Batch updating {} Snap packages: {}\nRunning: {display_args}",
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
            "running snap refresh batch"
        );

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("snap", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        output.push_str(&cmd_output.output);
        let success = cmd_output.exit_code == 0;

        Ok(items
            .iter()
            .map(|item| {
                BatchUpdateResult::new(item.package_identifier.clone(), success, output.clone())
            })
            .collect())
    }
}
