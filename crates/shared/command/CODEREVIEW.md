# Code Review: `uptrakit-command`

- Review date: 2026-03-17
- Scope: current-state review (full 14-dimension)

## Summary

`uptrakit-command` remains a strong low-level crate. Shell escaping, timeout handling, sudo
adaptation, interactive execution, and the `CommandExecutor` trait hierarchy are all currently in
good shape.

## Strengths

- Good separation between command description (`CommandSpec`), execution (`CommandExecutor`),
  sudo policy, and interactive mode.
- Shell escaping uses the safe `'\''` idiom for single-quote contexts.
- Every execution path is bounded by an in-function deadline (`DEFAULT_COMMAND_TIMEOUT` resolved at
  the decision point when the spec carries none); on expiry the child is killed explicitly via
  `start_kill()`. `kill_on_drop(true)` is set only under `AbandonmentPolicy::CloseOnAbandon`, so a
  dropped future still kills the child; `DrainOnAbandon` instead lets the child run to completion
  with its output drained, for commands that must not be interrupted mid-flight.
- `stdin(Stdio::null())` prevents commands from blocking on stdin unexpectedly.
- `MAX_OUTPUT_BYTES` (10 MB) cap prevents OOM from runaway commands.
- `NoopCommandExecutor` provides a clean zero-cost stub for the controller side.
- `build_remote_command_string` properly shell-escapes all components including env vars and
  working directories.
- Timeout support via the `with_timeout` builder pattern, with an in-function deadline race
  (`run_command_exec_impl`) replacing the earlier wrapper-based approach.
- Strong unit coverage for shell escaping, timeouts, sudo behavior, and interactive execution.
- Interactive execution cleanly feature-gated behind `#[cfg(feature = "interactive")]`.

## Active Findings

No active findings were confirmed in this review pass.
