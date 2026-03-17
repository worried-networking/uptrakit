use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_command::{CommandExecutor, CommandSpec, UpdateOutputLine};
use uptrakit_plugin_infrastructure_core::{
    PluginCapability, PreUpdateHookResult, Result, SudoCommandEntry, UpdateLifecycleContext,
    UpdateLifecyclePlugin, impl_plugin_base_config,
};

use crate::config::SystemdHookConfig;

/// Update lifecycle plugin that stops/starts a systemd service around updates.
///
/// - **Pre-hook**: runs `systemctl stop <service_name>`
/// - **Post-hook**: runs `systemctl start <service_name>` (always, even on
///   update failure, to restore service state)
pub struct SystemdHookPlugin {
    config: SystemdHookConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl SystemdHookPlugin {
    /// Compile-time capabilities declaration.
    pub const CAPABILITIES: &[PluginCapability] = &[
        PluginCapability::UpdateLifecycle,
        PluginCapability::ConfigTest,
    ];

    /// Create a new systemd hook plugin instance.
    pub async fn new(
        config: SystemdHookConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> std::result::Result<Self, String> {
        Ok(Self { config, executor })
    }

    /// Run a systemctl subcommand against the configured service.
    async fn run_systemctl(
        &self,
        action: &str,
        output_tx: &uptrakit_plugin_infrastructure_core::mpsc::Sender<UpdateOutputLine>,
    ) -> Result<()> {
        let spec = CommandSpec::exec(
            "systemctl",
            [action.to_string(), self.config.service_name.clone()],
        )
        .privileged();

        let result = self.executor.execute(&spec, output_tx).await.map_err(|e| {
            report!(
                uptrakit_plugin_infrastructure_core::PluginError::InstallFailed(format!(
                    "systemctl {action} {} failed: {e}",
                    self.config.service_name
                ))
            )
        })?;

        if result.exit_code != 0 {
            return Err(report!(
                uptrakit_plugin_infrastructure_core::PluginError::InstallFailed(format!(
                    "systemctl {action} {} exited with code {}",
                    self.config.service_name, result.exit_code
                ))
            ));
        }

        Ok(())
    }
}

impl_plugin_base_config!(SystemdHookPlugin, SystemdHookConfig, "hook_systemd", {
    fn capabilities(&self) -> Vec<PluginCapability> {
        Self::CAPABILITIES.to_vec()
    }

    fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
        vec![
            SudoCommandEntry::new("systemctl", "Stop service before update")
                .with_args_suffix("stop *"),
            SudoCommandEntry::new("systemctl", "Start service after update")
                .with_args_suffix("start *"),
        ]
    }

    fn as_update_lifecycle(&self) -> Option<&dyn UpdateLifecyclePlugin> {
        Some(self)
    }
});

#[async_trait]
impl UpdateLifecyclePlugin for SystemdHookPlugin {
    async fn execute_pre_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &uptrakit_plugin_infrastructure_core::mpsc::Sender<UpdateOutputLine>,
    ) -> Result<PreUpdateHookResult> {
        tracing::info!(
            service = %self.config.service_name,
            package = %ctx.package_identifier,
            "stopping systemd service before update"
        );

        if let Err(e) = self.run_systemctl("stop", output_tx).await {
            tracing::warn!(
                service = %self.config.service_name,
                error = %e,
                "failed to stop systemd service; aborting update"
            );
            return Ok(PreUpdateHookResult::abort(format!(
                "failed to stop systemd service {}: {e}",
                self.config.service_name
            )));
        }

        Ok(PreUpdateHookResult::proceed())
    }

    async fn execute_post_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &uptrakit_plugin_infrastructure_core::mpsc::Sender<UpdateOutputLine>,
    ) -> Result<()> {
        let succeeded = ctx.update_succeeded.unwrap_or(false);
        tracing::info!(
            service = %self.config.service_name,
            package = %ctx.package_identifier,
            update_succeeded = succeeded,
            "starting systemd service after update"
        );

        if let Err(e) = self.run_systemctl("start", output_tx).await {
            if !succeeded {
                tracing::warn!(
                    service = %self.config.service_name,
                    error = %e,
                    "failed to start service after failed update"
                );
            }
            return Err(e);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_type_id() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            let config = SystemdHookConfig {
                service_name: "nginx".to_string(),
            };
            let executor =
                Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
            let plugin = SystemdHookPlugin::new(config, executor).await.unwrap();

            use uptrakit_plugin_infrastructure_core::PluginBase;
            assert_eq!(plugin.plugin_type_id(), "hook_systemd");
            assert_eq!(
                plugin.capabilities(),
                vec![
                    PluginCapability::UpdateLifecycle,
                    PluginCapability::ConfigTest
                ]
            );
            assert!(plugin.as_update_lifecycle().is_some());
        });
    }

    #[test]
    fn required_sudo_commands_are_declared() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            let config = SystemdHookConfig {
                service_name: "nginx".to_string(),
            };
            let executor =
                Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
            let plugin = SystemdHookPlugin::new(config, executor).await.unwrap();

            use uptrakit_plugin_infrastructure_core::PluginBase;
            let cmds = plugin.required_sudo_commands();
            assert_eq!(cmds.len(), 2);
            assert_eq!(cmds[0].command, "systemctl");
            assert_eq!(cmds[1].command, "systemctl");
        });
    }
}
