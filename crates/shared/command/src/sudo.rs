//! Sudo-aware command execution.
//!
//! Provides [`SudoPolicy`], [`SudoContext`], and [`SudoAwareCommandExecutor`]
//! for runtime privilege escalation decisions. Instead of hard-coding `sudo`
//! in plugin [`CommandSpec`]s, plugins mark commands as
//! [`CommandSpec::privileged`] and the executor decides at runtime whether to
//! prepend `sudo` based on the detected host context.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::executor::{CommandExecutor, CommandMode, CommandOutput, CommandSpec};
use crate::types::UpdateOutputLine;

// ── SudoPolicy ─────────────────────────────────────────────────────────

/// Error returned when a [`SudoPolicy`] string cannot be parsed.
#[derive(Debug, Error)]
#[error("invalid sudo policy '{0}': expected 'auto', 'force_with', or 'force_without'")]
pub struct ParseSudoPolicyError(String);

/// Determines how privilege escalation is applied when executing privileged commands.
///
/// The default is [`SudoPolicy::Auto`], which mirrors the old hard-coded behaviour
/// (prepend `sudo` when the user is non-root and passwordless sudo is available).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SudoPolicy {
    /// Prepend `sudo` only when the user is non-root **and** passwordless sudo
    /// is available on the host. This is the backward-compatible default.
    #[default]
    Auto,
    /// Always prepend `sudo` for privileged commands, unless the current user
    /// is already root (running sudo as root is redundant and can cause issues).
    ForceWith,
    /// Never prepend `sudo`, even for privileged commands. Useful when the
    /// agent user is already root or runs in a privileged container.
    ForceWithout,
}

impl fmt::Display for SudoPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::ForceWith => "force_with",
            Self::ForceWithout => "force_without",
        })
    }
}

impl FromStr for SudoPolicy {
    type Err = ParseSudoPolicyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "force_with" => Ok(Self::ForceWith),
            "force_without" => Ok(Self::ForceWithout),
            _ => Err(ParseSudoPolicyError(s.to_string())),
        }
    }
}

// ── SudoContext ─────────────────────────────────────────────────────────

/// Runtime context used to decide whether to prepend `sudo` to privileged commands.
///
/// Created from the SSH host's database fields (populated during bootstrap or
/// `update-sudoers`) and passed to [`SudoAwareCommandExecutor`]. The local
/// agent uses [`SudoContext::default()`] which encodes the assumption that the
/// agent user is non-root with passwordless sudo available — matching the old
/// hard-coded `sudo` behaviour.
pub struct SudoContext {
    /// Whether the agent user is UID 0 on the target host.
    pub is_root: bool,
    /// Whether passwordless sudo (`sudo -n true`) succeeds on the target host.
    pub sudo_available: bool,
    /// The override policy controlling sudo usage.
    pub policy: SudoPolicy,
}

impl SudoContext {
    /// Returns `true` if `sudo` should be prepended for a privileged command.
    ///
    /// | Policy          | is_root=true | is_root=false & sudo=true | is_root=false & sudo=false |
    /// |:--------------- |:------------:|:-------------------------:|:--------------------------:|
    /// | Auto            | false        | **true**                  | false                      |
    /// | ForceWith       | false        | **true**                  | **true**                   |
    /// | ForceWithout    | false        | false                     | false                      |
    pub fn should_use_sudo(&self) -> bool {
        match self.policy {
            SudoPolicy::ForceWithout => false,
            SudoPolicy::ForceWith => !self.is_root,
            SudoPolicy::Auto => !self.is_root && self.sudo_available,
        }
    }
}

impl Default for SudoContext {
    /// Non-root user with passwordless sudo in auto mode.
    ///
    /// This matches the old hard-coded `sudo` behaviour: the local agent
    /// assumes its user has passwordless sudo configured (as set up during
    /// system installation). SSH-backed agents replace this with values loaded
    /// from the host database.
    fn default() -> Self {
        Self {
            is_root: false,
            sudo_available: true,
            policy: SudoPolicy::Auto,
        }
    }
}

// ── SudoAwareCommandExecutor ────────────────────────────────────────────

/// Wraps an inner [`CommandExecutor`] and conditionally prepends `sudo` to
/// [`CommandSpec::privileged`] commands based on the active [`SudoContext`].
///
/// # Shell mode
///
/// Shell-mode commands are always forwarded unchanged, even when `privileged`
/// is set. Shell commands used for update hooks must handle their own privilege
/// escalation. A `tracing::warn!` is emitted when a privileged Shell-mode
/// command is encountered.
pub struct SudoAwareCommandExecutor {
    inner: Arc<dyn CommandExecutor>,
    context: SudoContext,
}

impl SudoAwareCommandExecutor {
    /// Create a new executor wrapping `inner` with the given sudo `context`.
    pub fn new(inner: Arc<dyn CommandExecutor>, context: SudoContext) -> Self {
        Self { inner, context }
    }

    /// Apply sudo transformation to a spec if needed.
    ///
    /// Returns a clone of `spec` with `sudo` prepended when:
    /// - `spec.privileged` is `true`, **and**
    /// - [`SudoContext::should_use_sudo`] returns `true`, **and**
    /// - `spec.mode` is [`CommandMode::Exec`].
    ///
    /// Shell-mode specs are always returned unchanged (with a warning).
    fn apply_sudo(&self, spec: &CommandSpec) -> CommandSpec {
        if !spec.privileged || !self.context.should_use_sudo() {
            return spec.clone();
        }

        match &spec.mode {
            CommandMode::Exec { program, args } => {
                let mut new_args = Vec::with_capacity(1 + args.len());
                new_args.push(program.clone());
                new_args.extend(args.iter().cloned());
                CommandSpec {
                    mode: CommandMode::Exec {
                        program: "sudo".to_string(),
                        args: new_args,
                    },
                    working_dir: spec.working_dir.clone(),
                    timeout: spec.timeout,
                    privileged: false,
                }
            }
            CommandMode::Shell { .. } => {
                tracing::warn!(
                    "CommandSpec::privileged has no effect on Shell mode; \
                     Shell commands handle their own privilege escalation"
                );
                spec.clone()
            }
        }
    }
}

#[async_trait]
impl CommandExecutor for SudoAwareCommandExecutor {
    async fn execute(
        &self,
        spec: &CommandSpec,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> crate::Result<CommandOutput> {
        let modified = self.apply_sudo(spec);
        self.inner.execute(&modified, output_tx).await
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> crate::Result<CommandOutput> {
        let modified = self.apply_sudo(spec);
        self.inner.execute_quiet(&modified).await
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::LocalCommandExecutor;

    // ── SudoPolicy FromStr / Display round-trip ─────────────────────────

    #[test]
    fn sudo_policy_round_trip() {
        for (s, expected) in [
            ("auto", SudoPolicy::Auto),
            ("force_with", SudoPolicy::ForceWith),
            ("force_without", SudoPolicy::ForceWithout),
        ] {
            let parsed: SudoPolicy = s.parse().expect("should parse");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), s);
        }
    }

    #[test]
    fn sudo_policy_invalid_string_errors() {
        let err = "invalid_policy".parse::<SudoPolicy>().unwrap_err();
        assert!(err.to_string().contains("invalid_policy"));
    }

    #[test]
    fn sudo_policy_default_is_auto() {
        assert_eq!(SudoPolicy::default(), SudoPolicy::Auto);
    }

    // ── SudoContext::should_use_sudo ────────────────────────────────────

    #[test]
    fn should_use_sudo_force_without_always_false() {
        for (is_root, sudo_available) in [(false, false), (false, true), (true, true)] {
            let ctx = SudoContext {
                is_root,
                sudo_available,
                policy: SudoPolicy::ForceWithout,
            };
            assert!(
                !ctx.should_use_sudo(),
                "ForceWithout should always be false (is_root={is_root}, sudo_available={sudo_available})"
            );
        }
    }

    #[test]
    fn should_use_sudo_force_with_true_unless_root() {
        let non_root_with_sudo = SudoContext {
            is_root: false,
            sudo_available: true,
            policy: SudoPolicy::ForceWith,
        };
        assert!(non_root_with_sudo.should_use_sudo());

        let non_root_no_sudo = SudoContext {
            is_root: false,
            sudo_available: false,
            policy: SudoPolicy::ForceWith,
        };
        assert!(
            non_root_no_sudo.should_use_sudo(),
            "ForceWith ignores sudo_available"
        );

        let root = SudoContext {
            is_root: true,
            sudo_available: true,
            policy: SudoPolicy::ForceWith,
        };
        assert!(!root.should_use_sudo(), "sudo as root is redundant");
    }

    #[test]
    fn should_use_sudo_auto_only_when_non_root_and_sudo_available() {
        let cases = [
            (false, false, false), // non-root, no sudo → false
            (false, true, true),   // non-root, sudo available → true
            (true, false, false),  // root, no sudo → false
            (true, true, false),   // root, sudo available → false (already root)
        ];
        for (is_root, sudo_available, expected) in cases {
            let ctx = SudoContext {
                is_root,
                sudo_available,
                policy: SudoPolicy::Auto,
            };
            assert_eq!(
                ctx.should_use_sudo(),
                expected,
                "Auto policy: is_root={is_root}, sudo_available={sudo_available}"
            );
        }
    }

    #[test]
    fn sudo_context_default_should_use_sudo() {
        // Default = non-root + sudo available + auto → behaves like old hardcoded sudo.
        assert!(SudoContext::default().should_use_sudo());
    }

    // ── SudoAwareCommandExecutor: apply_sudo ────────────────────────────

    fn make_executor(context: SudoContext) -> SudoAwareCommandExecutor {
        let inner: Arc<dyn CommandExecutor> = Arc::new(LocalCommandExecutor);
        SudoAwareCommandExecutor::new(inner, context)
    }

    #[test]
    fn non_privileged_spec_unchanged() {
        let exec = make_executor(SudoContext::default());
        let spec = CommandSpec::exec("apt-get", ["update".to_string()]);
        assert!(!spec.privileged);
        let result = exec.apply_sudo(&spec);
        // Not privileged → untouched
        assert!(matches!(&result.mode, CommandMode::Exec { program, .. } if program == "apt-get"));
    }

    #[test]
    fn privileged_exec_gets_sudo_prepended() {
        let exec = make_executor(SudoContext::default());
        let spec = CommandSpec::exec(
            "apt-get",
            ["install".to_string(), "-y".to_string(), "nginx".to_string()],
        )
        .privileged();
        let result = exec.apply_sudo(&spec);
        match &result.mode {
            CommandMode::Exec { program, args } => {
                assert_eq!(program, "sudo");
                assert_eq!(args[0], "apt-get");
                assert_eq!(args[1], "install");
                assert_eq!(args[2], "-y");
                assert_eq!(args[3], "nginx");
            }
            other => panic!("expected Exec mode, got: {other:?}"),
        }
    }

    #[test]
    fn privileged_exec_not_modified_when_root() {
        let exec = make_executor(SudoContext {
            is_root: true,
            sudo_available: true,
            policy: SudoPolicy::Auto,
        });
        let spec = CommandSpec::exec("apt-get", ["update".to_string()]).privileged();
        let result = exec.apply_sudo(&spec);
        // Root → no sudo prepended
        match &result.mode {
            CommandMode::Exec { program, .. } => {
                assert_eq!(program, "apt-get");
            }
            other => panic!("expected Exec mode, got: {other:?}"),
        }
    }

    #[test]
    fn privileged_exec_not_modified_force_without() {
        let exec = make_executor(SudoContext {
            is_root: false,
            sudo_available: true,
            policy: SudoPolicy::ForceWithout,
        });
        let spec = CommandSpec::exec("apt-get", ["update".to_string()]).privileged();
        let result = exec.apply_sudo(&spec);
        match &result.mode {
            CommandMode::Exec { program, .. } => {
                assert_eq!(
                    program, "apt-get",
                    "ForceWithout: sudo must not be prepended"
                );
            }
            other => panic!("expected Exec mode, got: {other:?}"),
        }
    }

    #[test]
    fn privileged_shell_mode_passes_through_unchanged() {
        let exec = make_executor(SudoContext::default());
        let spec = CommandSpec::shell("echo hello").privileged();
        let result = exec.apply_sudo(&spec);
        // Shell mode: pass through without modification
        assert!(
            matches!(&result.mode, CommandMode::Shell { command, .. } if command == "echo hello")
        );
    }

    #[test]
    fn apply_sudo_preserves_working_dir_and_timeout() {
        use std::time::Duration;
        let exec = make_executor(SudoContext::default());
        let spec = CommandSpec::exec("apt-get", ["update".to_string()])
            .privileged()
            .with_working_dir("/tmp")
            .with_timeout(Duration::from_secs(60));
        let result = exec.apply_sudo(&spec);
        assert_eq!(result.working_dir.as_deref(), Some("/tmp"));
        assert_eq!(result.timeout, Some(Duration::from_secs(60)));
    }

    // ── SudoAwareCommandExecutor: end-to-end execute ────────────────────

    #[tokio::test]
    async fn execute_quiet_privileged_with_force_without_runs_directly() {
        // ForceWithout: sudo is not prepended, command runs without sudo.
        let exec = make_executor(SudoContext {
            is_root: false,
            sudo_available: false,
            policy: SudoPolicy::ForceWithout,
        });
        let spec = CommandSpec::exec("echo", ["hello from executor".to_string()]).privileged();
        let output = exec.execute_quiet(&spec).await.expect("should succeed");
        assert!(output.output.contains("hello from executor"));
        assert_eq!(output.exit_code, 0);
    }
}
