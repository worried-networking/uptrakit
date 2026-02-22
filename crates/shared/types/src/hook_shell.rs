use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Shell type for hook execution in update payloads.
///
/// Determines which shell interpreter and fail-early settings are used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum HookShell {
    /// Bash shell with `set -euo pipefail`
    #[default]
    Bash,
    /// POSIX sh with `set -eu`
    Sh,
    /// PowerShell with `$ErrorActionPreference = 'Stop'`
    #[serde(rename = "powershell")]
    PowerShell,
}

impl std::fmt::Display for HookShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl HookShell {
    /// Returns the string representation of the shell type.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Sh => "sh",
            Self::PowerShell => "powershell",
        }
    }

    /// Shell executable for the **local** machine (auto-detects platform).
    ///
    /// PowerShell uses `pwsh` (PowerShell Core) on Linux/macOS because
    /// `powershell` (Windows PowerShell 5.1) does not exist there.
    pub fn local_executable(self) -> &'static str {
        self.executable_for_platform(cfg!(target_os = "windows"))
    }

    /// Shell executable for a **remote** host.
    ///
    /// Pass `true` when the remote host is Windows; `false` for Linux, macOS,
    /// or any other non-Windows OS. SSH executors use this method because
    /// `local_executable()` reflects the agent machine's OS, not the target's.
    ///
    /// PowerShell: Windows → `powershell`, non-Windows → `pwsh`.
    pub fn remote_executable(self, remote_is_windows: bool) -> &'static str {
        self.executable_for_platform(remote_is_windows)
    }

    /// The argument flag for this shell (e.g., `-c` or `-Command`).
    pub fn flag(self) -> &'static str {
        match self {
            Self::Bash | Self::Sh => "-c",
            Self::PowerShell => "-Command",
        }
    }

    fn executable_for_platform(self, is_windows: bool) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Sh => "sh",
            Self::PowerShell => {
                if is_windows {
                    "powershell"
                } else {
                    "pwsh"
                }
            }
        }
    }
}

/// Error returned when parsing an invalid hook shell string.
#[derive(Debug)]
pub struct ParseHookShellError;

impl fmt::Display for ParseHookShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid hook shell value")
    }
}

impl std::error::Error for ParseHookShellError {}

impl FromStr for HookShell {
    type Err = ParseHookShellError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bash" => Ok(Self::Bash),
            "sh" => Ok(Self::Sh),
            "powershell" => Ok(Self::PowerShell),
            _ => Err(ParseHookShellError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for shell in [HookShell::Bash, HookShell::Sh, HookShell::PowerShell] {
            let json = serde_json::to_string(&shell).unwrap();
            let deserialized: HookShell = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, shell);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for shell in [HookShell::Bash, HookShell::Sh, HookShell::PowerShell] {
            assert_eq!(format!("{shell}"), shell.as_str());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for shell in [HookShell::Bash, HookShell::Sh, HookShell::PowerShell] {
            let s = shell.as_str();
            let parsed: HookShell = s.parse().unwrap();
            assert_eq!(parsed, shell);
        }
    }

    #[test]
    fn from_str_unknown_returns_err() {
        assert!("zsh".parse::<HookShell>().is_err());
        assert!("".parse::<HookShell>().is_err());
    }

    #[test]
    fn default_is_bash() {
        assert_eq!(HookShell::default(), HookShell::Bash);
    }

    #[test]
    fn hook_shell_local_executable_powershell() {
        #[cfg(target_os = "windows")]
        assert_eq!(HookShell::PowerShell.local_executable(), "powershell");
        #[cfg(not(target_os = "windows"))]
        assert_eq!(HookShell::PowerShell.local_executable(), "pwsh");
    }

    #[test]
    fn hook_shell_local_executable_bash_sh() {
        assert_eq!(HookShell::Bash.local_executable(), "bash");
        assert_eq!(HookShell::Sh.local_executable(), "sh");
    }

    #[test]
    fn hook_shell_remote_executable_windows() {
        assert_eq!(HookShell::PowerShell.remote_executable(true), "powershell");
        assert_eq!(HookShell::Bash.remote_executable(true), "bash");
        assert_eq!(HookShell::Sh.remote_executable(true), "sh");
    }

    #[test]
    fn hook_shell_remote_executable_non_windows() {
        assert_eq!(HookShell::PowerShell.remote_executable(false), "pwsh");
        assert_eq!(HookShell::Bash.remote_executable(false), "bash");
        assert_eq!(HookShell::Sh.remote_executable(false), "sh");
    }

    #[test]
    fn hook_shell_flag() {
        assert_eq!(HookShell::Bash.flag(), "-c");
        assert_eq!(HookShell::Sh.flag(), "-c");
        assert_eq!(HookShell::PowerShell.flag(), "-Command");
    }
}
