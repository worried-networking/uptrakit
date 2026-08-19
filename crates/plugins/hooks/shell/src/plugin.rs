use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, HookShell, HostRequirements, HostRuntime, LifecycleHook,
    PluginFamily, PreUpdateHookResult, Result, UpdateLifecycleContext, UpdateOutputSender,
    declare_plugin,
};

use crate::config::ShellHookConfig;

/// Update lifecycle plugin that runs shell commands before/after updates.
///
/// Commands run through the host runtime's injected [`CommandExecutor`], so a
/// hook configured for an SSH-managed host executes on the target host (local
/// or SSH), not the agent's own machine.
///
/// - **Pre-hook**: runs `pre_command` if set; non-zero exit aborts the update.
/// - **Post-hook**: runs `post_command` if set; respects `on_failure` flag;
///   errors are logged but non-fatal.
pub struct ShellHookPlugin {
    config: ShellHookConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl ShellHookPlugin {
    /// Create a new shell hook plugin instance.
    pub fn new(
        config: ShellHookConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();
        Ok(Self { config, executor })
    }

    /// Run a shell hook command through the host runtime's injected
    /// [`CommandExecutor`], so it targets the correct host (local **or** SSH),
    /// never the agent's own machine.
    ///
    /// Returns `Ok(exit_code)` for any command that actually ran, including
    /// non-zero exits: the executor surfaces a non-zero exit as
    /// [`uptrakit_command::CommandError::CommandFailed`], which this method
    /// unwraps back to the code so callers decide the semantics (pre-hook abort
    /// vs. post-hook non-fatal warn).
    ///
    /// # Errors
    /// Returns [`uptrakit_plugin_infrastructure_core::PluginError::InstallFailed`]
    /// only on a genuine transport/spawn failure (SSH unreachable, unsupported
    /// shell, or a `NoopCommandExecutor` `UnsupportedOperation`) — never for a
    /// hook that merely exited non-zero.
    async fn run_shell_command(
        &self,
        command: &str,
        shell: HookShell,
        output_tx: &UpdateOutputSender,
    ) -> Result<i32> {
        let spec = CommandSpec::shell_with(command, shell);
        match self.executor.execute(&spec, output_tx).await {
            Ok(output) => {
                tracing::debug!(
                    exit_code = output.exit_code,
                    output_len = output.output.len(),
                    "shell hook completed"
                );
                Ok(output.exit_code)
            }
            Err(e) => {
                // A command that ran but exited non-zero comes back as
                // CommandFailed(code); unwrap it so callers branch on the code.
                if let uptrakit_command::CommandError::CommandFailed(code) = e.current_context() {
                    return Ok(*code);
                }
                // Anything else is a real transport/spawn failure.
                Err(report!(
                    uptrakit_plugin_infrastructure_core::PluginError::InstallFailed(format!(
                        "shell hook command failed: {e}"
                    ))
                ))
            }
        }
    }
}

declare_plugin!(ShellHookPlugin, ShellHookConfig, "hook.shell", {
    display_name: "Shell Hook",
    family: PluginFamily::Hook,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::UpdateCommandValidation, ConfigTestKind::PreUpdateHook, ConfigTestKind::PostUpdateHook],
    roles: [LifecycleHook],
});

#[async_trait]
impl LifecycleHook for ShellHookPlugin {
    async fn execute_pre_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &UpdateOutputSender,
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

        let exit_code = self
            .run_shell_command(cmd, self.config.shell, output_tx)
            .await?;

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
        output_tx: &UpdateOutputSender,
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

        let exit_code = self
            .run_shell_command(cmd, self.config.shell, output_tx)
            .await?;

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
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;
    use uptrakit_command::CommandMode;
    use uptrakit_plugin_infrastructure_core::mpsc;
    use uptrakit_plugin_infrastructure_core::testing::RecordingExecutor;
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, PluginCapability, PluginMeta, StandardHostRuntime,
    };

    /// Helper to create a ShellHookPlugin for testing.
    fn test_plugin(config: ShellHookConfig) -> ShellHookPlugin {
        let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        ShellHookPlugin::new(config, runtime).unwrap()
    }

    /// Helper to create a ShellHookPlugin wired to a [`RecordingExecutor`]
    /// double, so tests can assert on the recorded `CommandSpec`s.
    fn plugin_with_double(
        config: ShellHookConfig,
        double: Arc<RecordingExecutor>,
    ) -> ShellHookPlugin {
        let runtime = Arc::new(StandardHostRuntime::new(
            double as Arc<dyn CommandExecutor>,
            HostCapabilities::default(),
        )) as Arc<dyn HostRuntime>;
        ShellHookPlugin::new(config, runtime).unwrap()
    }

    #[test]
    fn plugin_type_id() {
        let plugin = test_plugin(ShellHookConfig {
            pre_command: Some("echo pre".to_string()),
            post_command: None,
            on_failure: true,
            shell: HookShell::Bash,
        });

        assert_eq!(plugin.plugin_type_id().as_str(), "hook.shell");
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
        assert!(DESCRIPTOR.roles.version_detector.is_none());
    }

    #[tokio::test]
    async fn pre_hook_succeeds_with_echo() {
        let plugin = test_plugin(ShellHookConfig {
            pre_command: Some("echo 'pre-hook ran'".to_string()),
            post_command: None,
            on_failure: true,
            shell: HookShell::Bash,
        });
        let (tx, mut rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("test", "1.0", None, None);
        let result = plugin.execute_pre_hook(&ctx, &tx).await.unwrap();
        assert!(result.should_proceed);
        assert!(
            rx.try_recv().is_ok(),
            "hook output must still stream to output_tx"
        );
    }

    #[tokio::test]
    async fn pre_hook_aborts_on_failure() {
        let plugin = test_plugin(ShellHookConfig {
            pre_command: Some("exit 1".to_string()),
            post_command: None,
            on_failure: true,
            shell: HookShell::Bash,
        });
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("test", "1.0", None, None);
        let result = plugin.execute_pre_hook(&ctx, &tx).await.unwrap();
        assert!(!result.should_proceed);
    }

    #[tokio::test]
    async fn pre_hook_skipped_when_empty() {
        let plugin = test_plugin(ShellHookConfig {
            pre_command: None,
            post_command: Some("echo post".to_string()),
            on_failure: true,
            shell: HookShell::Bash,
        });
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("test", "1.0", None, None);
        let result = plugin.execute_pre_hook(&ctx, &tx).await.unwrap();
        assert!(result.should_proceed);
    }

    #[tokio::test]
    async fn post_hook_skipped_on_failure_when_disabled() {
        let plugin = test_plugin(ShellHookConfig {
            pre_command: None,
            post_command: Some("echo should-not-run".to_string()),
            on_failure: false,
            shell: HookShell::Bash,
        });
        let (tx, mut rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_post_hook("test", "1.0", None, None, false);
        plugin.execute_post_hook(&ctx, &tx).await.unwrap();
        // Channel should be empty since the hook was skipped
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn pre_hook_routes_command_through_injected_executor() {
        let double = RecordingExecutor::ok(0);
        let plugin = plugin_with_double(
            ShellHookConfig {
                pre_command: Some("echo routed".to_string()),
                post_command: None,
                on_failure: true,
                shell: HookShell::Bash,
            },
            double.clone(),
        );
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("test", "1.0", None, None);

        let result = plugin.execute_pre_hook(&ctx, &tx).await.unwrap();
        assert!(result.should_proceed);

        let specs = double.recorded();
        assert_eq!(
            specs.len(),
            1,
            "hook must route exactly one command through the executor"
        );
        assert!(
            matches!(&specs[0].mode, CommandMode::Shell { command, shell }
                if command == "echo routed" && *shell == HookShell::Bash),
            "recorded spec must be Shell mode carrying the configured command + shell",
        );
    }

    #[tokio::test]
    async fn post_hook_routes_command_through_injected_executor() {
        let double = RecordingExecutor::ok(0);
        let plugin = plugin_with_double(
            ShellHookConfig {
                pre_command: None,
                post_command: Some("echo routed".to_string()),
                on_failure: true,
                shell: HookShell::Bash,
            },
            double.clone(),
        );
        let (tx, _rx) = mpsc::channel(100);
        // update_succeeded = true so the post-hook runs.
        let ctx = UpdateLifecycleContext::for_post_hook("test", "1.0", None, None, true);

        plugin.execute_post_hook(&ctx, &tx).await.unwrap();

        let specs = double.recorded();
        assert_eq!(specs.len(), 1, "post-hook must route through the executor");
        assert!(
            matches!(&specs[0].mode, CommandMode::Shell { command, shell }
                if command == "echo routed" && *shell == HookShell::Bash),
            "recorded spec must be Shell mode carrying the configured command + shell",
        );
    }

    #[tokio::test]
    async fn non_zero_exit_is_extracted_not_hard_failed() {
        // A hook that runs and exits non-zero is a graceful abort, not an error:
        // the executor reports it as Err(CommandFailed(1)), which the plugin
        // unwraps back to the code.
        let plugin = plugin_with_double(
            ShellHookConfig {
                pre_command: Some("echo routed".to_string()),
                post_command: None,
                on_failure: true,
                shell: HookShell::Bash,
            },
            RecordingExecutor::failed(1),
        );
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("test", "1.0", None, None);

        let result = plugin.execute_pre_hook(&ctx, &tx).await.unwrap();
        assert!(
            !result.should_proceed,
            "non-zero pre-hook exit must abort gracefully, not Err"
        );
    }

    #[tokio::test]
    async fn post_hook_non_zero_exit_is_non_fatal() {
        // A non-zero post-hook exit is non-fatal: it warns and still returns
        // Ok(()). The recorded() assert pins that the command reached the
        // executor rather than the plugin short-circuiting.
        let double = RecordingExecutor::failed(1);
        let plugin = plugin_with_double(
            ShellHookConfig {
                pre_command: None,
                post_command: Some("echo routed".to_string()),
                on_failure: true,
                shell: HookShell::Bash,
            },
            double.clone(),
        );
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_post_hook("test", "1.0", None, None, true);

        // Must be Ok(()) despite the non-zero exit.
        plugin.execute_post_hook(&ctx, &tx).await.unwrap();
        assert_eq!(
            double.recorded().len(),
            1,
            "post-hook must reach the executor"
        );
    }

    /// Drive a pre-hook through a double that fails with `error`, and assert the
    /// plugin surfaces it as an `Err` rather than a silent success.
    async fn assert_transport_error_surfaces(double: Arc<RecordingExecutor>) {
        let plugin = plugin_with_double(
            ShellHookConfig {
                pre_command: Some("echo routed".to_string()),
                post_command: None,
                on_failure: true,
                shell: HookShell::Bash,
            },
            double,
        );
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("test", "1.0", None, None);

        let result = plugin.execute_pre_hook(&ctx, &tx).await;
        assert!(
            result.is_err(),
            "genuine transport error must surface as PluginError"
        );
    }

    #[tokio::test]
    async fn transport_error_unsupported_shell_surfaces_as_plugin_error() {
        // A real SSH/shell failure must not be mistaken for a hook that ran.
        assert_transport_error_surfaces(RecordingExecutor::erroring(|| {
            Err(report!(uptrakit_command::CommandError::UnsupportedShell(
                "test shell unsupported".to_string(),
            )))
        }))
        .await;
    }

    #[tokio::test]
    async fn transport_error_noop_executor_surfaces_as_plugin_error() {
        // UnsupportedOperation is what a NoopCommandExecutor returns: a Noop
        // leaking into a hook path must fail loudly, never pass silently.
        assert_transport_error_surfaces(RecordingExecutor::erroring(|| {
            Err(report!(
                uptrakit_command::CommandError::UnsupportedOperation("noop executor".to_string()),
            ))
        }))
        .await;
    }
}
