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
`self.executor`), mirroring `SystemdHookPlugin::run_systemctl`:

```rust
async fn run_shell_command(
    &self,
    command: &str,
    shell: HookShell,
    output_tx: &UpdateOutputSender,
) -> Result<i32> {
    let spec = CommandSpec::shell_with(command, shell);
    let output = self.executor.execute(&spec, output_tx).await.map_err(|e| {
        report!(uptrakit_plugin_infrastructure_core::PluginError::InstallFailed(format!(
            "shell hook command failed: {e}"
        )))
    })?;
    tracing::debug!(exit_code = output.exit_code, output_len = output.output.len(), "shell hook completed");
    Ok(output.exit_code)
}
```

Call sites at `plugin.rs:98` / `plugin.rs:139` become `self.run_shell_command(cmd, self.config.shell, &plugin_tx)`.

**Exit-code semantics simplify and stay correct.** `CommandExecutor::execute` returns `Ok(CommandOutput { exit_code })`
for a non-zero exit (it does **not** map non-zero to `Err` — confirmed by systemd checking `result.exit_code != 0`
after an `Ok`). So the method returns `Ok(output.exit_code)` and the existing `on_failure`/`should_proceed` logic in
`execute_pre_hook` (`plugin.rs:78-107`) and `execute_post_hook` (`plugin.rs:109-150`) is unchanged — it already
branches on the returned exit code. Only a _real_ transport/spawn error (SSH down, shell unsupported) becomes an `Err`
→ `PluginError::InstallFailed`. This drops the current `CommandError::CommandFailed(code) → Ok(code)` special-case,
which is no longer reachable (that error came from the local-spawn path; the executor already surfaces a clean
exit_code).

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
`CommandSpec` passed to `execute()` into a `parking_lot::Mutex<Vec<CommandSpec>>` and returns a configurable
`CommandOutput { exit_code }`. (The Docker plugin's `MockCommandExecutor`, `releases/docker/src/tests.rs:24-53`, is the
shape to copy, extended to record instead of discard.) `parking_lot` is a workspace dep; `#[async_trait]` per the trait.

Tests:

1. **Routing regression (the HIGH):** run a pre-hook through the recording double; assert exactly one captured spec,
   and that its `mode` is `CommandMode::Shell { command, shell }` carrying the configured command + `config.shell`.
   Proves the command went through `executor.execute`, not a local spawn. _(If `CommandMode`/its fields are not `pub`
   for cross-crate assertion, assert via the spec's `Debug`/`resolve()` output — resolve to `(shell, ["-c", wrapped])`
   and assert the wrapped string contains the command; implementer picks whichever is public.)_
2. **Post-hook routes too:** same assertion via `execute_post_hook`.
3. **Non-zero exit → `on_failure` respected:** double returns `exit_code = 1`; with `on_failure: true`, pre-hook
   result `should_proceed == false`; a second case with `on_failure: false` → `should_proceed == true`. Confirms the
   exit code is read from `CommandOutput`, not swallowed.
4. **Exit 0 → proceed:** double returns `exit_code = 0` → `should_proceed == true`.
5. **Transport error → `PluginError`, not a silent success:** double returns `Err(CommandError::…)`; assert the hook
   returns an `Err` (not `Ok(should_proceed = true)`). Guards the dropped `CommandFailed → Ok(code)` special-case from
   masking a genuine SSH/spawn failure as success.

Keep the existing `LocalCommandExecutor` echo/exit tests as end-to-end smoke coverage (unchanged).

No `start_paused` — no `tokio::time` API is used.

## Documentation deliverables

- **Doc comment** on the reworked `run_shell_command` method (and a one-line note on the struct) stating the hook runs
  through the injected `CommandExecutor`, so it targets the correct host (local or SSH) — replacing the stale
  `plugin.rs:47` comment that references the bypassed utility.
- **`docs/development/plugin-guidelines.md`** — if a hook-authoring section exists, add one invariant line: _"Lifecycle
  hook plugins MUST execute commands via the injected `CommandExecutor` (`self.executor.execute(&CommandSpec…)`), never
  by spawning a process locally — the executor is the only component that routes to the correct host."_ If no such
  section exists, state "no external doc surface" and skip (the doc comment carries it). Verify during implementation;
  do not invent a new section solely for this line.
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
