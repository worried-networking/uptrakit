# Interactive Update PTY Lifecycle Hardening — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — three HIGH findings in one subsystem: "Interactive update dispatch
blocks the agent event loop and can deadlock it permanently", "ForwardingInteractiveExecutor gives the PTY to the
first execute() caller", "Interactive PTY child process is orphaned when the update pipeline times out".

## Problem

The interactive update path (`crates/shared/agent-core/src/update.rs` + `client.rs`,
`crates/core/agent-runtime/src/lib.rs`, `crates/shared/command/src/interactive.rs`) has three coupled lifecycle
bugs. PHS updates dispatch `interactive: true` by default, so all three sit on a hot path.

1. **Event-loop blocking → permanent deadlock.** `handle_controller_message` inline-awaits
   `handle_execute_update` → `start_update` → `spawn_update_task` → `execute_update_interactive`, which blocks on
   `channels_rx.await` until the plugin's first `execute()` call — i.e. after version detection (backon retries),
   pre-hooks, and the attestation check. During all of that, `output_rx` (bounded 100) sits unpolled inside the
   not-yet-returned `InFlightUpdate`. A pre-hook streaming >100 output lines fills the channel; the pipeline
   blocks in `send_output`; the plugin's `execute()` is never reached; the oneshot never resolves;
   `channels_rx.await` never returns — and the timeout arm can't rescue it because it too calls `send_output` on
   the same full channel. Agent event loop wedged until process restart. Even without the deadlock, inline
   blocking for the pre-stage duration violates the crate's own `spawn_background` rule and starves WS keepalive.
2. **PTY handed to the wrong command.** `execute_update_interactive` funnels the *entire* pipeline (detect,
   pre-hooks, update, post-hooks) through the forwarding runtime; `ForwardingInteractiveExecutor::execute()`
   promotes the **first** caller (`channels_tx.lock().take()`). A systemd pre-hook's `systemctl stop` claims the
   PTY; the real update then runs non-interactively; the user's xterm.js attaches to a dead PTY; typed stdin goes
   to whatever was intercepted. The doc comment ("Pre/post hooks and version detection still run
   non-interactively") is factually wrong.
3. **Orphaned PTY child on pipeline timeout.** The outer `tokio::time::timeout` drops the pipeline future, which
   merely stops polling `handle.completion` — dropping a `JoinHandle` **detaches** the task. `drive_interactive_session`
   keeps running and owning the child; its internal deadline is `spec.timeout`, which is `None` for update
   commands (`CommandSpec::shell`), so the internal `kill_process_group` path never fires. A hung PHS script or
   apt holding the dpkg lock runs indefinitely with no signal path; subsequent updates fail on lock contention.

Verified during design (structure mapping): no test exercises `execute_update_interactive`,
`ForwardingInteractiveExecutor`, or the interactive `InFlightUpdate` wiring — zero direct coverage today.

## Approach

Three coordinated fixes, smallest seams that make the documented contracts true. The `ForwardingInteractiveExecutor`
wrapper pattern (no `Plugin` trait changes) is preserved throughout.

### Fix 2 first (it shapes fix 1): target the promotion, one seam parameter

`execute_update` gains one optional parameter instead of a fork or a second full runtime thread-through:

- `execute_update(payload, runtime, output_tx, early_result_tx, update_exec_runtime: Option<Arc<dyn HostRuntime>>)`
  (an `Option` seam, defaulting to `None` for the non-interactive path). Precision note: the actual plumbing
  point is one level down — the private `execute_update_pipeline` gets the same seam threaded through; 4 call
  sites total need the new parameter (`client.rs`, `update.rs:1179`, two in-module tests). Add a doc comment on
  the parameter explaining the seam (hooks/detection use `runtime`; only the update command uses the override).
  `execute_update`'s existing `#[tracing::instrument(skip_all, …)]` needs no change (`skip_all` covers the new
  param).
- The pipeline uses `runtime` (the plain inner runtime) for `detect_current_version`, `run_pre_hook_plugins`,
  `run_post_hook_plugins` — and `update_exec_runtime.unwrap_or(runtime)` **only** for `execute_plugin_update`.
- `execute_update_interactive` passes the original runtime as `runtime` and the forwarding runtime as
  `update_exec_runtime`. Hooks and detection can no longer steal the PTY regardless of which executor methods
  they call; the promotion targets exactly the update command. The doc comment becomes true; fix it to match.

Rejected: forking `execute_update` into an interactive variant (~180 duplicated lines); an "armed" flag on the
forwarding executor toggled mid-pipeline (requires the shared pipeline to know about the wrapper — layering leak).

### Fix 1: return immediately, resolve channels in the event loop

- `execute_update_interactive` no longer awaits `channels_rx` — it returns
  `InteractiveUpdateHandle { handle, channels_rx: oneshot::Receiver<InteractiveChannels> }` immediately after
  spawning the pipeline.
- `InFlightUpdate` replaces its pre-resolved `stdin_tx`/`signal_tx`/`attention_rx` initialization with
  `channels_rx: Option<oneshot::Receiver<…>>` (fields stay; they start `None`).
- `poll_in_flight_update` (agent-runtime) gains a select arm on `channels_rx` using the same take/select/put-back
  *shape* the loop already uses for `attention_rx` (borrow-checker precedent in place — but `channels_rx` is a
  `oneshot`, so it needs its own small helper, not a reuse of `recv_attention_rx`, which is mpsc-based). On
  resolution it fills `stdin_tx`/`signal_tx`/`attention_rx`. On oneshot error (pipeline ended without promotion)
  it clears the receiver and emits a `tracing::warn!` with the update id — not silent (the update was announced
  as interactive; the log line is the operator's breadcrumb).
- Add `#[tracing::instrument(skip_all, fields(update_history_id = …))]` to `execute_update_interactive` for
  observability parity with `execute_update` — its role changes materially (returns immediately) and it is
  uninstrumented today.
- `start_update` therefore returns `InFlightUpdate` without waiting for promotion: `output_rx` is polled by the
  event loop from the moment the pipeline starts → the bounded-channel deadlock is structurally gone, and the
  event loop never blocks on hooks/detection/attestation.
- `UpdateStarted.interactive` becomes **intent-based**: `payload.interactive && executor.supports_interactive()`,
  both known synchronously. Controller-side reality, verified: `update_history.interactive` is written at
  dispatch time (`caller_flag || config_prefers_interactive`) **and then unconditionally overwritten at
  UpdateStarted-claim time from the wire value** (`claim_or_replay_update_start_db`) — so the persisted truth
  today is the agent's confirmed flag, and this change makes the overwrite carry intent instead of confirmation.
  The frontend already treats the live `AdminEvent::UpdateStarted.interactive` as authoritative. Accepted drift,
  stated explicitly: if PTY setup fails *after* `UpdateStarted` (or the plugin never executes through the
  update-exec runtime), DB column and UI badge say interactive with no live PTY; no correction message is added
  (YAGNI — with fix 2 the promotion targets exactly the update command, so the window is the
  plugin-never-executes / PTY-allocation-failure edge, and the `tracing::warn!` above is the diagnostic). No
  wire-protocol change (`UpdateStarted.interactive` already exists).
- `handle_update_stdin_data` before resolution: `stdin_tx` is `None` → same behavior as today's
  channels-unavailable case (warn + drop). Stdin typed before the update command starts was never deliverable —
  **but** intent-based `UpdateStarted` widens the window in which the UI *looks* live while stdin drops silently
  (today the badge appears only after promotion; now it appears at pipeline start, spanning detect + pre-hooks).
  Because of that, one frontend change is **in scope**: gate the terminal's input-enabled state (not its
  visibility) on evidence the PTY is live — the first PTY output frame or the existing `stdin_attention` signal —
  rendering read-only until then. Without this the fix ships a worse interactive UX than today; with it, the
  operator sees hook output stream in a read-only terminal and input unlocks exactly when deliverable.
  Unlock-signal guarantee to verify during implementation: a prompt is itself PTY output (unlocks on the first
  frame), and `stdin_attention` exists to flag stdin-read-wait — but confirm attention fires even for a command
  that reads stdin with zero prior output; if that cannot be guaranteed, the gate needs a third unlock trigger
  (otherwise the input-gate trades silent-drop for a can't-unlock hang on pathological prompts).
- Reconnect replay keeps the **live-reality** derivation (`stdin_tx.is_some()`), not the intent flag: replay's
  job is to reflect current state, and the controller unconditionally overwrites the persisted flag from replayed
  `UpdateStarted` — an intent-based replay could resurrect an interactive badge on an update whose PTY is already
  dead. Intent on first send, live state on replay.
- **`agent-ssh-runtime` must be updated in the same change** (found during review — `start_update` is the shared
  primitive for both runtimes, and the SSH side would silently regress otherwise): `handle_execute_update_ssh`
  currently does `in_flight.stdin_tx.take()` / `.signal_tx.take()` immediately after `start_update` returns,
  which only works because of the blocking await this fix removes. `SshInFlightUpdate` gains the `channels_rx`;
  its spawned forwarder task awaits resolution **inside the task** (fine there — it is not the event loop) before
  wiring stdin/signal forwarding, without blocking its output forwarding (structure the forwarder so output
  streaming starts immediately and channel wiring attaches on resolution; the existing forwarder `select!` shape
  already decouples output from stdin wiring — verified feasible). Reconnect replay keeps the live-reality
  `stdin_tx.is_some()` derivation on both runtimes (see the replay rule above). Mechanical fallout: the
  `SshInFlightUpdate` struct literals in tests (`make_ssh_in_flight()` and the lib.rs test constructor) gain the
  new field.

Rejected: draining `output_rx` into a buffer while awaiting the oneshot (fixes the deadlock, keeps the event-loop
blocking and the `spawn_background`-rule violation); routing `ExecuteUpdate` through `spawn_background` (it needs
a long-lived `InFlightUpdate`, not a one-shot result — the existing special-casing is correct).

### Fix 3: bound and kill the PTY child on cancellation

Two complementary mechanisms (each alone is insufficient — verified against `interactive.rs`):

- **Deadline propagation:** `ForwardingInteractiveExecutor` stores `payload.timeout` and, when promoting, sets
  `spec.timeout = Some(payload.timeout)` on the promoted `CommandSpec` if the plugin left it `None`. The existing
  internal deadline path in `drive_interactive_session` then fires `kill_process_group` (correct group semantics —
  kills grandchildren/subshells) at most `payload.timeout` after the command starts, even if every outer guard is
  dropped. Bounded backstop, reuses the shipped kill path.
- **Immediate cleanup on cancellation:** expose `child_pid` on `InteractiveHandle` (plain data field; captured
  before the session task spawns — verified feasible), and promote `kill_process_group` to a `pub` fn (or public
  wrapper) in the `command` crate — it is private today and `ForwardingInteractiveExecutor` lives in agent-core,
  so this is a small deliberate public-API addition. `ForwardingInteractiveExecutor::execute` wraps its
  `handle.completion.await` in an RAII guard holding `completion` (for `.abort()`) and `child_pid`; on drop
  without defusal (i.e. the outer pipeline timeout cancelled us) it aborts the session task and calls
  `kill_process_group(child_pid)`. `kill_on_drop(true)` alone is insufficient — it SIGKILLs only the direct
  child, not the group; the guard uses the group kill. Defuse on normal completion.
  In-tree precedent for this Drop shape: `DockerSocketProxy::drop`
  (`crates/plugins/releases/docker/src/docker_proxy.rs`) already combines `JoinHandle::abort()` with a
  synchronous syscall (`remove_file`) in `Drop` — follow its discipline. Two constraints: the guard's `drop()`
  stays strictly synchronous (`.abort()` and `rustix` group-kill are sync; no
  `unwrap`, mirror `kill_process_group`'s existing `pid <= 0` early-return), and any defusal state uses
  `parking_lot` with guard-drop-before-await discipline. Residual PID-reuse TOCTOU, accepted and documented: the
  guard can fire after the group already exited, and a recycled pgid could in principle receive the SIGKILL; the
  window is the instant between normal exit and defusal, the target is a process *group* id freshly vacated, and
  the existing deadline path carries the same theoretical window — accepted risk, noted in the guard's doc
  comment rather than engineered around. One invariant to **verify before** promoting the group-kill to `pub`:
  the interactive child must be spawned into its **own** process group (`process_group(0)`/`setsid`) before
  `child_pid` is captured — if the child shared the agent's pgid, the group-kill's blast radius would include the
  agent itself. The existing deadline path already group-kills, which implies the invariant holds today; confirm
  it in `interactive.rs` and assert it in the guard's doc comment.

Rejected: guard-only (child outlives nothing, but if the guard itself is never dropped — e.g. a future
refactoring detaches differently — the deadline backstop still bounds it); deadline-only (orphan survives up to
`payload.timeout` past the pipeline timeout — the wire default is 7200s, so up to two hours of held
package-manager locks).

## Tests

All in-crate, no PTY-in-CI assumptions beyond what `interactive.rs` tests already handle (skip if PTY
unavailable).

Prerequisite, called out because it does **not** exist today (verified): tests 1–2 need a stub lifecycle-hook
plugin. The registry's `test-support` feature ships only `ReleaseFetcher` descriptors (`lifecycle_hook: None` on
all), and agent-core does not enable `test-support` anywhere. Deliverable: add a minimal hook-capable descriptor
to `test_support.rs` and wire the feature into agent-core's dev-dependencies — net-new test infrastructure, not
just invocation of existing scaffolding.

1. **Deadlock regression:** interactive update whose pre-hook emits >100 output lines (mock hook plugin through a
   stub runtime); assert the pipeline reaches the update command and completes, and that `start_update` returns
   before promotion (e.g. assert `InFlightUpdate` is obtained while a slow pre-hook is still running).
2. **PTY targeting:** stub runtime recording which executor received `execute_interactive`; pipeline with a
   pre-hook that calls `execute()`; assert the pre-hook ran on the inner executor and the update command got the
   promotion (channels resolve with the update command, not the hook).
3. **Channel resolution select arm:** drive `poll_in_flight_update` with a manually constructed `InFlightUpdate`
   holding a pending `channels_rx`; resolve it; assert `stdin_tx`/`signal_tx`/`attention_rx` populate and stdin
   forwarding works post-resolution (extends the existing manual-`InFlightUpdate` test precedent in
   agent-runtime).
4. **Orphan kill:** real PTY test (skip-if-unavailable, like existing `interactive.rs` tests): spawn an
   interactive command that ignores SIGTERM and sleeps; cancel the wrapping future (drop/timeout); assert the
   process group is gone (poll `kill(pid, 0)`). Plus a unit test that promotion sets `spec.timeout` from
   `payload.timeout` when the plugin left it `None`.
5. Time-API rule: tests that wait on real child processes poll with bounded retries; any test using
   `tokio::time` APIs uses `start_paused = true` **only** if it has no real-DB/PTY component (snapshot rule) —
   the PTY tests here use real processes, so no paused time.

## Documentation deliverables

- Fix the false doc comment on `ForwardingInteractiveExecutor` / `execute_update_interactive` (hooks/detect now
  genuinely non-interactive; describe the seam and the channel-resolution flow).
- `InFlightUpdate` field docs (channels start `None`, resolved by the event loop).
- `docs/api/wire-protocol.md`: no message shape changes, but the `UpdateStarted.interactive` semantics note
  (intent-based, matches dispatch-time resolution) belongs wherever that field is documented — update the
  existing description if it states "confirmed after PTY allocation".
- No new ADR (subsystem-internal lifecycle fixes; no architectural decision).

## Out of scope / deferred

- Shell hook plugin bypassing `CommandExecutor` (separate audit HIGH — different fix locus, next spec).
- A correction message for the PTY-setup-failure-after-UpdateStarted edge (accepted drift, documented above).
- Wire-protocol changes (none needed).
- Batch update path (`interactive` always false there — unchanged).
- Controller-side UI changes beyond the single input-enable gating above (badge/list rendering unchanged).
- Making `handle_update_stdin_data` buffer pre-resolution stdin (never deliverable semantics today; YAGNI).
