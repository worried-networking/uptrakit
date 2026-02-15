use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Shell type for hook execution in update payloads.
///
/// Determines which shell interpreter and fail-early settings are used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Sh => "sh",
            Self::PowerShell => "powershell",
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
}
