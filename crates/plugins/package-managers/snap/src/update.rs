use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::send_output;
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchUpdateItem, BatchUpdateResult, OutputStreamType, ReleaseInfo, Result, UpdateOutputLine,
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

        send_output(
            output_tx,
            &format!("Updating {package_identifier} to {to_version}"),
            OutputStreamType::Stdout,
        )
        .await;

        uptrakit_plugin_infrastructure_core::execute_command_update(
            uptrakit_plugin_infrastructure_core::CommandUpdateParams {
                executor: self.executor.as_ref(),
                binary: "snap",
                args,
                privileged: true,
                spec_modifier: None,
                exit_code_success: None,
                exit_code_error: None,
            },
            output_tx,
        )
        .await
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
        let pkg_list: Vec<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();
        let context_prefix = if items.is_empty() {
            None
        } else {
            Some(format!(
                "Batch updating {} Snap packages: {}",
                items.len(),
                pkg_list.join(", ")
            ))
        };
        let suffix_args = self
            .config
            .channel
            .as_ref()
            .map(|c| vec![format!("--channel={c}")])
            .unwrap_or_default();
        tracing::debug!(
            count = items.len(),
            packages = ?pkg_list,
            "running snap refresh batch"
        );
        uptrakit_plugin_infrastructure_core::execute_batch_names_command(
            uptrakit_plugin_infrastructure_core::BatchNamesParams {
                executor: self.executor.as_ref(),
                binary: "snap",
                prefix_args: vec!["refresh".to_string()],
                privileged: true,
                suffix_args,
                validate_identifier,
                validate_version: None,
                context_prefix,
            },
            items,
            output_tx,
        )
        .await
    }
}
