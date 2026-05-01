use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::send_output;
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    ExecuteUpdateResult, OutputStreamType, ReleaseInfo, Result, UpdateOutputLine,
};

use crate::plugin::CargoPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for CargoPlugin {
    /// Execute a `cargo install` update for a single crate.
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<ExecuteUpdateResult> {
        self.require_package_identifier(package_identifier)?;

        let mut args = vec![
            "install".to_string(),
            package_identifier.to_string(),
            "--version".to_string(),
            to_version.to_string(),
        ];
        if self.config.use_locked {
            args.push("--locked".to_string());
        }

        tracing::debug!(
            package = %package_identifier,
            to_version = %to_version,
            "running cargo install"
        );

        send_output(
            output_tx,
            &format!("Updating {package_identifier} to {to_version}"),
            OutputStreamType::Stdout,
        )
        .await;

        let output = uptrakit_plugin_infrastructure_core::execute_command_update(
            uptrakit_plugin_infrastructure_core::CommandUpdateParams {
                executor: self.executor.as_ref(),
                binary: "cargo",
                args,
                privileged: false,
                spec_modifier: None,
                exit_code_success: None,
                exit_code_error: None,
            },
            output_tx,
        )
        .await?;
        Ok(ExecuteUpdateResult::new(output, false))
    }
}
