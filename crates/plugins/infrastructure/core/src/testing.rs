//! Shared mock [`CommandExecutor`] implementations for plugin unit tests.
//!
//! Enable the `testing` Cargo feature to use these types in `[dev-dependencies]`:
//!
//! ```toml
//! [dev-dependencies]
//! uptrakit-plugin-infrastructure-core = { workspace = true, features = ["testing"] }
//! ```
//!
//! Then bring them into scope inside a test module:
//!
//! ```rust,ignore
//! use uptrakit_plugin_infrastructure_core::testing::{FixedOutputExecutor, RoutedOutputExecutor};
//! ```

use std::sync::Arc;

use async_trait::async_trait;

use crate::host_runtime::{HostRuntime, PosixHostRuntime};
use crate::{CommandExecutor, CommandOutput, CommandSpec, UpdateOutputLine};
use uptrakit_shared_types::HostCapabilities;

/// Returns the same output and exit code for every command.
///
/// - [`execute`]: always returns `Ok(CommandOutput { output, exit_code })`
/// - [`execute_quiet`]: returns `Ok(...)` when `exit_code == 0`;
///   returns `Err(CommandError::CommandFailed(code))` otherwise
///
/// [`execute`]: CommandExecutor::execute
/// [`execute_quiet`]: CommandExecutor::execute_quiet
pub struct FixedOutputExecutor {
    output: String,
    exit_code: i32,
}

impl FixedOutputExecutor {
    /// Returns successful execution (exit code 0) with the given stdout.
    pub fn success(output: impl Into<String>) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            output: output.into(),
            exit_code: 0,
        })
    }

    /// Returns empty output with the given exit code.
    ///
    /// `execute` returns `Ok` with that code; `execute_quiet` returns
    /// `Err(CommandFailed)` for any non-zero code.
    pub fn failure(exit_code: i32) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            output: String::new(),
            exit_code,
        })
    }

    /// Returns the given output and exit code.
    ///
    /// Equivalent to [`success`] when `exit_code == 0`.
    ///
    /// [`success`]: FixedOutputExecutor::success
    #[allow(clippy::new_ret_no_self)]
    pub fn new(output: impl Into<String>, exit_code: i32) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            output: output.into(),
            exit_code,
        })
    }
}

#[async_trait]
impl CommandExecutor for FixedOutputExecutor {
    async fn execute(
        &self,
        _spec: &CommandSpec,
        _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_command::Result<CommandOutput> {
        Ok(CommandOutput {
            output: self.output.clone(),
            exit_code: self.exit_code,
        })
    }

    async fn execute_quiet(&self, _spec: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        if self.exit_code == 0 {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: 0,
            })
        } else {
            use rootcause::prelude::*;
            bail!(uptrakit_command::CommandError::CommandFailed(
                self.exit_code
            ))
        }
    }
}

/// Routes `execute` and `execute_quiet` calls by the command program name.
///
/// Unrecognised programs (and shell-mode specs) return empty output with
/// exit code 0. Both methods always return `Ok(CommandOutput { ... })`
/// regardless of exit code — use this when the plugin logic inspects
/// `exit_code` directly rather than relying on the executor to propagate
/// failures.
///
/// # Construction
///
/// ```rust,ignore
/// // All routes succeed (exit code 0):
/// let exec = RoutedOutputExecutor::success([("rpm", rpm_output)]);
///
/// // Per-route exit codes:
/// let exec = RoutedOutputExecutor::new([("dnf", output, 100)]);
/// ```
pub struct RoutedOutputExecutor {
    routes: Vec<(&'static str, String, i32)>,
}

impl RoutedOutputExecutor {
    /// Build from `(program, stdout)` pairs; all routes return exit code 0.
    pub fn success(
        routes: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            routes: routes
                .into_iter()
                .map(|(p, o)| (p, o.to_string(), 0))
                .collect(),
        })
    }

    /// Build from `(program, stdout, exit_code)` triples.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        routes: impl IntoIterator<Item = (&'static str, &'static str, i32)>,
    ) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            routes: routes
                .into_iter()
                .map(|(p, o, c)| (p, o.to_string(), c))
                .collect(),
        })
    }

    fn lookup(&self, spec: &CommandSpec) -> (String, i32) {
        if let crate::CommandMode::Exec { program, .. } = &spec.mode {
            for (name, out, code) in &self.routes {
                if program == *name {
                    return (out.clone(), *code);
                }
            }
        }
        (String::new(), 0)
    }
}

#[async_trait]
impl CommandExecutor for RoutedOutputExecutor {
    async fn execute(
        &self,
        spec: &CommandSpec,
        _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_command::Result<CommandOutput> {
        let (output, exit_code) = self.lookup(spec);
        Ok(CommandOutput { output, exit_code })
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        let (output, exit_code) = self.lookup(spec);
        Ok(CommandOutput { output, exit_code })
    }
}

/// Build a [`HostRuntime`] backed by [`LocalCommandExecutor`] for unit tests.
///
/// Use this when the test needs a real executor (e.g., tests that test
/// host-compatibility detection against the actual host environment).
pub fn test_runtime() -> Arc<dyn HostRuntime> {
    Arc::new(PosixHostRuntime::new(
        Arc::new(crate::LocalCommandExecutor),
        HostCapabilities::default(),
    ))
}

/// Build a [`HostRuntime`] backed by the provided executor for unit tests.
///
/// Use this in the majority of unit tests where you control command output via
/// [`FixedOutputExecutor`] or [`RoutedOutputExecutor`].
pub fn test_runtime_with_executor(executor: Arc<dyn CommandExecutor>) -> Arc<dyn HostRuntime> {
    Arc::new(PosixHostRuntime::new(executor, HostCapabilities::default()))
}
