use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_command::{CommandExecutor, UpdateOutputLine};
use uptrakit_plugin_infrastructure_core::{
    HookShell, PluginCapability, PreUpdateHookResult, Result, UpdateLifecycleContext,
    UpdateLifecyclePlugin, impl_plugin_base_config,
};

use crate::config::ShellHookConfig;

/// Update lifecycle plugin that runs shell commands before/after updates.
///
/// - **Pre-hook**: runs `pre_command` if set; non-zero exit aborts the update.
/// - **Post-hook**: runs `post_command` if set; respects `on_failure` flag;
///   errors are logged but non-fatal.
pub struct ShellHookPlugin {
    config: ShellHookConfig,
    #[allow(dead_code)]
    executor: Arc<dyn CommandExecutor>,
}

impl ShellHookPlugin {
    /// Compile-time capabilities declaration.
    pub const CAPABILITIES: &[PluginCapability] = &[
        PluginCapability::UpdateLifecycle,
        PluginCapability::ConfigTest,
    ];

    /// Create a new shell hook plugin instance.
    pub async fn new(
        config: ShellHookConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> std::result::Result<Self, String> {
        Ok(Self { config, executor })
    }
}

impl_plugin_base_config!(ShellHookPlugin, ShellHookConfig, "hook_shell", {
    fn capabilities(&self) -> Vec<PluginCapability> {
        Self::CAPABILITIES.to_vec()
    }

    fn as_update_lifecycle(&self) -> Option<&dyn UpdateLifecyclePlugin> {
        Some(self)
    }
});

/// Run a shell command via the `run_command_with_shell` utility.
///
/// Returns `Ok(exit_code)` on success. On non-zero exit, the command crate
/// returns `CommandError::CommandFailed(exit_code)` — we extract and return
/// the exit code to let callers decide the semantics (abort vs. warn).
async fn run_shell_command(
    command: &str,
    shell: HookShell,
    output_tx: &uptrakit_plugin_infrastructure_core::mpsc::Sender<UpdateOutputLine>,
) -> Result<i32> {
    match uptrakit_command::run_command_with_shell(command, shell, output_tx).await {
        Ok((output, exit_code)) => {
            tracing::debug!(exit_code, output_len = output.len(), "shell hook completed");
            Ok(exit_code)
        }
        Err(e) => {
            // Extract exit code from CommandFailed if possible
            if let uptrakit_command::CommandError::CommandFailed(code) = e.current_context() {
                return Ok(*code);
            }
            Err(rootcause::report!(
                uptrakit_plugin_infrastructure_core::PluginError::InstallFailed(format!(
                    "shell hook command failed: {e}"
                ))
            ))
        }
    }
}

#[async_trait]
impl UpdateLifecyclePlugin for ShellHookPlugin {
    async fn execute_pre_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &uptrakit_plugin_infrastructure_core::mpsc::Sender<UpdateOutputLine>,
    ) -> Result<PreUpdateHookResult> {
        let Some(cmd) = self
            .config
            .pre_command
            .as_deref()
            .filter(|c| !c.trim().is_empty())
        else {
            return Ok(PreUpdateHookResult::proceed());
        };

        tracing::info!(
            command = %cmd,
            package = %ctx.package_identifier,
            "running shell pre-update hook"
        );

        let exit_code = run_shell_command(cmd, self.config.shell, output_tx).await?;

        if exit_code != 0 {
            return Ok(PreUpdateHookResult::abort(format!(
                "shell pre-update hook exited with code {exit_code}"
            )));
        }

        Ok(PreUpdateHookResult::proceed())
    }

    async fn execute_post_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &uptrakit_plugin_infrastructure_core::mpsc::Sender<UpdateOutputLine>,
    ) -> Result<()> {
        let Some(cmd) = self
            .config
            .post_command
            .as_deref()
            .filter(|c| !c.trim().is_empty())
        else {
            return Ok(());
        };

        let update_succeeded = ctx.update_succeeded.unwrap_or(false);
        if !update_succeeded && !self.config.on_failure {
            tracing::debug!(
                command = %cmd,
                "skipping shell post-update hook (update failed, on_failure=false)"
            );
            return Ok(());
        }

        tracing::info!(
            command = %cmd,
            package = %ctx.package_identifier,
            update_succeeded,
            "running shell post-update hook"
        );

        let exit_code = run_shell_command(cmd, self.config.shell, output_tx).await?;

        if exit_code != 0 {
            tracing::warn!(
                command = %cmd,
                exit_code,
                "shell post-update hook exited with non-zero code (non-fatal)"
            );
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
            let config = ShellHookConfig {
                pre_command: Some("echo pre".to_string()),
                post_command: None,
                on_failure: true,
                shell: HookShell::Bash,
            };
            let executor =
                Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
            let plugin = ShellHookPlugin::new(config, executor).await.unwrap();

            use uptrakit_plugin_infrastructure_core::PluginBase;
            assert_eq!(plugin.plugin_type_id(), "hook_shell");
            assert_eq!(
                plugin.capabilities(),
                vec![PluginCapability::UpdateLifecycle]
            );
            assert!(plugin.as_update_lifecycle().is_some());
        });
    }

    #[test]
    fn no_sudo_commands_needed() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            let config = ShellHookConfig {
                pre_command: Some("echo pre".to_string()),
                post_command: None,
                on_failure: true,
                shell: HookShell::Bash,
            };
            let executor =
                Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
            let plugin = ShellHookPlugin::new(config, executor).await.unwrap();

            use uptrakit_plugin_infrastructure_core::PluginBase;
            assert!(plugin.required_sudo_commands().is_empty());
        });
    }

    #[tokio::test]
    async fn pre_hook_succeeds_with_echo() {
        let config = ShellHookConfig {
            pre_command: Some("echo 'pre-hook ran'".to_string()),
            post_command: None,
            on_failure: true,
            shell: HookShell::Bash,
        };
        let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = ShellHookPlugin::new(config, executor).await.unwrap();
        let (tx, _rx) = uptrakit_plugin_infrastructure_core::mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("test", "1.0", None, None);
        let result = plugin.execute_pre_hook(&ctx, &tx).await.unwrap();
        assert!(result.should_proceed);
    }

    #[tokio::test]
    async fn pre_hook_aborts_on_failure() {
        let config = ShellHookConfig {
            pre_command: Some("exit 1".to_string()),
            post_command: None,
            on_failure: true,
            shell: HookShell::Bash,
        };
        let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = ShellHookPlugin::new(config, executor).await.unwrap();
        let (tx, _rx) = uptrakit_plugin_infrastructure_core::mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("test", "1.0", None, None);
        let result = plugin.execute_pre_hook(&ctx, &tx).await.unwrap();
        assert!(!result.should_proceed);
    }

    #[tokio::test]
    async fn pre_hook_skipped_when_empty() {
        let config = ShellHookConfig {
            pre_command: None,
            post_command: Some("echo post".to_string()),
            on_failure: true,
            shell: HookShell::Bash,
        };
        let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = ShellHookPlugin::new(config, executor).await.unwrap();
        let (tx, _rx) = uptrakit_plugin_infrastructure_core::mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("test", "1.0", None, None);
        let result = plugin.execute_pre_hook(&ctx, &tx).await.unwrap();
        assert!(result.should_proceed);
    }

    #[tokio::test]
    async fn post_hook_skipped_on_failure_when_disabled() {
        let config = ShellHookConfig {
            pre_command: None,
            post_command: Some("echo should-not-run".to_string()),
            on_failure: false,
            shell: HookShell::Bash,
        };
        let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = ShellHookPlugin::new(config, executor).await.unwrap();
        let (tx, mut rx) = uptrakit_plugin_infrastructure_core::mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_post_hook("test", "1.0", None, None, false);
        plugin.execute_post_hook(&ctx, &tx).await.unwrap();
        // Channel should be empty since the hook was skipped
        assert!(rx.try_recv().is_err());
    }
}
