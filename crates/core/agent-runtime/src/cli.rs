//! Maintenance subcommands shared by the standalone agent binary and the
//! controller's embedded-agent `agent` namespace.

use std::sync::Arc;

use clap::{Parser, Subcommand};
use rootcause::prelude::*;
use uptrakit_agent_core::sudoers::{
    ResolvedSudoCommand, SudoersContent, detect_is_root, ensure_docker_group_membership,
    install_helper_script, resolve_command_path, write_sudoers_file,
};
use uptrakit_command::{
    CommandExecutor, LocalCommandExecutor, LocalRemoteExecutor, RemoteExecutor,
    SudoAwareCommandExecutor, SudoContext, SudoPolicy,
};
use uptrakit_plugin_infrastructure_registry::{
    SudoCommandEntry, compatible_sudo_commands_for_host,
};
use uptrakit_shared_macros::impl_report_conversion;

/// Agent maintenance subcommands.
#[derive(Subcommand, Debug)]
pub enum AgentRuntimeCommand {
    /// Provision the local host for the agent: write the sudoers drop-in for
    /// the agent user and install plugin helper scripts. Must run as root.
    BootstrapHost(BootstrapHostArgs),
}

/// Arguments for `bootstrap-host`.
#[derive(Parser, Debug)]
pub struct BootstrapHostArgs {
    /// System user the sudoers drop-in grants commands to.
    #[arg(long, default_value = "uptrakit")]
    pub user: String,
}

/// Errors from agent maintenance subcommands.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BootstrapHostError {
    /// Command was run without root privileges.
    #[error("bootstrap-host must run as root (writes /etc/sudoers.d and /usr/local/bin)")]
    NotRoot,
    /// A provisioning step failed.
    #[error("host provisioning failed: {0}")]
    Provisioning(String),
}

/// Result alias covering every function in this module.
pub type Result<T> = std::result::Result<T, rootcause::Report<BootstrapHostError>>;

impl_report_conversion!(uptrakit_agent_core::sudoers::SudoersError => BootstrapHostError, |e| BootstrapHostError::Provisioning(e.to_string()));

/// Dispatch an [`AgentRuntimeCommand`].
pub async fn run_command(command: &AgentRuntimeCommand) -> Result<()> {
    match command {
        AgentRuntimeCommand::BootstrapHost(args) => bootstrap_host(&args.user).await,
    }
}

/// Provision the local host: resolve every plugin-declared sudo command via
/// the registry, then hand off to [`provision_host`] over the local executor.
/// Idempotent — safe to re-run on every update.
async fn bootstrap_host(user: &str) -> Result<()> {
    // Fail fast before the full plugin probe sweep. provision_host re-checks,
    // so the scripted-executor tests still cover the NotRoot path.
    if !detect_is_root(&LocalRemoteExecutor)
        .await
        .context_to::<BootstrapHostError>()?
    {
        bail!(BootstrapHostError::NotRoot);
    }

    // Probe executor for plugin compatibility checks; rootness was just
    // verified, so `is_root: true` is fact, not assumption.
    let probe: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
        Arc::new(LocalCommandExecutor),
        SudoContext {
            is_root: true,
            sudo_available: false,
            policy: SudoPolicy::default(),
        },
    ));
    let plugin_commands = compatible_sudo_commands_for_host(probe).await;
    let entries: Vec<SudoCommandEntry> = plugin_commands
        .into_iter()
        .flat_map(|(_plugin_id, entries)| entries)
        .collect();

    provision_host(&LocalRemoteExecutor, user, entries).await
}

/// Core provisioning over any [`RemoteExecutor`] (unit-tested with a scripted
/// double): require root, install helper scripts, resolve command paths, write
/// the validated sudoers drop-in, and ensure docker group membership.
async fn provision_host(
    executor: &dyn RemoteExecutor,
    user: &str,
    entries: Vec<SudoCommandEntry>,
) -> Result<()> {
    if !detect_is_root(executor)
        .await
        .context_to::<BootstrapHostError>()?
    {
        bail!(BootstrapHostError::NotRoot);
    }

    // We ARE root, so commands run directly (`privileged = false` everywhere).
    let mut resolved = Vec::new();
    let mut skipped = 0usize;
    for entry in entries {
        if let Some(helper) = &entry.helper_script {
            install_helper_script(executor, helper, false)
                .await
                .context_to::<BootstrapHostError>()?;
            resolved.push(ResolvedSudoCommand {
                command_path: helper.install_path.to_string(),
                explanation: entry.explanation.clone(),
                needs_setenv: entry.needs_setenv,
            });
        } else if let Some(path) = resolve_command_path(executor, &entry.command)
            .await
            .context_to::<BootstrapHostError>()?
        {
            let command_path = match &entry.args_suffix {
                Some(suffix) => format!("{path} {suffix}"),
                None => path,
            };
            resolved.push(ResolvedSudoCommand {
                command_path,
                explanation: entry.explanation.clone(),
                needs_setenv: entry.needs_setenv,
            });
        } else {
            skipped += 1;
            tracing::warn!(command = %entry.command, "command not found on host; skipping sudo grant");
        }
    }

    // Never write an empty drop-in: `visudo -cf` accepts a header-only file,
    // so activating one would silently wipe every existing grant. (The SSH
    // sync loop skips the write in this case; here, with no allow_all
    // fallback, an empty grant list on a compatible host is an error.)
    if resolved.is_empty() {
        bail!(BootstrapHostError::Provisioning(format!(
            "no sudo commands resolved ({skipped} skipped as not found on this host) — refusing to write an empty sudoers drop-in"
        )));
    }

    write_sudoers_file(
        executor,
        user,
        &SudoersContent::SpecificCommands(resolved),
        false,
    )
    .await
    .context_to::<BootstrapHostError>()?;
    ensure_docker_group_membership(executor, user, false)
        .await
        .context_to::<BootstrapHostError>()?;

    tracing::info!(user, "host bootstrap complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use clap::Parser;
    use uptrakit_command::{RemoteCommandResult, RemoteExecutor};
    use uptrakit_plugin_infrastructure_registry::{SudoCommandEntry, SudoHelperScript};

    use super::*;

    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(subcommand)]
        command: AgentRuntimeCommand,
    }

    #[test]
    fn bootstrap_host_user_defaults_to_uptrakit() {
        let cli = TestCli::try_parse_from(["test", "bootstrap-host"]).expect("parse");
        let AgentRuntimeCommand::BootstrapHost(args) = cli.command;
        assert_eq!(args.user, "uptrakit");
    }

    #[test]
    fn bootstrap_host_user_override() {
        let cli =
            TestCli::try_parse_from(["test", "bootstrap-host", "--user", "svc"]).expect("parse");
        let AgentRuntimeCommand::BootstrapHost(args) = cli.command;
        assert_eq!(args.user, "svc");
    }

    // Minimal local scripted double. Consolidating scripted RemoteExecutor
    // doubles into shared test support is deferred (spec Part B testing note);
    // this mirrors the shape of the double in uptrakit-agent-core's sudoers
    // tests (VecDeque of results + recorded calls behind std::sync::Mutex —
    // the approved Mutex + .unwrap() test exception applies here too).
    struct ScriptedRemoteExecutor {
        results: Mutex<VecDeque<RemoteCommandResult>>,
        calls: Mutex<Vec<String>>,
    }

    impl ScriptedRemoteExecutor {
        fn new(results: impl IntoIterator<Item = RemoteCommandResult>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn recorded_calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl RemoteExecutor for ScriptedRemoteExecutor {
        async fn exec_command(
            &self,
            command: &str,
        ) -> uptrakit_command::Result<RemoteCommandResult> {
            self.calls.lock().unwrap().push(command.to_string());
            Ok(self
                .results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(RemoteCommandResult {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 0,
                }))
        }
    }

    fn stdout_result(stdout: &str) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn err_result(stderr: &str) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code: 1,
        }
    }

    #[tokio::test]
    async fn provision_non_root_fails_with_typed_error() {
        let executor = ScriptedRemoteExecutor::new([stdout_result("1000\n")]); // id -u
        let err = provision_host(&executor, "uptrakit", Vec::new())
            .await
            .expect_err("must fail");
        // `NotRoot` is a unit variant: rootcause's `{:?}` renders the raw
        // Debug of the context (bare "NotRoot", no message text), so assert
        // against `{err}` (Display), which renders the thiserror `#[error]`
        // message via `FormattingFunction::Display`.
        assert!(format!("{err}").contains("must run as root"), "{err}");
        // Nothing beyond the root probe ran.
        assert_eq!(executor.recorded_calls().len(), 1);
    }

    #[tokio::test]
    async fn provision_installs_helper_then_writes_sudoers() {
        let helper = SudoHelperScript::new("/usr/local/bin/test-helper", "#!/bin/sh\nexit 0\n");
        let entries =
            vec![SudoCommandEntry::new("test-helper", "test helper").with_helper_script(helper)];
        let executor = ScriptedRemoteExecutor::new([
            stdout_result("0\n"),                // id -u → root
            stdout_result(""),                   // helper install (tee + chmod 755)
            stdout_result("/usr/sbin/visudo\n"), // command -v visudo
            stdout_result(""),                   // combined sudoers write+validate+move
            err_result("no docker group"),       // getent group docker → skip
        ]);
        provision_host(&executor, "uptrakit", entries)
            .await
            .expect("provision");
        let calls = executor.recorded_calls();
        // Helper install happens before the sudoers write, and the drop-in
        // grants the helper's install path.
        let helper_call = calls
            .iter()
            .position(|c| c.contains("/usr/local/bin/test-helper") && c.contains("chmod 755"))
            .expect("helper install call");
        let sudoers_call = calls
            .iter()
            .position(|c| c.contains("visudo") && c.contains(".tmp"))
            .expect("sudoers write call");
        assert!(helper_call < sudoers_call, "{calls:?}");
        assert!(
            calls[sudoers_call].contains("/usr/local/bin/test-helper"),
            "sudoers grants the helper path: {calls:?}"
        );
    }

    #[tokio::test]
    async fn provision_visudo_failure_propagates_error() {
        let entries = vec![SudoCommandEntry::new("apt-get", "package updates")];
        let executor = ScriptedRemoteExecutor::new([
            stdout_result("0\n"),                   // id -u → root
            stdout_result("/usr/bin/apt-get\n"),    // command -v apt-get
            stdout_result("/usr/sbin/visudo\n"),    // command -v visudo
            err_result("syntax error near line 3"), // compound write fails
        ]);
        let err = provision_host(&executor, "uptrakit", entries)
            .await
            .expect_err("must fail");
        assert!(format!("{err:?}").contains("syntax error"), "{err:?}");
    }

    #[tokio::test]
    async fn provision_refuses_empty_grant_list() {
        // Zero entries (or all skipped as unresolvable) must never produce a
        // header-only drop-in that would wipe existing grants on activation.
        let executor = ScriptedRemoteExecutor::new([stdout_result("0\n")]); // id -u → root
        let err = provision_host(&executor, "uptrakit", Vec::new())
            .await
            .expect_err("must fail");
        assert!(
            format!("{err:?}").contains("empty sudoers drop-in"),
            "{err:?}"
        );
        // Only the root probe ran — no write was attempted.
        assert_eq!(executor.recorded_calls().len(), 1);
    }
}
