use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, HostRequirements, HostRuntime, LifecycleHook, OsFamily,
    PluginFamily, PreUpdateHookResult, Result, SudoCommandEntry, UpdateLifecycleContext,
    UpdateOutputSender, declare_plugin, host_features,
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
    /// Create a new systemd hook plugin instance.
    pub fn new(
        config: SystemdHookConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();
        Ok(Self { config, executor })
    }

    /// Run a systemctl subcommand against the configured service.
    async fn run_systemctl(&self, action: &str, output_tx: &UpdateOutputSender) -> Result<()> {
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

    /// Sudo commands required by this plugin (static — uses wildcard for service names).
    fn required_sudo_commands(_config: &serde_json::Value) -> Vec<SudoCommandEntry> {
        vec![
            SudoCommandEntry::new("systemctl", "Stop service before update")
                .with_args_suffix("stop *"),
            SudoCommandEntry::new("systemctl", "Start service after update")
                .with_args_suffix("start *"),
        ]
    }
}

/// Required features for systemd hook plugin. Static to avoid const-eval
/// destructor limitation with `Cow<'static, str>` inside `HostFeature`.
static REQUIRED_FEATURES: [uptrakit_plugin_infrastructure_core::HostFeature; 2] =
    [host_features::POSIX_SHELL, host_features::SYSTEMD];

declare_plugin!(SystemdHookPlugin, SystemdHookConfig, "hook_systemd", {
    display_name: "Systemd Hook",
    family: PluginFamily::Hook,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::new(
        &[OsFamily::Linux],
        &REQUIRED_FEATURES,
        false,
    ),
    config_test: [ConfigTestKind::PreUpdateHook, ConfigTestKind::PostUpdateHook],
    roles: [LifecycleHook],
    sudo: SystemdHookPlugin::required_sudo_commands,
});

#[async_trait]
impl LifecycleHook for SystemdHookPlugin {
    async fn execute_pre_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &UpdateOutputSender,
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
        output_tx: &UpdateOutputSender,
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
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, PluginCapability, PluginMeta, PosixHostRuntime,
    };

    /// Helper to create a SystemdHookPlugin for testing.
    fn test_plugin(config: SystemdHookConfig) -> SystemdHookPlugin {
        let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let caps = HostCapabilities::default();
        let runtime = Arc::new(PosixHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        SystemdHookPlugin::new(config, runtime).unwrap()
    }

    #[test]
    fn plugin_type_id() {
        let plugin = test_plugin(SystemdHookConfig {
            service_name: "nginx".to_string(),
        });
        assert_eq!(plugin.plugin_type_id().as_str(), "hook_systemd");
    }

    #[test]
    fn descriptor_capabilities() {
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::UpdateLifecycle)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ConfigTest)
        );
    }

    #[test]
    fn descriptor_has_lifecycle_hook_role() {
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_some());
        assert!(DESCRIPTOR.roles.discoverer.is_none());
    }

    #[test]
    fn descriptor_has_sudo() {
        assert!(DESCRIPTOR.sudo.is_some());
        let cmds = (DESCRIPTOR.sudo.unwrap())(&serde_json::json!({}));
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].command, "systemctl");
        assert_eq!(cmds[1].command, "systemctl");
    }
}
