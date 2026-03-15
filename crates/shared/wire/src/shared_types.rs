use std::fmt;

use serde::{Deserialize, Serialize};
use time::UtcDateTime;

use uptrakit_shared_types::HookShell;

/// Unix epoch timestamp in milliseconds.
pub type Timestamp = i64;

/// Returns the current time as Unix epoch milliseconds.
pub fn now_millis() -> Timestamp {
    let now = UtcDateTime::now();
    now.unix_timestamp() * 1000 + i64::from(now.millisecond())
}

/// A single hook command to execute on the agent.
///
/// Predefined hooks use the `Exec` variant which avoids shell interpretation.
/// Custom commands use the `Shell` variant which runs through a shell.
///
/// # Wire forward-compatibility
///
/// `Other { raw }` is a catch-all for hook command types introduced in a
/// newer agent build. Serde deserialization is infallible: an unknown
/// variant becomes `Other { raw: ... }` rather than a parse error, allowing
/// older controllers to survive rolling upgrades.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
// Note: Eq is not derived because the Other variant contains serde_json::Value
// which does not implement Eq.
pub enum HookCommand {
    /// Execute a command string through a shell interpreter.
    Shell {
        command: String,
        #[serde(default)]
        shell: HookShell,
    },
    /// Execute a program directly with arguments (no shell interpretation).
    Exec {
        program: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        working_dir: Option<String>,
    },
    /// Unknown hook command from a newer peer.
    ///
    /// The raw JSON value is preserved for logging. The receiver should
    /// log a warning and skip execution.
    #[serde(skip)]
    Other { raw: serde_json::Value },
}

impl<'de> Deserialize<'de> for HookCommand {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        if let Some(obj) = raw.as_object() {
            if let Some(shell_val) = obj.get("shell") {
                #[derive(Deserialize)]
                struct ShellFields {
                    command: String,
                    #[serde(default)]
                    shell: HookShell,
                }
                if let Ok(f) = serde_json::from_value::<ShellFields>(shell_val.clone()) {
                    return Ok(HookCommand::Shell {
                        command: f.command,
                        shell: f.shell,
                    });
                }
            }
            if let Some(exec_val) = obj.get("exec") {
                #[derive(Deserialize)]
                struct ExecFields {
                    program: String,
                    #[serde(default)]
                    args: Vec<String>,
                    #[serde(default)]
                    working_dir: Option<String>,
                }
                if let Ok(f) = serde_json::from_value::<ExecFields>(exec_val.clone()) {
                    return Ok(HookCommand::Exec {
                        program: f.program,
                        args: f.args,
                        working_dir: f.working_dir,
                    });
                }
            }
        }
        Ok(HookCommand::Other { raw })
    }
}

/// Human-readable formatting for logging only. Not intended for round-trip
/// serialization — use serde for machine-readable encoding.
impl fmt::Display for HookCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shell { command, .. } => write!(f, "{command}"),
            Self::Exec {
                program,
                args,
                working_dir,
            } => {
                if let Some(dir) = working_dir {
                    write!(f, "(in {dir}) ")?;
                }
                write!(f, "{program}")?;
                for arg in args {
                    write!(f, " {arg}")?;
                }
                Ok(())
            }
            Self::Other { raw } => write!(f, "<unknown hook command: {raw}>"),
        }
    }
}

/// Final status of an update execution.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for status strings received from a newer
/// agent that this build does not yet recognise. Serde deserialization is
/// infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing older controllers to survive rolling upgrades without
/// dropping the enclosing `UpdateResult` message.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateFinalStatus {
    Completed,
    Failed,
    /// An unknown status received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl UpdateFinalStatus {
    /// Returns the string representation.
    ///
    /// For [`UpdateFinalStatus::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for UpdateFinalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for UpdateFinalStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for UpdateFinalStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UpdateFinalStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(UpdateFinalStatus::from)
    }
}

/// Default timeout for update execution (2 hours).
pub const DEFAULT_UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(7200);

/// Default timeout for update execution.
pub(crate) fn default_update_timeout() -> std::time::Duration {
    DEFAULT_UPDATE_TIMEOUT
}

/// Reason for service disconnection.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for reason strings received from a newer
/// peer that this build does not yet recognise. Serde deserialization is
/// infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing rolling upgrades without dropping the `Disconnecting` message.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    /// SIGTERM/SIGINT - clean exit.
    Shutdown,
    /// SIGHUP - will reconnect after external restart.
    Restart,
    /// An unknown reason received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl DisconnectReason {
    /// Returns the string representation.
    ///
    /// For [`DisconnectReason::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Shutdown => "shutdown",
            Self::Restart => "restart",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for DisconnectReason {
    fn from(s: String) -> Self {
        match s.as_str() {
            "shutdown" => Self::Shutdown,
            "restart" => Self::Restart,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for DisconnectReason {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DisconnectReason {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(DisconnectReason::from)
    }
}
