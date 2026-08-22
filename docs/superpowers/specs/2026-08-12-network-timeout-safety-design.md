# Network & Timeout Safety — Design

**Date:** 2026-08-12
**Status:** Design (pending plan)
**Amended:** 2026-08-22 — M1.6/M1.7 restructured (controller-side registry removed; agent-side guard is the sole
overlap-prevention mechanism). See § Amendment log.
**Scope:** command execution (`crates/shared/command/`), agent operations (`crates/shared/agent-core/`, `crates/core/agent-runtime/`,
`crates/core/agent-ssh-runtime/`), controller-side reaper/dispatch (`crates/ui/web-api/`, `crates/ui/web-api-queries/`,
`crates/core/scheduler-runtime/`), sudoers generation (`crates/core/agent-ssh-runtime/src/operations/`), plugin trait surface
(`crates/plugins/infrastructure/core/`), service-sdk keepalive, openapi-client SSE streams, and the documentation set listed in
§ Documentation deliverables. Three ordered milestones; each becomes its own plan (or plan series) and lands in order.

> All `file:line` references in this spec are locator hints captured at authoring time (commit `adc48ca57`); re-grep the symbol
> before editing — line numbers drift.

## Problem

`apt-get update`, dispatched by the scheduler as part of a version check, can hang for hours on a managed host. Audit of the full
dispatch chain found that **no layer applies a deadline**:

1. The scheduler's `detect_version` executor sends `CheckVersions` fire-and-forget and returns `Ok(())` immediately
   (`crates/core/scheduler-runtime/src/executors/detect_version.rs:111-116`); its per-task `TASK_EXECUTION_TIMEOUT` (7200 s,
   `scheduler.rs:24`) bounds only the controller-side send, never the agent-side work.
2. The agent dispatches the message to an unbounded `tokio::spawn` with no retained handle, no dedup, and no concurrency cap
   (`spawn_background`, `crates/shared/agent-core/src/client.rs:285-294`).
3. `refresh_package_indexes` awaits the apt plugin's `refresh_package_index()` with no timeout
   (`crates/shared/agent-core/src/version_check.rs:356-399`).
4. The apt plugin builds `CommandSpec::exec("apt-get", ["update", "-q"]).privileged()` with `timeout: None`
   (`crates/plugins/package-managers/apt/src/update.rs:15-37`).
5. `LocalCommandExecutor` has a timeout mechanism (`apply_timeout`, `crates/shared/command/src/executor.rs:140-152`) but the
   `None` branch is a bare `fut.await`; `run_command_exec_impl` awaits `child.wait()` forever
   (`crates/shared/command/src/command.rs:62-195`).

`CommandSpec.timeout` exists (`crates/shared/command/src/types.rs`) but has **zero production call sites** — the only runtime
writer is the interactive-update promotion (`crates/shared/agent-core/src/update.rs:1143-1147`). Verified by repo-wide grep for
`with_timeout(`: all remaining hits are tests, the builder definition, and unrelated `u32`-seconds surface builders.

Compounding defects:

- **A timeout kill would not clean the host.** Non-interactive spawns use `kill_on_drop(true)` with no process group
  (`command.rs:71-79`): dropping the future SIGKILLs only the direct child (`sudo`), orphaning `apt-get`. And the unprivileged
  agent (architecture invariant: agents run unprivileged) cannot signal root-owned processes at all — `kill(2)` requires the
  sender's uid to match the target's real or saved uid, and sudo runs as full root.
- **The local timeout path already hangs forever when the kill fails.** On deadline the interactive driver calls
  `kill_process_group` (EPERM against root members, logged and ignored, `interactive.rs:388-391`) and then awaits
  `child.wait()` unboundedly (`crates/shared/command/src/interactive.rs:268-277`) — the completion future never resolves and the
  update never reports.
- **Nothing controller-side notices.** `VersionCheckResults` is awaited by nobody
  (`crates/ui/web-api/src/routes/software_items/version_check_dispatch.rs:298`); the ADR-0024 reaper covers only
  `update_history` rows in `InProgress` with non-null `started_at` (`crates/ui/web-api-queries/src/queries/update_reaper.rs:29-56`)
  — version checks and discovery have no reaper, and `Pending` rows are never reaped, so a dispatch the agent never acts on
  blocks the host's update slot forever (the partial unique index `uix_update_history_host_active` treats `Pending` as active).
- **Each scheduler tick stacks another hung process** — no dedup at either end.
- SSH mode is identical: `PosixSshCommandExecutor` honours `spec.timeout` (`crates/core/agent-ssh-runtime/src/ssh_executor.rs:110-140`)
  but no plugin sets it, and non-interactive SSH exec requests no PTY (`ssh_transport.rs:275,325`), so even tearing the channel
  down would not kill the remote command (no SIGHUP without a controlling terminal; the process dies only on SIGPIPE at its
  next write — a network-stalled `apt-get` writes nothing).

## Goals

1. Every command execution path carries a deadline; a hung command becomes a reported, typed failure within minutes, not hours.
2. Hosts stay free of stuck processes: kill what we can kill directly; back-stop what we cannot (root-owned) with a
   host-local mechanism that has root authority (sudoers `TIMEOUT=`).
3. No duplicate pile-up: a wedged operation never causes the scheduler to stack more copies of itself.
4. Failures are legible: timeout kills are distinguishable from real command failures in results, logs, and the UI —
   **for operations that return results.** A fire-and-forget operation whose result never arrives is bounded and
   non-stacking (M1.7) but surfaces only in logs: the controller result handler today drops errored results without
   persistence or SSE (log + audit error count only), so there is no failure channel to feed. Building that channel
   (persist per-item error + SSE + UI display, covering _all_ errors, not just timeouts) is descoped to the follow-up
   epic `uptrakit-async-op-failure-surface` (2026-08-22 amendment).
5. Fleet safety during rollout: nothing in this spec may brick sudo on a host, and the kill/backstop milestone is
   canary-deployed to one host before fleet rollout.

## Non-goals / rejected alternatives

- **SSH session teardown + reconnect on timeout** — rejected by owner: closing a non-PTY channel does not reliably kill the
  remote process (see § Problem), and the pool multiplexes channels over one session
  (`crates/core/agent-ssh-runtime/src/ssh_pool.rs`), so teardown collateral-damages concurrent channels. The remote
  `timeout(1)` wrapper and the sudoers backstop do the killing; the local deadline only bounds the await and closes the channel.
- **`timeout(1)` wrapping `sudo`** — useless: an unprivileged `timeout` cannot signal its root child (EPERM), and GNU timeout
  then waits for the child indefinitely, so the wrapper adds nothing. Wrappers apply to unprivileged commands only.
- **The inversion — `timeout(1)` inside the sudo allowlist (`sudo timeout -k 10 N apt-get update`)** — root-owned timeout
  can kill its group, so it works mechanically; rejected at design time (owner decision) in favour of sudoers `TIMEOUT=`.
  Honest rationale: with literal numeric tokens the wrapped entry opens no new injection surface, and a budget change means
  a sudoers resync under either mechanism — the real difference is the size of the diff to the command-match surface of a
  file that can brick sudo (a single `TIMEOUT=` option token on the existing `Cmnd_Spec` line vs. rewriting every entry's
  program+argument shape, where a later careless wildcard on the numeric field _would_ widen toward
  `sudo timeout N /bin/sh …`). Price accepted: the grandchild caveat and the ≥ 1.8.20 gate (§ M2.4, canary-verified).
- **One global sudoers `Defaults command_timeout` ceiling** — rejected in favour of per-command `TIMEOUT=`: a single number
  must cover both a 30-second index refresh and a legitimate 2-hour interactive dist-upgrade and gets both wrong; killing a
  mutating `apt-get install` mid-flight also risks wedging dpkg, whereas killing an index refresh is always safe.
- **Global settings-store / per-plugin-config timeout knobs** — rejected for now (owner): named constants + per-call override.
  The one exception is the generic shell plugin, whose commands are user-authored and legitimately slow (§ M1.5).

## Design principles

- **Three independent layers, three distinct jobs.** (a) _Report deadline_ (tokio-level): bounds the await, produces the typed
  failure the user sees. (b) _Kill mechanism_ (process-group signal or remote `timeout(1)`): best-effort cleanup by the agent
  itself. (c) _Root backstop_ (sudoers `TIMEOUT=`): host-local root authority reaps what the agent cannot. Any layer may fail
  without disabling the others. The honest framing for the backstop is: it reclaims the _process_, and (for index refreshes)
  the host; it is not a substitute for the report deadline.
- **The deadline lives inside the executing function.** A three-stage kill (SIGTERM → grace → SIGKILL → bounded reap) cannot be
  expressed from a cancelled outer future, and cancellation detaches (not aborts) the stdout/stderr reader tasks, leaking fds.
  `run_command_exec_impl` and `exec_command_streaming` own their own `select!`/`sleep_until` deadline and clean up via RAII.
- **Defaults are applied at the `apply_timeout` decision point, never written back into `CommandSpec`.** Mutating the spec at a
  wrapper layer (e.g. inside `apply_sudo`) would fill `timeout` before `ForwardingInteractiveExecutor` sees it and permanently
  disable the budget promotion at `update.rs:1143-1147`. A regression test alongside the existing
  `promotion_fills_timeout_and_survives_sudo_layer` pins this.
- **Timeout results are typed, never stringly inferred.** New error variants land in every sibling error enum that gains the
  failure mode, in the same change (variant + `#[error]` text + mapping arm + mapping test as one unit).

## Existing prior art this spec builds on (does not duplicate)

- **`Shell Hook Executor Routing`** — spec `docs/superpowers/specs/2026-07-12-shell-hook-executor-routing-design.md`, plan
  written (tracker: plan `NEW`). Fixes `hook.shell` executing user shell hooks on the controller instead of the SSH-managed
  target host. **Milestone 0 of this spec requires that plan to land first** — it is the funnel that makes hook commands
  reachable by spec-level timeout policy at all. This spec adds nothing to it.
- **`Agent Update-Gate Rejection Result`** — spec `docs/superpowers/specs/2026-07-12-agent-update-gate-rejection-result-design.md`,
  plan written. Makes the agent reply `UpdateResult(Failed)` when its update gate rejects a dispatch. Complementary to — not a
  substitute for — the `Pending`-row reaping in M0.4: the reply path needs a live, connected, healthy agent; the reaper covers
  dead/never-delivered dispatches. Note that spec's problem statement assumes the reaper already catches `Pending` rows in
  ~6 minutes; that assumption is wrong (verified against `update_reaper.rs:29-31` — `Pending` is explicitly left untouched).
  M0.4 makes the assumption true.
- **ADR-0024** (update liveness, `Interrupted` status, wall-clock reaper) — the M0.4/M1.8 reaper changes extend its model to
  `Pending` rows and per-row budgets; a new ADR records the extension (§ Documentation deliverables).

## Milestone 0 — safety prerequisites (live-bug fixes, no new behaviour)

Every item here is an independently reviewable fix for a defect that exists today. No user-facing timeout knob is introduced,
and no healthy path changes behaviour — M0.2 and M0.4 do introduce two new internal constants (`KILL_REAP_GRACE`,
`PENDING_DISPATCH_GRACE`) because bounding a today-unbounded hang requires a bound; both fire only on already-broken paths.

### M0.1 Sudoers validate-before-write

`write_sudoers_file` currently pipes content into the live `/etc/sudoers.d/uptrakit-<user>` via `sudo tee` and runs
`visudo -cf` only afterwards (`crates/core/agent-ssh-runtime/src/operations/sudoers.rs:261-312`). A syntactically invalid file
— e.g. a directive the host's sudo does not support — makes sudo refuse to run **for every user on the host**, and the agent
then cannot rewrite or delete the file. Restructure to: write to a temp path → `chmod 0440` → `visudo -cf <temp>` → `mv` onto
the final path; on validation failure, delete the temp file and fail the operation with the validator's output. The shell
installer already does exactly this (`scripts/pvehs/install/uptrakit-install.sh:78-86`) — mirror it. **Hard prerequisite for
M2.4**: no `TIMEOUT=` emission before this ships.

### M0.2 Bounded post-kill reap on the interactive path

On deadline, `drive_interactive_session` calls `kill_process_group(child_pid)` then `let _ = child.wait().await;`
(`crates/shared/command/src/interactive.rs:268-277`). When the group contains root-owned members, `killpg` EPERMs (logged at
`:388-391`) and the `wait()` never returns — the session never reports. Bound the reap:
`tokio::time::timeout(KILL_REAP_GRACE, child.wait())`; on expiry, return `TimedOut` carrying an `unkillable` marker (§ M2.2's
error shape; until M2.2 lands, a structured `warn!` + plain `TimedOut` suffices) and abandon the child. Standalone fix — the
current behaviour is a live hang.

### M0.3 Land `Shell Hook Executor Routing` (existing plan)

Ordering dependency only; see § Existing prior art. Milestone 0 is complete when that plan's commit is merged.

### M0.4 Reap `Pending` update rows

`update_history` rows are created `Pending` before dispatch and count as active for the one-update-per-host guard, but the
reaper touches only `InProgress` (`update_reaper.rs:29-56`). An agent that silently drops the dispatch (freeze/cooldown/
machine-id mismatch today — until the gate-rejection plan lands — or a crash, or a disconnect race) leaves the row `Pending`
forever, permanently blocking updates for that host. Extend `reap_overdue_updates` (or add a sibling query in the same module)
to also flip `Pending` rows older than `PENDING_DISPATCH_GRACE` (proposed: 600 s, wall-clock from `created_at`) to
`Interrupted`, with a distinct `REASON`/`RECOVERY_HINT` pair naming the never-started case. `Queued` stays untouched (batch
promotion owns it, per ADR-0024).

Split-brain guard: reaping a `Pending` row frees the host's active-update slot, so a dispatch that was _delivered but slow_
must not later start executing against a freed slot (would break the one-active-update-per-host invariant). Therefore the
`Pending` reap additionally requires evidence of non-delivery: the host's service connection has been absent for the full
grace window (connection state is controller-local and queryable). A late result arriving for an already-`Interrupted` row
follows the existing non-`InProgress` fallthrough/stale handling in the result path — the plan names the exact arm.
Documented residual: connection absence is evidence of non-_connectivity_, not non-delivery — an agent that received the
dispatch, kept executing across a disconnect, and reconnects after the reap can still overlap a newly started update. The
full close is reconnect reconciliation (agent reports its in-flight op set on reconnect; controller refuses to treat a
reaped row's slot as free while the agent claims it) — deferred to § Out of scope as a tracked follow-up. Plan-time
requirement: enumerate every writer of `update_history` status transitions (grep `ActiveModel`/`update_many` over the entity,
not the task file list) and confirm none races the new reap; the existing CAS-style status filters in the reaper's
`update_many` are the pattern to follow.

### M0.5 Documentation drift fixes

- `docs/api/wire-protocol.md:851` claims a 5-minute per-hook kill that has no implementing code anywhere. Remove the row in
  M0 (M1.4 reintroduces it as a true statement).
- `docs/development/command-executor.md` — the `CommandSpec` snippet omits `envs`, the builder table omits `.with_timeout`.
  Refresh the snippet against current source; compile-check every identifier in the touched blocks against the defining files.
- `crates/shared/command/src/types.rs:170` doc comment claims env forwarding via `sudo env NAME=VALUE …`, contradicting the
  implementation and the correct adjacent doc at `types.rs:85-88`. Fix the comment (doc-only).

## Milestone 1 — deadlines everywhere (no kill-behaviour change)

This milestone alone converts "hangs for hours" into "fails and reports within minutes". It is reversible by raising one
constant. No process is killed any harder than today (`kill_on_drop` still applies); M2 adds the kill/cleanup story.

### M1.1 Executor default timeout

`DEFAULT_COMMAND_TIMEOUT` (proposed: 600 s) applies whenever `spec.timeout` is `None`, for both `execute` and
`execute_quiet`. Two independent enforcement sites — the mechanism is **not** shared code:

- **Local:** `apply_timeout` (`executor.rs:140-152`) is a private helper used only by `LocalCommandExecutor`. It resolves the
  default at that decision point, but enforcement moves **inside** `run_command_exec_impl` in this same task (a `select!` on
  `sleep_until` alongside the reader joins) — an outer `tokio::time::timeout` cancellation detaches the spawned stdout/stderr
  reader tasks (§ Design principles), and M1.1 makes timeouts routine, so the leak must be closed here, not deferred to M2.1.
  On expiry (M1 semantics): abort both reader tasks, kill the direct child (today's `kill_on_drop` semantics — direct child
  only, best effort; EPERM against a root-owned `sudo` is expected and logged), return `TimedOut`. The reader `JoinHandle`s
  are held in an abort-on-drop guard so that _any_ outer cancellation (e.g. the M1.3 op deadline firing mid-command) also
  cleans up. Orphan lifecycle: aborting the readers drops the `ChildStdout`/`ChildStderr` fds, so a surviving orphan
  receives SIGPIPE/EPIPE at its next write and terminates then — a stalled-then-recovering process self-reaps at first
  output; only a process that never writes again persists until M2's kill/backstop. That fd-close is itself a kill vector,
  so it is **classified, not indiscriminate**: `CommandSpec` gains an abandonment policy (close-on-abandon, the default,
  for read-only/refresh commands vs drain-on-abandon for mutating commands — the same read-only/mutating classification
  M2.4 uses). For mutating specs (update/install pipelines set this via the M1.2 executor wrapper and plugin construction),
  abandonment hands the pipe ends to a detached drain-to-`/dev/null` task bounded by the op deadline, so a
  budget-exceeding package transaction is abandoned passively rather than SIGPIPE-killed mid-transaction. The policy binds
  **both** abandonment routes: the deadline-expiry arm _and_ the abort-on-drop guard (outer cancellation — e.g. the batch
  pipeline's own `tokio::time::timeout(payload.timeout, …)` at `client.rs:534`, the only bound on batch updates, firing
  mid-`apt-get install`) — a drain-on-abandon spec is never reader-aborted from either route. The external-cancellation
  test runs in both policy modes; the drain-mode assertion is that the child does _not_ receive EPIPE. Plan-time
  verification for the close path: assert the reader tasks hold the _only_ copies of the stdio handles (no `Child`-retained
  handle, no reader in `spawn_blocking` — `abort()` cannot interrupt a blocking `read(2)`), and remember `abort()` is
  asynchronous — tests await the aborted `JoinHandle`s before probing for EPIPE.
  M1.1 also logs a structured warning for any command completing above 80% of its budget (command
  identity + duration), so the 600 s default is tuned from data instead of re-guessing. M2.1 later upgrades this same site
  with the setsid group-kill escalation; the deadline/guard structure lands here.
- **SSH:** `PosixSshCommandExecutor::run_remote` (`crates/core/agent-ssh-runtime/src/ssh_executor.rs:104-140`) has its own
  hand-rolled outer `tokio::time::timeout` wrap and never calls `apply_timeout` (different crate, incompatible future types).
  It applies the same default when `spec.timeout` is `None`; M1.9 (Plan 2) later retires the outer wrap by threading the
  duration into `exec_command_streaming`. The interim (default enforced by the existing outer wrap) is acceptable: the
  double-timeout hazard only exists once M1.9's inner deadline exists, and SSH's outer wrap cancels a remote await, not
  locally-spawned reader tasks.

The default is a named constant in `uptrakit-command`, documented as the command-execution contract. It is resolved at the
decision point only — never written into the spec (§ Design principles; regression test mandated).

### M1.2 Budget plumbing for non-interactive updates (`BudgetForwardingExecutor`)

Without plumbing, M1.1 kills every non-interactive update longer than 10 minutes: plugins build their own specs with
`timeout: None`, and the only existing bound is the outer pipeline timeout (`update.rs:241-251`). Introduce a
`BudgetForwardingExecutor` — same wrapper shape as `ForwardingInteractiveExecutor` — installed on the non-interactive update
runtime, filling `spec.timeout` from the wire payload budget (`payload.timeout`, serde-defaulted to `DEFAULT_UPDATE_TIMEOUT`
7200 s at `crates/shared/wire/src/payloads.rs:547-550,641-644`) when the plugin left it `None`. **Lands in the same commit as
M1.1** — the intermediate state (default without budget plumbing) deploys strictly worse behaviour and is forbidden
(commit-ordering rule; see common-mistakes ledger on dark-first sequencing).

Also in this task, two latent bugs in `ForwardingInteractiveExecutor` become live with M1.1 and are fixed together:

- The interactive→non-interactive fallback passes the _original_ spec, not the promoted one
  (`update.rs:1187`) — after M1.1 that silently downgrades a 7200 s budget to the 600 s default exactly when the PTY path
  already failed. Use the promoted spec in the fallback branch.
- `execute_quiet` (`update.rs:1195-1200`) passes through without promotion — same treatment.

### M1.3 Operation-level deadlines on the agent

Wrap each background operation in `tokio::time::timeout` at the `run_*` entry points (`run_check_versions`
`client.rs:319-335`, discovery, test-plugin-config):

- `VERSION_CHECK_OP_TIMEOUT` (proposed: 1800 s) and `DISCOVERY_OP_TIMEOUT` (proposed: 1800 s) — generous; the per-command
  default (M1.1) is the sharp edge, the op deadline is the belt.
- Test-plugin-config: align with the controller's existing 30 s proxy deadline (the `Duration::from_secs(30)` is passed at
  `crates/ui/web-api/src/routes/plugin_configs/test_action.rs:276` into the generic wait in `config_test_proxy.rs`) —
  agent-side 25 s so the agent's answer, not the proxy 504, is the common failure.
- On op timeout: per-item error results for every item not yet completed; the batch continues where item-level isolation
  already exists; **results are always sent** (plugin rule: failures set `error`, never silently degrade).
- `run_with_retry` (`version_check.rs:524-552`) gains an overall deadline so backoff cannot extend past the op budget.
- An op deadline firing mid-command cancels the command future from outside; this is safe only because M1.1's abort-on-drop
  reader guard makes external cancellation clean. The Testing strategy includes a case for exactly this interaction.

### M1.4 Per-hook timeout

Implement the (currently fictional) per-hook contract: each pre/post update hook plugin execution is bounded by
`HOOK_TIMEOUT` (proposed: 300 s, matching the number the docs already promised). Pre-hook timeout aborts the update (first
failure aborts — existing rule); post-hook timeout logs a warning and continues (post-hooks are non-fatal — existing rule).
Reinstate the wire-protocol.md table row removed in M0.5, now true. Requires M0.3 (hooks must flow through the executor for
the bound to reach SSH-managed hosts).

### M1.5 Long-operation overrides

- Generic shell plugin (`crates/plugins/generic/shell/`): add an optional validated `timeout_seconds` config field (bounds:
  1 s..=86400 s) — user-authored commands are legitimately slow. Schema-driven validation alongside the existing
  `validate_command_length()` treatment.
- GitHub release download+install and any plugin operation with a measured legitimate duration beyond the default sets an
  explicit `with_timeout` at spec-construction time. Plan-time inventory: grep plugin command constructions and classify each
  against the 600 s default; the apt/dnf/pacman/... index refreshes and version detections stay on the default.

### M1.6 Overlap prevention is agent-side only — no controller registry (2026-08-22 amendment)

Dispatch today is fire-and-forget with no correlation (`version_check_dispatch.rs:298-307`,
`routes/service_ws/handler/discovery.rs:146-160`, `executors/detect_version.rs:111-115`). The original design here added a
controller-side pending-request registry (in-memory, `(host_id, op_kind)`-keyed, with a watchdog synthesizing timed-out
results). **Removed by the 2026-08-22 amendment** after plan review; this section now records the decision and its reasons
so it is not re-invented:

- **The watchdog's premise was false.** It relied on "the existing failure surface for that op" — but the result handlers
  deliberately _drop_ errored results (`handle_version_check_results` skips any result with `error.is_some()` before DB
  writes, SSE, and MQTT; only a `debug!` log and an audit error count remain). A synthesized timed-out result was a provable
  no-op. No failure channel for async agent ops exists today; building one is descoped to the follow-up epic
  `uptrakit-async-op-failure-surface` (see Goal 4).
- **Nothing consumes controller-side pending state.** The web UI's "checking"/"discovering" indicators are scoped to the
  triggering HTTP request (`finally`-cleared), `VersionCheckCompleted`/`DiscoveryCompleted` SSE only trigger list refreshes,
  a 300 s poll covers missed refreshes, and no DB column records in-flight state. There is no stuck UI state to clear and
  no reader for "already running".
- **Mirror-state defects are structural, not fixable.** The overlap harm — concurrent duplicate runs — occurs on the agent
  host. A controller-side mirror can only approximate agent truth: entries die on disconnect while the agent keeps running,
  the dedup key conflates scheduler batches with manual single-item dispatches, and result-to-entry matching by item id
  can free the wrong host's slot on the multi-host SSH runtime. Enforcement belongs where the harm is.
- **Config test never belonged in this machinery.** `TestPluginConfig` is request/response, correlated by `ConfigTestProxy`
  (`crates/ui/web-api/src/config_test_proxy.rs`) with a bounded REST wait (30 s proxy deadline) and an agent-side in-op
  deadline (M1.3, 25 s). Gating it would turn a skip into a silent REST 504. It stays entirely outside the guard.

Consequences: the controller dispatches freely; the agent-side guard (M1.7) is the sole overlap-prevention mechanism. A
scheduler tick re-dispatching to a host with an op in flight costs one WS message that the agent skips with an `info!` log —
accepted. The "budget symmetry" rule between a controller TTL and the agent deadline is void (there is no controller TTL);
the op-timeout constants stay in `uptrakit-shared-types` as the single home for the per-op budgets the agent enforces.

### M1.7 Agent-side spawn guard (sole overlap-prevention mechanism; 2026-08-22 amendment)

Replace bare `spawn_background` (`client.rs:285-294`) usage for the scheduled, idempotent ops — CheckVersions /
DiscoverSoftware (**not** TestPluginConfig; request/response, excluded per M1.6) — with a small registry keyed
`(host_machine_id, op_kind)` (the WS agent serves one host; the SSH runtime
(`spawn_check_versions_ssh`, `agent-ssh-runtime/src/client.rs:820-890`) serves many):

- **Dedup is item-set-aware for version checks.** The entry records the dispatched software-item id set. A new
  `CheckVersions` is skipped (`info!` log, re-issued next scheduler tick) only when its requested item set is a subset of
  the running set — the running op will deliver those items' results. A non-subset request (e.g. a manual check for an
  item outside a running scheduled batch) runs concurrently: the harm being prevented is duplicate _same-item_ work, and
  disjoint item sets were always able to overlap. Discovery has no item set; its dedup degenerates to the plain
  `(host, kind)` key.
- **Dedup window vs cancellation are split.** The skip window ends at the op _budget_ (M1.3 constant); the guarded future
  is hard-cancelled — dropped via `tokio::time::timeout_at` — at _budget + grace_ (a registry entry must never outlive its
  own deadline; the blocking-await failure class in M0.2 would otherwise wedge the slot permanently). Between budget and
  budget + grace a new dispatch may start while the stale future awaits cancellation — accepted; the stale future's result,
  if any, is idempotent. No retained `JoinHandle`/`abort()`: `timeout_at` on the un-spawned future gives identical
  drop-at-deadline cancellation without a second task, and panic isolation is moot under the workspace's `panic = "abort"`.

Per-plugin-type keying was considered and rejected: the op-level deadline (M1.3) already prevents one
wedged plugin from blocking the host's next round, and per-type keys multiply registry states for no additional safety.
Registry locking: `parking_lot::Mutex`, guard dropped before any `.await` (project convention).

`ExecuteBatchUpdate` is **excluded from the registry by design**: duplicate update dispatches are already structurally
prevented controller-side (`validate_update_preconditions` HTTP 409 + the `uix_update_history_host_active` partial unique
index) and agent-side (the single-update gate; the SSH runtime's one-update-per-host rule), and the batch task self-bounds
via its existing `tokio::time::timeout(payload.timeout, …)` wrap (`client.rs:534`). A skip-or-busy-reply policy here would
depend on the deferred batch gate-rejection follow-up; the existing guards make that dependency unnecessary.

### M1.8 Reaper keyed on the row's budget

The reaper loop (`crates/ui/web-api/src/update_reaper.rs:35,48` — the scheduling wrapper, not the same-basename query module
in `web-api-queries`) uses `DEFAULT_UPDATE_TIMEOUT + REAPER_GRACE` as a constant cutoff, contradicting its own doc ("keys
purely on the update's own budget"). Harmless today (the controller always sends the default) — but M1.2 makes per-item
budgets real. Persist the dispatched budget on the `update_history` row (new nullable integer column, seconds; migration +
entity change in one commit per the migration/entity-swap rule) and reap each row at `its budget + REAPER_GRACE`, falling
back to the constant for NULL legacy rows.

### M1.9 SSH deadline owned inside the stream loop

Move the deadline into `exec_command_streaming` (`ssh_transport.rs:319-372`): a `select!` arm on `sleep_until` alongside the
`channel.wait()` loop, so on expiry the code can `channel.close().await`, return `TimedOut`, and leave the loop cleanly.
This **retires** `PosixSshCommandExecutor::run_remote`'s existing outer `tokio::time::timeout` wrap
(`ssh_executor.rs:119-138`) — the duration threads through `SshSession::exec_command` into the loop instead; leaving the
outer wrap in place would recreate the cancellation anti-pattern (§ Design principles) as a double timeout. Also
handle `ChannelMsg::Eof` (currently ignored — a server sending `ExitStatus`+`Eof` without `Close` blocks the loop forever) and
capture `ChannelMsg::ExitSignal` (currently discarded) for M2.2's signal-aware error mapping. The remote process may keep
running after `close` — documented, and addressed by M2.3/M2.4.

### M1.10 `RemoteExecutor::exec_command` timeout parameter

`crates/shared/command/src/remote_executor.rs` gains a timeout on its contract (parameter or spec-carried — plan decides
against the trait's call-site inventory). Implementors bound their awaits by **passing the deadline through to the layer that
owns it** — for the SSH implementor that means threading the timeout into `SshSession::exec_command` →
`exec_command_streaming`, reusing M1.9's internal `select!` deadline. Wrapping the call in an outer `tokio::time::timeout` is
forbidden here for the same reason stated in § Design principles (cancellation cannot close the channel or express cleanup).

### M1.11 SSE stream stall detection + missed-pong

- The three openapi-client SSE streams set `.timeout(86400 s)` with no per-read bound
  (`events_stream.rs:106`, `batch_progress_stream.rs:83`, `update_output_stream.rs:55`) — a silently stalled stream never
  errors for 24 h. `reqwest::ClientBuilder::read_timeout` (reqwest 0.13) is a **client-level** setting, and `UptrakitClient`
  holds one shared `reqwest::Client` for streams and ordinary calls alike (`openapi-client/src/lib.rs:132-137`) — so the fix
  is a second, dedicated streaming `Client` (same connect settings, `read_timeout` sized to the server's keepalive cadence)
  used only by the three stream constructors; setting `read_timeout` on the shared client would bound every ordinary request
  too. Other reqwest clients already carry total timeouts and need nothing.
- service-sdk client: pongs are logged but not tracked (`event_loop.rs:423-429`). Count outstanding pings; after
  `MISSED_PONG_LIMIT` (proposed: 3) consecutive misses, treat the connection as dead and enter the existing reconnect path
  (`reconnect_backoff_builder` machinery). This closes the "half-open TCP, agent believes it is connected" gap.

## Milestone 2 — kill mechanics + root backstop (canary rollout)

Everything that signals processes or touches sudoers lives here. Rollout rule: deploy to one canary host, observe one full
scheduler cycle (version check + discovery + one manual update), then fleet.

### M2.1 Process-group kill on the non-interactive path

- Spawn with a `pre_exec` `setsid()` (the interactive path's `pty_pre_exec` at `interactive.rs:112-123` is the working
  precedent; the non-interactive variant is setsid-only). `bash -c` pipeline members all land in the new group, so the group
  kill reaps whole pipelines — pinned by an explicit test.
- Upgrade the expiry arm of `run_command_exec_impl`'s internal deadline (structure landed in M1.1): on expiry an RAII
  guard sends SIGTERM to the group, waits `KILL_GRACE` (proposed: 10 s), sends SIGKILL to the group, then bounds the reap
  (`KILL_REAP_GRACE`, proposed: 5 s) and aborts both reader tasks. `killpg` happens **before** the `Child` drops (mirroring
  the ordering rationale documented at `update.rs:1120-1126`); tokio's subsequent `kill_on_drop` `kill(pid)` is harmless
  (ESRCH). EPERM from `killpg` (root-owned members) → structured `warn!` + `TimedOut` with the unkillable marker; the sudoers
  backstop (M2.4) owns that process's fate.
- Signal delivery via the already-vendored `rustix` — no new external dependency, **but a manifest change**: `rustix` is
  currently `optional = true` in `uptrakit-command`, gated behind the `interactive` feature (`Cargo.toml:12,21`), and most
  non-interactive consumers build without it. Make `rustix` (with the `process` feature) a non-optional dependency of
  `uptrakit-command` (workspace-referenced per the dependency policy; additive — no `cfg(not(feature))` anywhere), so the
  non-interactive kill path exists in every build. Companion edit in the same commit: the `[features]` table's
  `interactive = ["rustix"]` must become `interactive = []` — a bare dependency name in a feature value may only reference an
  _optional_ dependency, so the old line is a hard cargo manifest error once `optional = true` drops (reproduced on cargo
  1.97.0); the feature keeps gating the PTY-only code via `#[cfg(feature = "interactive")]` unchanged.

### M2.2 Signal-aware error taxonomy

- Local: use `ExitStatusExt::signal()` instead of collapsing to `-1` (`command.rs:187`, `interactive.rs:335`); add
  `CommandError::KilledBySignal(i32)` and an `unkillable: bool` dimension on `TimedOut` (exact shape — field vs sibling
  variant — is a plan decision; the requirement is that "deadline fired", "killed by signal", and "kill failed, process
  abandoned" are three distinguishable outcomes in results and logs). `CommandError` is a plain `pub enum` today; the change
  that adds variants also applies the project convention (`#[non_exhaustive]` by default) — the plan enumerates the match
  sites this affects (see ledger rule below).
- SSH: map `ChannelMsg::ExitSignal` (captured in M1.9) to the same taxonomy.
- Ledger rule applied at plan time: enumerate every `match` over `CommandError` and every sibling error enum that gains the
  new failure mode; variant + `#[error]` text + mapping arm + mapping test land in every sibling in one unit; classify each
  match site as exhaustive (compile-fails, fine) or wildcard (silent misroute — needs an explicit arm).

### M2.3 Remote `timeout(1)` wrapper for unprivileged SSH commands

- Applied **inside** `build_remote_command_string` (`executor.rs:111-129`), never by string-prefixing — the emitted shapes are
  `[cd '<dir>' &&] <envs> <program> <args>`, and the wrapper slots between envs and program:
  `[cd '<dir>' &&] <envs> timeout -k <KILL_GRACE> <secs> <program> <args>` (env assignments before `timeout` are inherited
  through it). Golden tests for all three shapes (plain, env, cwd).
- Applied only when the spec is **not** privileged (see § Non-goals for why wrapping `sudo` is useless) and only when the host
  has a compatible `timeout`.
- Capability probe: behavioural, not existence — run `timeout -k 1 1 true` once per pooled session and cache the verdict on
  the pool entry (dies with the session; `IDLE_TTL` 300 s gives natural expiry). BusyBox pre-1.30 (`-t` flag syntax), toybox,
  and macOS (no `timeout`) all fail the probe and get no wrapper — the local deadline (M1.9) still bounds the await. The
  probe also resolves the absolute path (`command -v timeout`) and the wrapper uses the pinned path, mirroring the sudoers
  convention of resolving bare names at bootstrap rather than trusting the remote PATH at each invocation.
- Exit-code mapping: 124 (and 137 when `-k` escalated) → `TimedOut`, **only when the wrapper was applied** (tracked on the
  execution, not inferred globally) — a plugin command genuinely exiting 124 must not be misreported.

### M2.4 Sudoers per-command `TIMEOUT=` backstop

- `SudoCommandEntry` (`crates/plugins/infrastructure/core/src/traits.rs:164` — `#[non_exhaustive]` + builder, so additive)
  gains `command_timeout: Option<Duration>` + `with_command_timeout()`. The sudoers generator renders it as a `TIMEOUT=<n>`
  option on that entry's `Cmnd_Spec` line.
- Classification policy: **read-only/refresh commands get a tight timeout** — derived, not independent: `TIMEOUT=` value =
  that command's report budget + `KILL_GRACE` + slack (proposed slack: 120 s; with the 600 s default that yields ~730 s,
  rounded to 900 s), referenced from the same constant the executor uses — and `TIMEOUT=` may only be emitted for entries
  whose commands never receive a per-call `with_timeout` or per-config budget override (the generator cannot see those, so
  an overridden command under a derived backstop would be killed early with a confusing error) — so the backstop cannot drift into a long
  UI-says-failed-but-lock-still-held window (documented residual: the window is `KILL_GRACE + slack`, and a re-dispatched
  check during it may fail on lock contention — logged, not specially typed); **mutating update/install commands get none** (killing a package
  install mid-flight risks a wedged dpkg/rpm state worse than the hang; their cleanup story is the agent-side budget +
  M2.1/M2.2 reporting); **interactive-session commands get none** (a 2-hour human-driven dist-upgrade is legitimate).
  Each package-manager plugin's entries are classified in the plan with the plugin author rule documented in
  plugin-guidelines.md.
- **Version gate:** `TIMEOUT=` requires sudo ≥ 1.8.20 (verified against sudoers(5); re-verify at plan time on a target host).
  Parse `sudo -V` line 1 during bootstrap/sync (a new probe alongside the existing `sudo -n -l` checks,
  `operations/sudoers.rs:30`, `host_feature.rs:75`); emit `TIMEOUT=` only when supported. M0.1's validate-before-write is the
  second line of defence: an unsupported directive fails `visudo -cf` on the temp file and aborts the sync harmlessly.
- **Coverage honesty:** the generator runs only for SSH-bootstrapped/synced hosts (`operations/bootstrap.rs`,
  `operations/sync.rs` call sites). Hand-installed agents and the PVE helper-script installer
  (`scripts/pvehs/install/uptrakit-install.sh:57-86` — a second, already-drifted copy of the sudoers format) do not receive
  it. Deliverables: (a) update the installer script to emit the same `TIMEOUT=` lines behind the same version check;
  (b) **agent-side sudoers self-verification as a first-class M2 item**: at startup (and on sudoers-relevant config change)
  the agent parses `sudo -n -l`, compares against its plugins' `required_sudo_commands()` incl. `TIMEOUT=` presence, and
  records the delta as a host feature via the existing probe mechanism — hand-installed hosts then visibly report
  backstop-absent instead of silently lacking it (structured log + host facts; no new UI work in this spec; an agent-driven
  local sudoers resync is explicitly out of scope — writing sudoers requires root, which the agent does not have).
- **Grandchild caveat (recorded, verified at plan time):** without a terminal, sudo's `use_pty` is inert and the timeout kill
  may target the command process rather than its group — sudo may not reap grandchildren (e.g. a forked `dpkg`). The backstop
  is scoped to index-refresh commands precisely because their process trees are shallow and lock-free. Plan-time verification
  on the canary host: wedge an `apt-get update` (unroutable mirror), confirm the `TIMEOUT=` kill reaps the tree; if
  grandchildren survive, document the residual and keep the tight scope.

### M2.5 SSH interactive kill path

`InteractiveSessionGuard`'s `kill_process_group` is a documented no-op sentinel over SSH (`child_pid: 0`,
`ssh_transport.rs:434-441`); mapped PTY control characters cover SIGINT/SIGQUIT/SIGTSTP only. On interactive deadline: send
`\x03` (SIGINT to the foreground group), then close the channel — with a PTY, sshd delivers SIGHUP to the session on close,
which is the reliable path. If verification shows a gap, `russh`'s `channel.signal()` request is the fallback (server support
varies; OpenSSH ≥ 7.9 — behavioural check at plan time).

### M2.6 Sudo invocation hardening

Non-interactive privileged specs invoke `sudo` with `-n` (fail fast instead of prompting into a null stdin) and `--` before
the program name (a program starting with `-` must not parse as a sudo option) — `sudo.rs:180-186`. Interactive specs keep
prompting semantics. Sudoers matching is unaffected (`-n`/`--` are sudo options, not command tokens); pinned by the existing
sudo-layer tests plus one new case per flag.

## New named constants (proposed values; single canonical home per constant)

| Constant                   | Value                                                | Home                                                                                                                 |
| -------------------------- | ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `DEFAULT_COMMAND_TIMEOUT`  | 600 s                                                | `uptrakit-command`                                                                                                   |
| `KILL_GRACE`               | 10 s                                                 | `uptrakit-command`                                                                                                   |
| `KILL_REAP_GRACE`          | 5 s                                                  | `uptrakit-command`                                                                                                   |
| `VERSION_CHECK_OP_TIMEOUT` | 1800 s                                               | `uptrakit-shared-types` (agent-side in-op deadline + guard dedup window)                                             |
| `DISCOVERY_OP_TIMEOUT`     | 1800 s                                               | `uptrakit-shared-types` (same)                                                                                       |
| `HOOK_TIMEOUT`             | 300 s                                                | `uptrakit-agent-core`                                                                                                |
| `CONFIG_TEST_OP_TIMEOUT`   | 25 s                                                 | `uptrakit-shared-types`, beside the proxy's 30 s deadline with the recorded invariant: agent budget < proxy deadline |
| `PENDING_DISPATCH_GRACE`   | 600 s                                                | `uptrakit-web-api` (reaper module)                                                                                   |
| `BG_OP_ABORT_GRACE`        | 120 s                                                | `uptrakit-agent-core` (guard cancellation slack past the op budget; M1.7)                                            |
| `MISSED_PONG_LIMIT`        | 3                                                    | `uptrakit-service-sdk`                                                                                               |
| Sudoers refresh `TIMEOUT=` | budget + `KILL_GRACE` + 120 s slack (≈900 s rounded) | sudoers generator, derived from the executor's constant                                                              |

Values are engineering estimates; plans may tune them but every value crossing a test fixture must be derived from the
constant by name (`CONST + 1`), never a bare literal.

## Error handling

Per-boundary typed enums throughout (`thiserror` + `rootcause`); no `unwrap`/`panic!`. New variants: `CommandError` gains the
signal/unkillable taxonomy (M2.2); `PluginError::TimedOut` mapping already exists
(`crates/plugins/infrastructure/core/src/command.rs:33`) and is reused. Every sibling query/handler enum that newly observes a
timeout failure gains its variant + mapping arm + test in the same change (see M2.2 ledger rule). No raw status-code literals;
no stringly matching on error text.

## Testing strategy

- Timer-driven logic (op deadlines, agent-guard dedup/cancellation, missed-pong): `#[tokio::test(start_paused = true)]` +
  `tokio::time::advance()` — **except** anything touching SQLx/SeaORM pools (reaper queries), which use real short delays per
  the documented testing rule.
- Kill-path tests spawn real child processes (`sleep`-style) and must use short real timeouts — these are a documented
  exception to the paused-clock rule and the plan must add them to the exception list in `docs/development/testing.md`.
  Group-kill coverage includes the `bash -c` pipeline shape (§ M2.1).
- Concurrency/liveness guards get red-checkable tests: the M1.2 regression test must fail when the budget promotion is
  removed; the M1.7 dedup test must fail when the guard check is deleted (delete-the-guard red check as acceptance).
- External-cancellation cleanliness (M1.1/M1.3 interaction): a test cancels the command future mid-execution and asserts the
  reader tasks are aborted (not detached) — red-checked by removing the abort-on-drop guard.
- Golden tests for all three `build_remote_command_string` shapes with and without the wrapper (M2.3).
- Reaper changes: two-phase seeds with distinct unique-key values; budget-column reap covered for per-row, NULL-fallback, and
  boundary cases derived from the constants by name.
- New web-api handler tests (none planned — no controller-side machinery after the 2026-08-22 amendment) would use the
  shared `TestApp` harness; `db_access_policy.toml` rows are added only for fns under `routes/` per the gate's actual scope.

## Wire/API/schema impact

- **No wire-protocol payload changes planned.** Op budgets are agent-local constants; `CheckVersions` et al. are untouched.
  If a plan discovers a payload change is needed after all, it carries `WireValidate` + `./scripts/regen-asyncapi.sh` +
  committed `asyncapi.yaml` in the same change.
- **REST:** no new endpoints planned. M1.8 adds a DB column but no API field is required for the milestone's goal; if any
  response type changes (e.g. exposing the budget or an `Interrupted` reason string already surfaced), run
  `./scripts/regen-api.sh` and commit `openapi.json` + generated client in the same change — the step is run even when the
  expected diff is empty.
- **DB:** one migration (M1.8 budget column, nullable integer seconds) named per the migration convention, entity swap in the
  same commit; before appending, re-target any tip-relative `Migrator::down` tests per the known migration-test hazard.

## Documentation deliverables (enumerated; each clause is a deliverable)

| File                                              | Milestone              | Change                                                                                                                                                                                                                                                                                                      |
| ------------------------------------------------- | ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/development/command-executor.md`            | M0.5, M1.1, M2.1, M2.2 | fix stale `CommandSpec` snippet + builder table; document the default-timeout contract; document group-kill escalation + unkillable semantics; document that defaults are never written into specs                                                                                                          |
| `crates/shared/command/src/types.rs` doc comments | M0.5                   | fix the `sudo env` claim at `:170`                                                                                                                                                                                                                                                                          |
| `docs/api/wire-protocol.md`                       | M0.5, M1.3, M1.4       | remove fictional per-hook row; document op deadlines + always-send-results; reinstate per-hook row as true                                                                                                                                                                                                  |
| `docs/security/sudoers-management.md`             | M0.1, M2.4, M2.6       | validate-before-write flow; `TIMEOUT=` policy incl. version gate + classification rule + coverage/degraded-state; `-n`/`--` invocation hardening                                                                                                                                                            |
| `docs/architecture/update-history-entity.md`      | M0.4, M1.8             | `Pending` reap semantics + new reason; per-row budget keying                                                                                                                                                                                                                                                |
| new ADR (via `adrs new`, never hand-numbered)     | M1/M2 boundary         | "Layered command deadlines and kill policy" — records the three-layer model, the never-write-defaults-into-spec rule, the sudoers backstop scope, and the extension of ADR-0024's reaper to `Pending` + per-row budgets; every factual claim about adjacent subsystems verified against source at authoring |
| `docs/development/update-hooks.md`                | M1.4                   | per-hook timeout semantics (pre aborts, post warns)                                                                                                                                                                                                                                                         |
| `docs/development/plugin-guidelines.md`           | M1.5, M2.4             | plugin-author guidance: when to `with_timeout`, shell plugin `timeout_seconds`, `SudoCommandEntry::with_command_timeout` classification rule                                                                                                                                                                |
| `docs/development/testing.md`                     | M2.1                   | documented exception: real-process kill tests use real short timeouts                                                                                                                                                                                                                                       |
| `docs/api/services-operations.md`                 | M1.11                  | missed-pong dead-connection detection alongside ping-interval mechanics                                                                                                                                                                                                                                     |
| `docs/development/autodiscovery-internals.md`     | M1.7                   | discovery dedup behaviour (agent-side guard; file already documents discovery cadence)                                                                                                                                                                                                                      |
| `AGENTS.md`                                       | M1                     | one new MUST-FOLLOW rule line: every command execution path carries a deadline (executor default + explicit budgets); link to command-executor.md as canonical home                                                                                                                                         |
| `CONTEXT.md`                                      | M1                     | glossary entries: _Report deadline_, _Kill escalation_, _Root backstop_ (the three-layer vocabulary)                                                                                                                                                                                                        |
| `scripts/pvehs/install/uptrakit-install.sh`       | M2.4                   | emit `TIMEOUT=` lines behind the same sudo-version check (script, listed here because it embeds sudoers documentation-by-example)                                                                                                                                                                           |

Plan-time doc sweep rule: before finalizing each plan, grep the whole repo (including root-level markdown, hidden dirs) for
the concept words of anything that milestone deletes or changes ("kill_on_drop", "per-hook", "command_timeout", stale path
strings) and promote every hit to a deliverable.

## Plan-time verification obligations (claims that must be re-proven, not trusted)

1. sudoers `TIMEOUT=`/`command_timeout` minimum version (1.8.20) — check `man 5 sudoers` on a target host; behavioural test on
   canary.
2. sudo timeout kill scope (command vs process group; grandchild reaping) — wedge test on canary (§ M2.4).
3. GNU/BusyBox/toybox `timeout` flag compatibility — the behavioural probe (M2.3) is the mechanism; verify the probe command
   against BusyBox ≥/< 1.30 syntax before pinning it.
4. `russh` channel close/`signal()` semantics against the deployed OpenSSH versions (M1.9, M2.5).
5. External-scheduler dispatch funnel (M1.6) — obsolete since the 2026-08-22 amendment (no controller-side registry to
   place). The funnel was verified anyway during plan review: all dispatch paths, including the external scheduler's
   NATS→WS bridge, reach `ServiceConnectionRegistry::send`.
6. `update_history` writer inventory (M0.4, M1.8) — grep-derived, not task-list-derived.
7. Baseline runs of every scoped gate command against the untouched tree (feature flags per crate checked against each
   crate's own `Cargo.toml`), per the standing gate rules.

## Out of scope (deferred, tracked)

- **Proxmox guest-exec adapter** (`crates/plugins/infrastructure/proxmox/src/agent/guest_exec_adapter.rs:81-98`) ignores
  `spec.timeout` entirely, and QGA exec is fire-and-forget on the guest side — killing the poller strands the guest process.
  Needs its own design (guest-side reaping story).
- **Direct `SshSession::exec_command` call sites** (~45, in `host_info.rs`, `operations/bootstrap*.rs`, `operations/sync.rs`)
  bypass the executor and take no timeout — bootstrap can hang forever. Deferred: wide blast radius, own sweep.
- **`open_stdio_tunnel` / docker-over-SSH** (`sudo.rs:226-228`, `docker_proxy.rs`) — unbounded; own design.
- **SSH session teardown/reconnect on timeout** — rejected (§ Non-goals).
- **`ExecuteBatchUpdate` gate-rejection batch half** — owned by the gate-rejection spec's declared follow-up.
- **Reconnect reconciliation for reaped `Pending` rows** — the delivered-then-disconnected residual documented in M0.4
  (agent reports in-flight ops on reconnect; controller refuses to free a reaped row's slot while the agent claims it).
- **Global settings-store timeout configuration / per-plugin timeout config fields** (beyond the shell plugin) — rejected for
  now; revisit only if constants prove wrong in practice.
- **UI surfacing of degraded-backstop state beyond host facts + logs** — follow-up once the host-feature data exists.
- **hook.shell executor routing** — owned by the existing `Shell Hook Executor Routing` spec/plan (M0.3 is an ordering
  dependency, not a work item of this spec).

## Milestone → plan mapping

- **Plan 0** (M0.1–M0.5): independent live-bug fixes; land before anything else. M0.3 is "merge the existing plan", not new work.
- **Plan 1** (M1.1–M1.5): command/op deadlines + budget plumbing (agent side). M1.1+M1.2 in one commit.
- **Plan 2** (M1.6–M1.11): agent-side op guard, reaper budget, SSH stream deadline, SSE/pong. Can parallel Plan 1 except
  M1.8 (budget column) depends on M1.2's budget concept only nominally — plans state the ordering explicitly.
- **Plan 3** (M2.1–M2.6): kill mechanics + sudoers backstop; canary rollout gate before fleet.

Dependencies: Plan 0 → (Plan 1 ∥ Plan 2) → Plan 3.

## Amendment log

### 2026-08-22 — M1.6/M1.7 restructured after plan review (owner-approved via grilling)

Plan review of the M1.6–M1.11 plan found the controller-side registry + watchdog unsound at the root:

1. **Fatal:** watchdog result synthesis was a provable no-op — the result handlers drop errored results before any DB
   write, SSE, or MQTT emission, so the "existing failure surface" the design reused does not exist.
2. Nothing consumes controller-side pending state (UI in-flight indicators are HTTP-request-scoped; SSE events only
   trigger refreshes; no DB in-flight column).
3. The mirror's defects were structural: disconnect loses entries while the agent keeps running, `(host, kind)` keying
   conflates scheduler batches with manual single-item dispatches, and item-id-based clearing can free the wrong host's
   slot on the multi-host SSH runtime.
4. Config test is request/response (bounded by `ConfigTestProxy` + the M1.3 in-op deadline) — gating it converted a skip
   into a silent REST 504.

Resolution: overlap prevention moved entirely to the agent-side guard (M1.7), which gained item-set-aware dedup and a
split dedup-window/cancellation deadline; config test removed from the guard; Goal 4 re-scoped honestly; per-item
async-op failure surfacing extracted to follow-up epic `uptrakit-async-op-failure-surface` (discovered-from the
M1.6–M1.11 plan epic; independent enhancement, no blocking edge).
