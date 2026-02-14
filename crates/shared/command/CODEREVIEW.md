# Code Review: uptrakit-command

## Summary

Shell command execution crate (~360 lines across 4 source files) providing `run_command()`, `run_command_with_shell()`, and `run_command_exec()` with real-time output streaming via `mpsc` channels. Implements shell escaping, fail-early shell wrappers (`set -euo pipefail`), and a direct-exec path that bypasses shell interpretation entirely.

## Architecture

- **Module structure**: `lib.rs` re-exports from `command.rs` (execution), `error.rs` (typed errors), `types.rs` (output line types, shell type enum).
- **Public API surface**: 5 functions + 4 types. Clean, focused surface.
- **Dependency choices**: `tokio` (process spawning, async I/O), `tracing`, `rootcause`/`thiserror` -- all workspace-managed.
- **Layering**: Leaf crate used by provider-core and agent. No upstream coupling.

## Security & Safety

- **Shell injection prevention**: Three defense layers:
  1. `shell_escape()` using POSIX single-quote wrapping (`src/command.rs:19-25`).
  2. Fail-early shell wrappers: `set -euo pipefail` for bash, `set -eu` for sh, `$ErrorActionPreference = 'Stop'` for PowerShell (`src/command.rs:144-155`).
  3. `run_command_exec()` direct process execution without shell (`src/command.rs:41-142`).
- **Injection test**: `shell_escape_prevents_injection_in_bash` (`src/command.rs:233-245`) validates end-to-end safety with payload `"2.0.0'; echo 'MARKER"`.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code. Output channel send failures are intentionally ignored (`let _ = output_tx.send(...)`) since channel closure should not crash the executor.

## Code Quality

- **Error handling**: `CommandError` enum with `CommandSpawn`, `CaptureFailed`, `CommandFailed(i32)`, `CommandWait` variants. Uses `rootcause::Report` wrapper and `bail!`/`report!` macros.
- **Memory safety**: `MAX_OUTPUT_BYTES` (10 MiB) caps accumulated output (`src/command.rs:17`).
- **Consistency**: Async task failures are logged and appended as error markers rather than panicking (`src/command.rs:115-128`).
- **Test coverage**: 20 tests covering shell escaping (6), shell wrappers (3), shell args (2), command execution (6), injection prevention (1), and type variants (2).

## Coding Standards Compliance

- Typed error enum with `thiserror` and `rootcause::Report` wrapper -- compliant.
- `Result<T>` type alias defined (`src/error.rs:21`).
- No `#[allow()]` directives.

## Extensibility Assessment

The crate is well-scoped as a leaf dependency. Two type-overlap issues affect cross-crate consistency:

1. **`ShellType` duplicates `HookShell` from the wire crate**: Both enums have the same variants
   (`Bash`, `Sh`, `PowerShell`). No conversion exists between them. When the agent receives an
   `ExecuteUpdatePayload` with `HookShell::Bash`, it must manually map to `ShellType::Bash` before
   calling `run_command_with_shell`. These should be unified into a single type.

2. **All public functions require `mpsc::Sender<UpdateOutputLine>`**: This makes the execution functions
   unusable for simple use cases where output streaming is not needed. A wrapper that discards output or
   returns it as a collected string would improve ergonomics for external consumers.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| CMD-01 | Info | Code Quality | No timeout mechanism. Callers cannot kill stuck commands. The `MAX_OUTPUT_BYTES` cap prevents OOM but does not address hung processes. Timeout is the responsibility of the caller (agent/controller), but the absence of a built-in mechanism is worth noting. | `src/command.rs:41-142` |
| CMD-02 | Info | Code Quality | `send_output()` ignores channel send failures (`let _ = output_tx.send(...)`). Intentional and correct (channel closure is not an error for the executor), but callers have no indication that output was dropped. | `src/command.rs:33-38` |
| ~~CMD-03~~ | ~~Major~~ | ~~Extensibility~~ | ~~`ShellType` duplicates `HookShell` from the wire crate.~~ **FIXED.** `ShellType` replaced with `HookShell` from `shared-types`. The agent no longer needs manual mapping. | `src/types.rs` |
| CMD-04 | Minor | Extensibility | All public execution functions require `mpsc::Sender<UpdateOutputLine>`. Simple use cases (e.g., running a command and collecting output) need unnecessary channel setup. A convenience wrapper `run_command_simple()` returning `String` would improve ergonomics. | `src/command.rs` |

## Verdict

**Pass.** Excellent security posture with multi-layered injection prevention and comprehensive tests. The `ShellType` duplication (CMD-03) is the main extensibility concern. Timeout management is left to callers by design.
