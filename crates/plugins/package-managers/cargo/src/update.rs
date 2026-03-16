use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    OutputStreamType, PluginError, ReleaseInfo, Result, UpdateOutputLine,
};

use crate::plugin::CargoPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin for CargoPlugin {
    /// Execute a `cargo install` update for a single crate.
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
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

        let display_cmd = if self.config.use_locked {
            format!(
                "cargo install {} --version {} --locked",
                package_identifier, to_version
            )
        } else {
            format!(
                "cargo install {} --version {}",
                package_identifier, to_version
            )
        };
        tracing::debug!(
            package = %package_identifier,
            to_version = %to_version,
            "running cargo install"
        );

        send_output(
            output_tx,
            &format!("Updating {package_identifier} to {to_version}\nRunning: {display_cmd}"),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_cmd}\n");

        // No `.privileged()` -- cargo install does not require sudo.
        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("cargo", args), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "cargo install failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }
}
