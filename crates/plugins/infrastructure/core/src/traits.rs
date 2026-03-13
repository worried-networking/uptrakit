use std::borrow::Cow;

use crate::types::ReleaseInfo;

/// Whether a plugin is applicable to the current host.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCompatibility {
    /// Plugin is applicable.
    Compatible,
    /// Plugin is not applicable (e.g., APT on a non-Debian host).
    Incompatible(String),
}

/// Contextual data passed to plugin lifecycle hooks.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct UpdateHookContext {
    /// The package identifier being updated.
    pub package_identifier: String,
    /// The target version being installed.
    pub to_version: String,
    /// Optional release metadata from the upstream source.
    pub release_info: Option<ReleaseInfo>,
}

impl UpdateHookContext {
    /// Create a new [`UpdateHookContext`].
    pub fn new(
        package_identifier: impl Into<String>,
        to_version: impl Into<String>,
        release_info: Option<ReleaseInfo>,
    ) -> Self {
        Self {
            package_identifier: package_identifier.into(),
            to_version: to_version.into(),
            release_info,
        }
    }
}

/// Result of a pre-update hook.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PreUpdateHookResult {
    /// Whether the update should proceed.
    pub should_proceed: bool,
    /// Reason for aborting if `should_proceed` is false.
    pub abort_reason: Option<String>,
}

impl PreUpdateHookResult {
    /// Create a result that allows the update to proceed.
    pub fn proceed() -> Self {
        Self {
            should_proceed: true,
            abort_reason: None,
        }
    }

    /// Create a result that aborts the update with the given reason.
    pub fn abort(reason: impl Into<String>) -> Self {
        Self {
            should_proceed: false,
            abort_reason: Some(reason.into()),
        }
    }
}

/// A helper script installed by the bootstrap process on the managed host.
///
/// Enables argument-validated sudo commands — something sudoers wildcards
/// cannot express safely, because `*` in sudoers matches `/`, making
/// path-based wildcard restrictions ineffective (e.g. `/usr/bin/cat /root/.*`
/// would still allow reading `/root/.ssh/id_rsa`).
///
/// The script must validate its own arguments before acting, making the
/// corresponding sudoers entry unconditional (no argument wildcard needed):
///
/// ```text
/// uptrakit ALL=(root) NOPASSWD: /usr/local/bin/my-helper
/// ```
///
/// # Contract
///
/// - `install_path` must be an absolute path (e.g. `/usr/local/bin/my-helper`).
/// - `content` must be a complete, self-contained shell script that validates
///   its arguments and exits non-zero on invalid input.
/// - The script is installed with mode `0755` and owned by root.
#[non_exhaustive]
pub struct SudoHelperScript {
    /// Absolute path where the script is installed on the managed host.
    ///
    /// Used directly as the sudoers command (no `command -v` resolution).
    pub install_path: &'static str,
    /// Complete shell script content installed verbatim at `install_path`.
    pub content: &'static str,
}

impl SudoHelperScript {
    /// Create a new [`SudoHelperScript`].
    pub fn new(install_path: &'static str, content: &'static str) -> Self {
        Self {
            install_path,
            content,
        }
    }
}

/// Describes a single command that a plugin needs to run with passwordless sudo.
///
/// Plugins return a [`Vec<SudoCommandEntry>`] from
/// [`PluginBase::required_sudo_commands`](crate::PluginBase::required_sudo_commands) to declare which commands they need
/// elevated privileges for. The bootstrap process and `update-sudoers` command
/// use these declarations to generate minimal, specific sudoers entries instead
/// of a blanket `NOPASSWD: ALL` rule.
///
/// # Contract
///
/// When `helper_script` is `None`:
/// - `command` must be a **bare command name** (e.g. `"apt-get"`), never an
///   absolute path. The agent resolves it to an absolute path on the target
///   host at sudoers-generation time using `command -v`.
///
/// When `helper_script` is `Some`:
/// - `command` is used only as a display name (not resolved via `command -v`).
/// - Bootstrap installs the script at `helper_script.install_path`, sets
///   permissions to `0755`, and uses that path as the sudoers command.
///   The script's own argument validation enforces restrictions that sudoers
///   wildcards cannot safely express.
///
/// In both cases `explanation` is shown as a comment in the generated sudoers
/// file and in CLI output for human reviewers.
#[non_exhaustive]
pub struct SudoCommandEntry {
    /// Bare command name (e.g. `"apt-get"`) or a short display identifier for
    /// helper scripts.
    ///
    /// When `helper_script` is `None`, this is resolved to an absolute path
    /// via `command -v` on the target host. When `helper_script` is `Some`,
    /// this field is used only for logging and display purposes; the sudoers
    /// entry uses `helper_script.install_path`.
    pub command: String,
    /// Human-readable explanation shown in sudoers comments and CLI output.
    pub explanation: String,
    /// Optional helper script to install on the managed host during bootstrap.
    ///
    /// When `Some`, bootstrap installs this script before writing the sudoers
    /// entry. The sudoers entry uses the install path directly. The script must
    /// validate its own arguments to enforce the least-privilege contract.
    pub helper_script: Option<SudoHelperScript>,
    /// Optional argument suffix appended to the resolved command path in the
    /// sudoers entry (e.g. `"start *"` → `/usr/bin/systemctl start *`).
    ///
    /// Use this to restrict the allowed subcommands/arguments without needing a
    /// helper script. The suffix is appended as whitespace-delimited command
    /// tokens after a space separator. The SSH agent escapes sudoers-special
    /// characters when rendering the drop-in file while preserving wildcard
    /// tokens such as `*` anywhere in the argument list. When `None`, the
    /// sudoers entry permits any arguments.
    pub args_suffix: Option<Cow<'static, str>>,
    /// When `true`, the sudoers entry is generated with the `SETENV:` tag, which
    /// allows the agent to pass inline `NAME=VALUE` env var assignments before
    /// the program name (e.g. `sudo DEBIAN_FRONTEND=noninteractive apt-get …`).
    ///
    /// Set this to `true` only when the plugin invokes the command with
    /// [`CommandSpec::with_env`] in combination with `.privileged()`.
    /// Defaults to `false` for commands that don't need env var forwarding.
    pub needs_setenv: bool,
}

impl SudoCommandEntry {
    /// Create a new [`SudoCommandEntry`] with the given command and explanation.
    ///
    /// Optional fields (`helper_script`, `args_suffix`, `needs_setenv`) default
    /// to `None`/`false` and can be set via the builder methods or public fields.
    pub fn new(command: impl Into<String>, explanation: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            explanation: explanation.into(),
            helper_script: None,
            args_suffix: None,
            needs_setenv: false,
        }
    }

    /// Set the argument suffix (builder method).
    #[must_use]
    pub fn with_args_suffix(mut self, suffix: impl Into<Cow<'static, str>>) -> Self {
        self.args_suffix = Some(suffix.into());
        self
    }

    /// Enable `SETENV:` in the sudoers entry (builder method).
    #[must_use]
    pub fn with_setenv(mut self) -> Self {
        self.needs_setenv = true;
        self
    }

    /// Set the helper script (builder method).
    #[must_use]
    pub fn with_helper_script(mut self, script: SudoHelperScript) -> Self {
        self.helper_script = Some(script);
        self
    }
}
