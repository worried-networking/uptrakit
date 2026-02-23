# Code Review: `uptrakit-command`

**Date:** 2026-02-17
**Reviewer:** Claude Opus 4.6 (automated)
**Scope:** Architecture, security, code quality, coding standards
**Overall quality: HIGH**

All 37 tests pass. The crate compiles cleanly with no warnings.

---

## Architecture

The crate provides a command execution abstraction with two layers:

- **Low-level functions** (`command.rs`): `run_command_exec`, `run_command_with_shell`, `run_command`, plus `_quiet`
  variants. These spawn processes directly via `tokio::process::Command`.
- **High-level trait** (`executor.rs`): `CommandExecutor` trait with `CommandSpec`/`CommandMode` types.
  `LocalCommandExecutor` implements it for the local machine; an `SshCommandExecutor` (in `agent-ssh`) implements it for
  remote hosts.

This is a well-designed separation of concerns. Providers build a `CommandSpec` describing _what_ to run, and the
injected executor decides _how_ to run it. The `CommandExecutor` trait is minimal (two methods), `Send + Sync`, and
object-safe, enabling `Arc<dyn CommandExecutor>` usage.

---

## Extensibility Findings

All type duplication issues have been resolved: `ShellType` alias removed, `UpdateOutputStream` removed in favour of
the canonical `OutputStreamType` from `uptrakit-shared-types`.

---

## Code Quality Findings

### PASS: Shell injection prevention

`shell_escape` in `src/command.rs` uses the standard POSIX single-quote-wrapping technique (`'\''` idiom). The test
suite explicitly verifies injection prevention with semicolons, backticks, `$(...)` subshells, and embedded single
quotes. The `shell_escape_prevents_injection_in_bash` integration test confirms the escaped value round-trips correctly
through bash.

`run_command_exec` uses direct program execution with `Command::new(program).args(args)`, bypassing shell interpretation
entirely. Shell-mode functions require callers to use `shell_escape` on interpolated values -- this is an appropriate
trust boundary.

### PASS: Error handling

Uses `rootcause` + `thiserror` consistently per project standards. All error paths use `report!()` / `bail!()`. No
errors are silently swallowed.

### PASS: Output size limits

`MAX_OUTPUT_BYTES = 10 MB` cap per stream prevents OOM. Both stdout and stderr independently capped.

### PASS: Resource cleanup

`child.wait().await` always called after both reader tasks complete. `tokio::join!` ensures both stdout/stderr reader
tasks complete before waiting on the child, avoiding deadlocks from full pipe buffers. Channel send errors silently
ignored with `let _ = tx.send(...)` -- correct since dropped receivers indicate nothing meaningful to do.

### PASS: No production `unwrap`/`panic`

All `unwrap()` and `expect()` calls are exclusively in `#[cfg(test)]` blocks. The only production `unwrap_or` is
`status.code().unwrap_or(-1)` which is correct for signal-terminated processes on Unix.

### PASS: No dead code

All public items exported from `lib.rs` are used by downstream crates.

### ~~INFORMATIONAL: No command timeout mechanism~~ RESOLVED

`CommandSpec` now has a `timeout: Option<std::time::Duration>` field and a `with_timeout()` builder method.
`LocalCommandExecutor` applies `tokio::time::timeout` when the field is set, returning
`CommandError::TimedOut` on expiry. `ProviderError::TimedOut` was added to `uptrakit-provider-core` to
cover the new variant in its conversion match. Tests use `tokio::time::pause()` / `advance()` per project
conventions.

### ~~INFORMATIONAL: PowerShell executable name~~ RESOLVED

`HookShell` in `uptrakit-shared-types` now exposes `local_executable()`, `remote_executable(is_windows)`,
and `flag()` methods. `local_executable()` returns `"pwsh"` on non-Windows platforms and `"powershell"` on
Windows. `get_shell_args` in `src/command.rs` delegates to these methods. SSH executors targeting remote
hosts should call `remote_executable(target_is_windows)` instead.

### ~~INFORMATIONAL: Output concatenation order~~ RESOLVED

The `CommandOutput.output` field docstring now explicitly states: "The accumulated stdout followed by
stderr output. Stdout content always precedes stderr content, regardless of the actual temporal
interleaving of the two streams. This is a fundamental limitation of reading from separate pipes."

---

## Summary

| Category            | Status | Notes                                                            |
| ------------------- | ------ | ---------------------------------------------------------------- |
| Shell injection     | PASS   | `shell_escape` + exec mode bypass shell entirely                 |
| Input validation    | PASS   | OS errors propagate cleanly through the error chain              |
| Resource cleanup    | PASS   | Process reaped, tasks joined, pipe buffers drained               |
| Output size limit   | PASS   | 10 MB cap per stream                                             |
| Error handling      | PASS   | Uses rootcause/thiserror per project standards                   |
| Sensitive data      | PASS   | No credentials, secrets, or environment variable leakage         |
| Dependencies        | PASS   | Minimal dependency set; all workspace-managed                    |
| Test coverage       | PASS   | 37 tests covering success/failure paths, injection, all variants |
| `unwrap`/`panic`    | PASS   | Zero in production code                                          |
| Extensibility       | GOOD   | Well-designed trait; type duplication resolved                   |
