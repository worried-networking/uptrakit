//! Test doubles for [`RemoteExecutor`](crate::RemoteExecutor) consumers.
//!
//! Available behind the additive `test-support` feature; downstream crates
//! enable it from `[dev-dependencies]` only
//! (`uptrakit-command = { workspace = true, features = ["test-support"] }`).

use std::collections::VecDeque;

use async_trait::async_trait;

use crate::{RemoteCommandResult, RemoteExecutor};

/// Scripted [`RemoteExecutor`] test double: answers first by command-substring
/// matcher rules, then by a FIFO queue, and records every command string it
/// was asked to execute.
///
/// Commands beyond the script succeed with empty output and exit code `0`.
/// Locks are `parking_lot` (this is library code — the workspace bans
/// `std::sync::Mutex` in async code) and guards are dropped before any
/// `.await`; the matcher is a plain field, set at construction, read-only
/// after.
pub struct ScriptedRemoteExecutor {
    matcher: Vec<(&'static str, RemoteCommandResult)>,
    results: parking_lot::Mutex<VecDeque<RemoteCommandResult>>,
    calls: parking_lot::Mutex<Vec<String>>,
}

impl ScriptedRemoteExecutor {
    /// Create a double that replays `results` in FIFO order.
    pub fn new(results: impl IntoIterator<Item = RemoteCommandResult>) -> Self {
        Self {
            matcher: Vec::new(),
            results: parking_lot::Mutex::new(results.into_iter().collect()),
            calls: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Create a double that answers by first-matching command SUBSTRING
    /// (rule order matters); unmatched commands fall through to the FIFO
    /// queue, then to the empty-stdout exit-0 default. Use this whenever a
    /// test's assertions depend on WHICH command gets which answer rather
    /// than call order — FIFO scripts go silently green when the code under
    /// test reorders its commands.
    pub fn with_matcher(rules: Vec<(&'static str, RemoteCommandResult)>) -> Self {
        Self {
            matcher: rules,
            results: parking_lot::Mutex::new(VecDeque::new()),
            calls: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Every command string passed to `exec_command`, in order.
    pub fn recorded_calls(&self) -> Vec<String> {
        self.calls.lock().clone()
    }
}

#[async_trait]
impl RemoteExecutor for ScriptedRemoteExecutor {
    async fn exec_command(&self, command: &str) -> crate::Result<RemoteCommandResult> {
        self.calls.lock().push(command.to_string());
        if let Some(result) = self
            .matcher
            .iter()
            .find(|(needle, _)| command.contains(needle))
            .map(|(_, r)| r.clone())
        {
            return Ok(result);
        }
        let result = self
            .results
            .lock()
            .pop_front()
            .unwrap_or_else(|| RemoteCommandResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            });
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replays_fifo_and_records_calls() {
        let exec = ScriptedRemoteExecutor::new([RemoteCommandResult {
            stdout: "one".to_string(),
            stderr: String::new(),
            exit_code: 1,
        }]);
        let first = exec.exec_command("cmd-a").await.expect("scripted result");
        let second = exec.exec_command("cmd-b").await.expect("default result");
        assert_eq!(first.stdout, "one");
        assert_eq!(first.exit_code, 1);
        assert_eq!(second.exit_code, 0);
        assert_eq!(exec.recorded_calls(), vec!["cmd-a", "cmd-b"]);
    }
}
