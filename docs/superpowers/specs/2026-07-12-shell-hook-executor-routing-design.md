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

**Local-host invariance (the common case) is airtight, not just documented.** In production the local runtime is
`SudoAwareCommandExecutor(LocalCommandExecutor)`, so routing through the executor now runs `apply_sudo(spec)` on every
hook where the old free-function path did not. But `apply_sudo` cannot add sudo here for **two** independent reasons:
(1) the production hook spec has `privileged: false`, so `apply_sudo` short-circuits at the `!spec.privileged` guard
(`sudo.rs:165`) and returns `spec.clone()` before any `match`; and (2) even the harder `privileged: true` case forwards
Shell specs unchanged via the `CommandMode::Shell { .. }` arm (`sudo.rs:195`, emitting only a `tracing::warn!`). That
harder branch is already covered by the existing `privileged_shell_mode_passes_through_unchanged` test (`sudo.rs:428`).
So the "no sudo prefix on Shell specs" property is a **tested** invariant of `SudoAwareCommandExecutor` (production
relies on the even-earlier guard), not a documentation assertion — re-testing it in the shell plugin would only
duplicate that coverage (and test another component's contract), so this spec does not.

## Approach

Route the hook command through the injected executor, exactly as the systemd hook does. The **production** fix is fully
contained in `crates/plugins/hooks/shell/src/plugin.rs` — no new production types, no config change, no change to
`uptrakit_command`. (The regression test adds one shared **test-only** double to `infrastructure-core::testing`; see
Tests.)

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
`self.executor`). The exit-code handling stays **byte-for-byte identical**; the changes are the invocation line plus a
small **import diff** (do not undersell it):

- Add `use uptrakit_command::CommandSpec;` — `plugin.rs:4` currently imports **only** `CommandExecutor` from
  `uptrakit_command`, and the new body names `CommandSpec::shell_with`.
- `rootcause::report!` and `uptrakit_command::CommandError` stay **fully-qualified** exactly as today (`plugin.rs:64,67`)
  — no `use rootcause::prelude::*;` is added, matching the current file's style.
- The method takes **no** `#[tracing::instrument]` — it keeps the current free fn's inline `tracing::debug!` only, so
  instrumentation is unchanged.

```rust
/// Run a shell hook command through the host runtime's injected [`CommandExecutor`],
/// so it targets the correct host (local or SSH), not the agent's own machine.
///
/// Deliberately uses the non-interactive [`CommandExecutor::execute`]; shell hooks
/// get no PTY/stdin (stdin is `/dev/null`, as before this change). Interactive/PTY
/// hooks are tracked separately in `2026-07-11-interactive-pty-lifecycle-design.md`.
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
            Err(rootcause::report!(
                uptrakit_plugin_infrastructure_core::PluginError::InstallFailed(format!(
                    "shell hook command failed: {e}"
                ))
            ))
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

The regression test needs a double that **records each `CommandSpec`** and returns a **configurable
`uptrakit_command::Result<CommandOutput>`** (so a test can make it yield `Ok(CommandOutput { exit_code: 0, .. })`,
`Err(CommandError::CommandFailed(1))`, or `Err(CommandError::UnsupportedShell(..))`). Matching the real contract — where
non-zero exit is an `Err(CommandFailed)`, not an `Ok` — is essential; a double that returns `Ok(exit_code: 1)` would not
exercise the extraction path.

**Do not hand-roll a private mock.** The canonical home for shared `CommandExecutor` doubles is
`crates/plugins/infrastructure/core/src/testing.rs` (feature `testing`), which already ships `FixedOutputExecutor` and
`RoutedOutputExecutor` and is consumed by other plugin crates via a `features = ["testing"]` dev-dependency. Neither
existing double records the spec **or** returns a caller-supplied `Err`, so extend that module rather than reinventing a
fourth private mock (the Docker plugin's `MockCommandExecutor` at `releases/docker/src/tests.rs:24-53` is exactly the
duplication to stop propagating). Add a new `RecordingExecutor` there:

- Captures every `CommandSpec` passed to `execute`/`execute_quiet` into a `parking_lot::Mutex<Vec<CommandSpec>>` (both
  methods record, and both return the same configured outcome — a uniform reusable double, not a poison method), exposed
  via a `recorded() -> Vec<CommandSpec>` accessor (`CommandSpec` derives `Clone`, so `.clone()`-into-`Vec` is trivial;
  `#[async_trait]` per the trait). **`parking_lot` is not yet a dependency of `infrastructure-core`** — the `testing`
  feature is empty (`testing = []`) and the module's current deps (`async-trait`, `tokio`, `uptrakit-command`,
  `rootcause`) are all non-optional, so add `parking_lot = { workspace = true }` to
  `crates/plugins/infrastructure/core/Cargo.toml`'s `[dependencies]` (non-optional, mirroring those) before `testing.rs`
  can name `parking_lot::Mutex`.
- Returns a caller-supplied result. A constructor cloning a fixed `Result` is awkward (`CommandError` is not `Clone`, so
  a stored `Result<CommandOutput>` cannot be — `CommandOutput` itself is `Clone`), so store a boxed result-producing
  closure
  (`Box<dyn Fn() -> uptrakit_command::Result<CommandOutput> + Send + Sync>`) and expose constructors such as
  `RecordingExecutor::ok(exit_code)`, `::failed(code)` (→ `Err(CommandError::CommandFailed(code))`), and
  `::erroring(|| Err(...))` for the transport-error cases. Unlike the sibling doubles (`FixedOutputExecutor`
  /`RoutedOutputExecutor`), whose ctors return `Arc<dyn CommandExecutor>`, these constructors return `Arc<Self>` — the
  test needs a typed handle to call the inherent `recorded()` after injection, and `.recorded()` is unreachable through
  an `Arc<dyn CommandExecutor>`. The `Arc<Self>` coerces to `Arc<dyn CommandExecutor>` at the injection site
  (`StandardHostRuntime::new`), so it still satisfies the executor slot.

The shell plugin's test module then consumes it via a new dev-dependency
`uptrakit-plugin-infrastructure-core = { workspace = true, features = ["testing"] }` — benefiting every hook/plugin
crate that later needs spec-recording, not just this one.

Tests:

1. **Routing regression (the HIGH):** run a pre-hook through the recording double (configured `Ok(exit_code: 0)`);
   assert exactly one captured spec, and that its `mode` is `CommandMode::Shell { command, shell }` carrying the
   configured command + `config.shell`. `CommandMode` and `CommandSpec.mode` are `pub`, so a cross-crate
   `matches!(spec.mode, CommandMode::Shell { .. })` (plus field equality) asserts directly — no `Debug`/`resolve()`
   round-trip needed. Proves the command went through `executor.execute`, not a local spawn.
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

- **Doc comment** on the reworked `run_shell_command` method (shown in §1). The `# Errors` section is not strictly
  mandated — the method is private to the crate, and the coding-standards `# Errors` rule is a _should_ for public /
  shared-crate APIs — but it is included as consistent with the surrounding practice. Also add a one-line note on the
  struct stating the hook runs through the injected `CommandExecutor` so it targets the correct host (local or SSH) —
  replacing the stale `plugin.rs:47-51` comment that references the bypassed `run_command_with_shell` utility.
- **`docs/development/plugin-guidelines.md`** — the `## Update Lifecycle Plugins` section already exists (it documents
  the pre-hook `proceed`/`abort` and post-hook non-fatal contracts). Add one invariant line, matching that section's
  plain `--` code-span bullet style (no bold lead-in): `Lifecycle hook plugins execute commands via the injected
  CommandExecutor (self.executor.execute(&CommandSpec…)) -- never by spawning a process locally, since the executor is
  the only component that routes to the correct host (local or SSH).` Required deliverable, not conditional.
- **`docs/development/update-hooks.md`** — the built-in-hook reference that the plugin-guidelines section links to. Add
  the same routing invariant here (this is the canonical home for `hook_shell`/`hook_systemd` behavior), so the rule
  lives with the hook docs and not only in the general guidelines.
- **`crates/plugins/infrastructure/core/src/testing.rs`** — the new `RecordingExecutor` (Tests §) is a test-only shared
  helper; document it with a doc-comment matching `FixedOutputExecutor`/`RoutedOutputExecutor`. No doc-catalogue entry
  (test infrastructure, not a public runtime API).
- **`agent-core/src/config_test.rs`** — add a one-line code comment on the unimplemented `_ =>` arm that today swallows
  the `PreUpdateHook`/`PostUpdateHook` config-test kinds, e.g. `// PreUpdateHook/PostUpdateHook intentionally
  unimplemented — wiring these up requires a real routing executor, never a Noop (see shell-hook routing spec).` This
  puts the forward-looking warning (below) where the next author will actually see it, not only in a spec they may never
  read.
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
