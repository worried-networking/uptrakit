# 0046 — Agent-side guard for fire-and-forget operation overlap

Date: 2026-08-25

## Status

Accepted

## Context

The controller dispatches version checks and discovery runs to agents as fire-and-forget
operations. Nothing stops the controller or the external scheduler from re-dispatching the same
operation to the same host while a prior run is still executing — a reconnect, a manual retrigger,
or an overlapping periodic tick can all cause this.

A redispatch is harmless at the message layer: the controller sends one more WebSocket message. The
harm happens on the agent. Two concurrent version-check or discovery runs for the same host contend
for the same plugin locks, double the outbound network and command load, and can produce two
results racing to update the same rows. The problem is local to the agent process, so the fix must
live there too.

## Decision

The agent's `BackgroundOps` registry (`uptrakit-agent-core`, `crates/shared/agent-core/src/client.rs`)
is the sole overlap-prevention mechanism for `CheckVersions` and `DiscoverSoftware`. The controller
keeps no pending-op state and performs no dedup of its own.

### Guard key and scope

Entries are keyed by `(host_machine_id, kind)`. `kind` is `CheckVersions` or `Discovery`. A
version-check entry additionally carries the requested item-set (`Vec<Uuid>` of software item IDs).
A new dispatch is skipped only when a live entry already covers it:

- **Version checks are item-set-aware.** A new request is skipped only when its item set is a
  subset of a live run's item set. Disjoint or partially overlapping sets run concurrently — this is
  accepted overlap, not a gap in the guard.
- **Discovery is whole-host.** Any live discovery run for the host suppresses a new dispatch; there
  is no sub-host scope to compare.

### Dedup window and cancellation backstop

Each entry records a dedup window ending at spawn time plus `budget + OP_DEADLINE_GRACE` — the
in-operation deadline instant defined in `uptrakit_shared_types::op_timeouts`. The window cannot end
before that instant: an operation still inside its own deadline is still legitimately running, and a
shorter window would let the guard suppress or double-dispatch around a run that has not finished.

Past the window end the entry stops suppressing new dispatches, but the spawned task itself is not
yet reclaimed. The guard wraps the future in `tokio::time::timeout_at`, set to the window end plus a
further `BG_OP_ABORT_GRACE` (120s). If the future has not resolved by then, it is dropped — the
cancellation backstop. A well-behaved run always produces a result well before this point, because
the in-operation deadline already guarantees a result (partial or timed-out) at the window end. The
backstop exists only to reap a plugin that hangs past its own deadline, a bug the guard cannot
prevent but must not let leak resources forever.

### Config tests are excluded

Plugin config tests do not use this guard. They are request/response, correlated by their own
request ID, bounded by a 25s in-op deadline (`CONFIG_TEST_OP_TIMEOUT`) plus a 30s REST-level bound.
Their overlap risk and failure mode are different: a caller is waiting synchronously for one
response, so there is no unbounded background task to guard against.

## Rejected alternative: controller-side pending-op registry

A controller-side registry mirroring agent-side in-flight state — tracking one pending-op entry per
`(host, kind)` and rejecting or queuing a redundant dispatch before it reaches the wire — was
considered and rejected.

Mirroring agent state at the controller is structurally unreliable, not merely more code:

- A service disconnect during a run orphans the controller's entry. The controller has no signal
  that tells it the agent-side run ended, so the entry can only expire on its own timer, duplicating
  the exact deadline logic the agent already has to run.
- Controller-side keying by `(host, kind)` alone conflates distinct item sets. Recovering the
  agent's subset rule at the controller means either shipping the item-set logic to both sides or
  coarsening version-check dedup to whole-host, which would incorrectly block legitimate disjoint
  version checks.
- Nothing at the controller consumes the pending-op state once written. It exists only to be
  checked before the next dispatch, so it adds a stateful table with no independent read path — pure
  guard overhead with no other product value.

Rejecting the controller-side mirror removes a whole class of stuck-state defects: an orphaned entry
that never expires (or expires wrongly) either wedges future dispatches or silently stops guarding
anything. The agent already owns the ground truth of what is running, so the guard belongs where the
truth lives.

## Consequences

The controller dispatches freely. A redundant dispatch costs one skipped WebSocket message and one
agent-side log line — nothing more. This applies uniformly, including to a user-triggered manual
check: if its item set is covered by a live run, the agent skips it and the live run's results serve
both requests. Until the `uptrakit-async-op-failure-surface` epic lands, the skip is visible only in
the agent's log; the controller and the UI have no way to tell the user their manual trigger was
absorbed into an existing run.

Dedup is subset-only for version checks. A dispatch whose item set only partially overlaps a live
run's set is not suppressed — it runs concurrently, and the two results race on shared items with
last-writer-wins semantics. This is an accepted overlap, not a defect: full precision would require
splitting a partially-overlapping request into covered and uncovered halves, which the guard does
not attempt.

A hung plugin is cancelled without ever producing a result. The cancellation backstop drops the
future silently past `budget + OP_DEADLINE_GRACE + BG_OP_ABORT_GRACE`; the only trace is the agent's
warning log. Building an actual failure-surfacing channel back to the controller and UI is out of
scope here and belongs to the `uptrakit-async-op-failure-surface` epic.

The in-loop SSH command deadline stays a per-command bound inside the execution loop rather than an
outer wrap around the whole SSH session. An outer wrap would need to account for legitimate
long-running interactive work, which the in-loop deadline already does per command.

`exec_command`'s default timeout bound is scoped to non-interactive execution only. The interactive
PTY path (`SshSession::exec_command_interactive`) is a user-attended session, where a human is
present to abort or extend the session, and stays unbounded. This is an explicit, deferred
exception, not an oversight — closing it needs a distinct interactive-session deadline model.

Per-row update budgets recorded on `update_history.timeout_seconds`, and the ADR-0024 terminal-state
extension they build on, are out of scope for this decision. They are recorded in the
spec-mandated "Layered command deadlines and kill policy" ADR (Plan 4). See
[ADR-0024](0024-update-liveness-and-interrupted-status.md) for the terminal-state contract they
extend.
