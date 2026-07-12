# Shell Hook Plugin: Route Commands Through the Injected Executor — Design

**Date:** 2026-07-12 **Status:** Draft **Source:** `.superpowers/audit-2026-07-11.md` — HIGH "Shell hook plugin
bypasses the CommandExecutor and always runs hook commands locally" (`crates/plugins/hooks/shell/src/plugin.rs:57`).

## Problem

`ShellHookPlugin` receives the host runtime's `CommandExecutor` (`runtime.executor()`, `plugin.rs:33`) but holds it as
`#[expect(dead_code)]` (`plugin.rs:20-24`) and executes hook commands with the free function
`uptrakit_command::run_command_with_shell` (`plugin.rs:57`), which spawns a **local** `tokio::process::Command`
(`command.rs:257-267` → `run_command_exec_impl` → `tokio::process::Command::new` at `command.rs:71`).

For an **SSH-managed** host the runtime's executor is `SudoAwareCommandExecutor(PosixSshCommandExecutor)`, which routes
commands over SSH to the target host. The systemd hook plugin uses it correctly
(`self.executor.execute(&spec, output_tx)`, `systemd/src/plugin.rs:42`). The shell hook does not, so a shell pre/post
hook configured for a remote host **runs on the agent's own machine instead of the target host**. It can report success
without ever touching the target, or run destructive commands (service stops, backups) on the wrong machine. This
breaks the executor abstraction the agent-core/agent boundary is built on.

Root cause: the plugin bypasses the injected executor entirely. The injected executor is the sole thing that knows
_where_ a command must run (local vs SSH). Spawning locally is unconditionally wrong for any non-local runtime.

**Scope correction (verified against `types.rs:143-152`):** the audit also notes the bypass "skips the SudoAware
privileged-command transformation." That transform applies **only to privileged Exec-mode** commands — `privileged` has
**no effect on Shell-mode commands** (documented at `types.rs:147`; sudo is never prepended to a shell string). Shell
hooks are arbitrary unprivileged user scripts and must stay unprivileged (a script needing root uses its own `sudo`,
governed by the plugin sudo allowlist). So the sudo angle is a **non-issue** here; the single real defect is
wrong-host execution. This spec fixes exactly that and changes nothing about privilege.

## Approach

Route the hook command through the injected executor, exactly as the systemd hook does. The fix is fully contained in
`crates/plugins/hooks/shell/src/plugin.rs` — no new types, no config change, no change to `uptrakit_command`.

### 1. Execute via `self.executor.execute(&CommandSpec::shell_with(cmd, shell), output_tx)`

`CommandSpec::shell_with(command, shell)` (`types.rs:129-141`) already exists and is the exact constructor needed: it
builds `CommandMode::Shell { command, shell }` with `timeout: None`, `privileged: false`, no envs, no working_dir —
matching everything the shell plugin passes today (it passes **only** command + shell; no env/cwd/timeout
customization).

**Behavior-equivalence (load-bearing, verified):** `CommandSpec::resolve()` for Shell mode
(`types.rs:196-200`) calls the **same** `wrap_command_for_shell(command, shell)` + `get_shell_args(shell)` helpers that
`run_command_with_shell` calls (`command.rs:258-260`). So `executor.execute(&CommandSpec::shell_with(cmd, shell))`
resolves to a **byte-identical** `(shell_exec, ["-c", "set -euo pipefail\n<cmd>"])` invocation — the fail-early
semantics, shell selection, and `UnsupportedShell` error are all preserved. The _only_ thing that changes is the
execution site: the injected executor decides local-vs-SSH. There is no second shell-wrapping path to drift against.

Convert the current free function `run_shell_command(command, shell, output_tx)` into a `&self` method (so it can reach
`self.executor`). Only the command-invocation line changes; the exit-code handling stays **byte-for-byte identical**:

```rust
/// Run a shell hook command through the host runtime's injected [`CommandExecutor`],
/// so it targets the correct host (local or SSH), not the agent's own machine.
///
/// Returns `Ok(exit_code)` for any command that ran, including non-zero exits — the
/// executor surfaces a non-zero exit as `CommandError::CommandFailed(code)`, which we
/// unwrap to the code so callers decide the semantics (pre-hook abort vs. post-hook warn).
///
/// # Errors
/// Returns `PluginError::InstallFailed` only on a genuine transport/spawn failure
/// (SSH unreachable, unsupported shell) — never for a hook that merely exited non-zero.
async fn run_shell_command(
    &self,
    command: &str,
    shell: HookShell,
    output_tx: &UpdateOutputSender,
) -> Result<i32> {
    let spec = CommandSpec::shell_with(command, shell);
    match self.executor.execute(&spec, output_tx).await {
        Ok(output) => {
            tracing::debug!(exit_code = output.exit_code, output_len = output.output.len(), "shell hook completed");
            Ok(output.exit_code)
        }
        Err(e) => {
            if let uptrakit_command::CommandError::CommandFailed(code) = e.current_context() {
                return Ok(*code);
            }
            Err(report!(uptrakit_plugin_infrastructure_core::PluginError::InstallFailed(format!(
                "shell hook command failed: {e}"
            ))))
        }
    }
}
```

Call sites at `plugin.rs:98` / `plugin.rs:139` become `self.run_shell_command(cmd, self.config.shell, output_tx)`.

**Exit-code semantics are preserved (load-bearing, verified).** `CommandExecutor::execute` maps a **non-zero exit to
`Err(CommandError::CommandFailed(exit_code))`**, in **every** implementation:

- `LocalCommandExecutor::execute` (`executor.rs:190-206`) delegates to `run_command_exec_impl`, which
  `bail!(CommandError::CommandFailed(exit_code))` when `!status.success()` (`command.rs:190-191`). The
  `execute_failure` unit test (`executor.rs:362-370`) asserts `result.is_err()` for `exit 1`.
- The SSH path (`agent-ssh-runtime/src/ssh_executor.rs:142-147`) does the same: `if result.exit_code != 0 { …
bail!(CommandError::CommandFailed(exit_code)) }`.

So the shell hook **must keep** the current `CommandFailed(code) → Ok(code)` extraction — it is _not_ dead, it is the
only thing that turns a non-zero hook exit back into a value the `on_failure`/`should_proceed` logic in
`execute_pre_hook` (`plugin.rs:98-104`, non-zero ⇒ graceful `abort`) and `execute_post_hook` (`plugin.rs:139-149`,
non-zero ⇒ **non-fatal warn**, still `Ok`) can branch on. Dropping it would turn a post-hook that exits `1` into a hard
`PluginError` failure — a behavior regression. The Err-arm is copied verbatim from today's free function; only the
command source changes from `run_command_with_shell(...)` to `self.executor.execute(&spec, ...)`. Systemd gets away
with a plain `?` + `if result.exit_code != 0` because it always _wants_ any non-zero to fail; the shell hook does not.

### 2. Remove `#[expect(dead_code)]`

The `executor` field is now used, so the `#[expect(dead_code, reason = …)]` attribute (`plugin.rs:20-23`) must be
deleted — an unfulfilled `#[expect]` is itself a lint error under the workspace `warnings = "deny"`. No other change to
the struct.

### `run_command_with_shell` stays

Not orphaned: it retains callers (`run_command` at `command.rs:291`, the `lib.rs` re-export, and its own unit tests).
Leave it untouched (YAGNI — deleting it is out of scope and unrelated).

### Why not the audit's fallback ("gate the plugin to exclude SSH-remote runtimes")

Rejected as infeasible and wrong-direction. `HostRequirements` distinguishes OS family / capabilities, **not**
local-vs-remote (`host_requirements.rs:146-171`; `POSIX` matches any Linux/macOS/FreeBSD host, SSH or local), and no
`HostRuntime`/`CommandExecutor` method reports whether execution is remote. There is no clean way to express "not SSH."
More fundamentally, the plugin _should_ work on remote hosts — routing through the executor is the correct, idiomatic
fix that makes it work everywhere, not a restriction that disables it where it's most useful.

## Tests

The current tests (`plugin.rs:154-261`) use `LocalCommandExecutor` and assert `echo`/`exit 1` behavior. They pass
today **and** after the fix (local shell still runs), so they cannot catch a regression to the local-spawn bypass — the
bug is invisible to them because on a local runtime both paths spawn locally. The regression test must prove the command
is **routed through the injected executor**.

Add a **recording executor double** in the shell plugin's test module — a `CommandExecutor` impl that captures each
`CommandSpec` passed to `execute()` into a `parking_lot::Mutex<Vec<CommandSpec>>` and returns a **configurable
`crate::Result<CommandOutput>`** (so a test can make it yield `Ok(CommandOutput { exit_code: 0, .. })`,
`Err(CommandError::CommandFailed(1))`, or `Err(CommandError::UnsupportedShell(..))`). Matching the real contract — where
non-zero exit is an `Err(CommandFailed)`, not an `Ok` — is essential; a double that returns `Ok(exit_code: 1)` would not
exercise the extraction path. (The Docker plugin's `MockCommandExecutor`, `releases/docker/src/tests.rs:24-53`, is the
shape to copy, extended to record the spec and to return a caller-supplied result.) `parking_lot` is a workspace dep;
`#[async_trait]` per the trait.

Tests:

1. **Routing regression (the HIGH):** run a pre-hook through the recording double (configured `Ok(exit_code: 0)`);
   assert exactly one captured spec, and that its `mode` is `CommandMode::Shell { command, shell }` carrying the
   configured command + `config.shell`. Proves the command went through `executor.execute`, not a local spawn. _(If
   `CommandMode`/its fields are not `pub` for cross-crate assertion, assert via the spec's `Debug`/`resolve()` output —
   resolve to `(shell, ["-c", wrapped])` and assert the wrapped string contains the command; implementer picks whichever
   is public.)_
2. **Post-hook routes too:** same capture assertion via `execute_post_hook`.
3. **Non-zero exit is extracted, not hard-failed (the regression guard):** double returns
   `Err(CommandError::CommandFailed(1))`. Assert `execute_pre_hook` returns `Ok(PreUpdateHookResult)` with
   `should_proceed == false` (graceful abort, **not** an `Err`); assert `execute_post_hook` (with `on_failure: true` so
   it runs) returns `Ok(())` (non-fatal warn, **not** an `Err`). This is the exact behavior the naive `?`-only approach
   would have broken.
4. **Exit 0 → proceed:** double returns `Ok(exit_code: 0)` → pre-hook `should_proceed == true`, post-hook `Ok(())`.
5. **Genuine transport error → `PluginError`, not a silent success:** double returns a **non-`CommandFailed`** error;
   assert the hook returns an `Err` (not `Ok(should_proceed = true)`). Cover both `Err(CommandError::UnsupportedShell(..))`
   (a real SSH/shell failure) **and** `Err(CommandError::UnsupportedOperation(..))` — the latter is what a
   `NoopCommandExecutor` returns, so this doubles as a guard that a Noop-executor leak surfaces loudly as
   `PluginError::InstallFailed` rather than a silent success. Confirms only real spawn/transport failures propagate,
   while non-zero exits (test 3) do not.

Keep the existing `LocalCommandExecutor` echo/exit tests as end-to-end smoke coverage (unchanged).

No `start_paused` — no `tokio::time` API is used.

## Documentation deliverables

- **Doc comment** on the reworked `run_shell_command` method (shown in §1, including the `# Errors` section required by
  coding-standards for a public-ish fallible fn) plus a one-line note on the struct, stating the hook runs through the
  injected `CommandExecutor` so it targets the correct host (local or SSH) — replacing the stale `plugin.rs:47-51`
  comment that references the bypassed `run_command_with_shell` utility.
- **`docs/development/plugin-guidelines.md`** — the **`## Update Lifecycle Plugins`** section already exists (line 163;
  line 189 already documents the post-hook non-fatal contract). Add one invariant line there: _"Lifecycle hook plugins
  MUST execute commands via the injected `CommandExecutor` (`self.executor.execute(&CommandSpec…)`), never by spawning a
  process locally — the executor is the only component that routes to the correct host (local or SSH)."_ This is a
  required deliverable, not conditional.
- **No API / wire / OpenAPI / config change.** `ShellHookConfig` is untouched; no endpoint or wire payload involved; no
  regen.
- **No ADR** — bugfix restoring an existing abstraction (the executor boundary), using the in-repo systemd-hook
  pattern. No architectural decision.

## Out of scope / deferred

- Any change to `uptrakit_command::run_command_with_shell` or the `command.rs` streaming helpers (still used elsewhere).
- Adding local-vs-remote introspection to `HostRuntime`/`CommandExecutor` (not needed; the executor already routes
  correctly — the plugin just has to use it).
- The separate interactive-PTY hook finding (`update.rs` forwarding runtime) — owned by
  `2026-07-11-interactive-pty-lifecycle-design.md`; unrelated mechanism.

**Forward-looking note (for the next author):** hooks are safe from the controller-side `NoopCommandExecutor` today only
because real hook execution runs on the agent (`agent-core/update.rs` `run_pre_hook_plugins`/`run_post_hook_plugins`,
always with `LocalCommandExecutor` or `SudoAwareCommandExecutor`), and the `PreUpdateHook`/`PostUpdateHook`
`ConfigTestKind`s currently fall into the unimplemented `_ =>` arm in `agent-core/src/config_test.rs` (never construct
`ShellHookPlugin`). If those config-test kinds are ever wired up, they MUST be handed a real routing executor — never a
Noop — or the hook will fail with `PluginError::InstallFailed`. Test 5 encodes this expectation.
