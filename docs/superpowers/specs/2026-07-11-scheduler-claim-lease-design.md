# Scheduler Claim Lease Integrity — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — HIGH "Claim protocol permits duplicate task execution and
foreign-claim clobbering in HA".

## Problem

Three interacting defects in `crates/core/scheduler-runtime/src/claim.rs` break lease uniqueness for scheduled
tasks in multi-instance deployments (verified against current code):

1. `STALE_CLAIM_SECONDS = 600` while `TASK_EXECUTION_TIMEOUT = 2h` (`scheduler.rs`) — a task legitimately allowed
   to run two hours looks "stale" after ten minutes.
2. `locked_at` is written once in `try_claim` and never refreshed — there is no liveness signal while a task
   runs.
3. `release_claim` filters only on `Id` (`claim.rs:101`), not `LockedBy = controller_id` — a finishing instance
   unconditionally clears whoever currently holds the claim. (`release_all_claims` is already correctly scoped —
   the miss is only in the per-task release.)

Sequence in HA: any task running >10 min (e.g. `FetchReleases` against slow upstreams) gets its claim wiped by
another instance's `recover_stale_claims` (runs every 15s poll tick), is immediately re-claimed and executed
concurrently; when the original finishes, its unscoped `release_claim` clears the second instance's live claim,
opening a third concurrent claim. The comment on `STALE_CLAIM_SECONDS` (running tasks "check their own
cancellation token independently") explains the intent but does not make 10-minute recovery safe — the recovered
task is still executing.

Non-issues, checked: every claim operation is a single-statement atomic `update_many` (no read-then-write, so the
SQLite BEGIN IMMEDIATE rule is not implicated); the `scheduler-engine` crate named in AGENTS.md does not exist
(that is the separate docs-drift audit finding) — `claim.rs` in scheduler-runtime is the only claim logic, no
duplicate to reconcile.

## Approach

Lease semantics, minimally: **ownership-scoped release + heartbeat-backed staleness.** Both mechanisms reuse the
existing 15-second poll loop; no new background task, no new table, no config.

### 1. Ownership-scoped release

`release_claim` gains a `controller_id` parameter, the
`.filter(scheduled_task::Column::LockedBy.eq(controller_id))`, and a **return-type change** to
`error::Result<u64>` (rows affected, mirroring `recover_stale_claims`/`release_all_claims`) — "lost claim" is
`Ok(0)`, not an `Err`, and callers can't see it under the current `Result<()>`. Both `scheduler.rs` call sites
(release-on-completion and the shutdown-abort release, each inside the `join_set.spawn(async move …)` closure)
inspect the count and emit `tracing::warn!(task_id = %…, controller_id = %…, "claim already taken over")` with
structured fields on `0`. The closures currently capture only a cloned `db` — implementation adds
`let controller_id = self.config.controller_id;` to the capture list. Losing the claim skips nothing else by
design: the update carries run metadata (`next_run_at`, `run_count`, `last_error`), and if we lost the claim
that metadata now belongs to the takeover owner's run; writing it through a lost claim would clobber the live
owner's state. One statement, atomic, same shape as the already-correct `release_all_claims`.

### 2. Heartbeat via a dedicated lightweight task

Review killed the obvious placement: the poll loop **cannot** heartbeat, because `run()`'s tick arm awaits
`poll_cycle()` to completion and `poll_cycle` drains its `JoinSet` before returning — a single 2-hour task
blocks the next tick for its whole duration, so a poll-tick heartbeat would be structurally unreachable in
exactly the long-task scenario this fix targets (verified in `scheduler.rs`: `interval.tick() =>
poll_cycle(...).await` + `while let Some(_) = join_set.join_next().await`).

Instead: the scheduler spawns **one** dedicated heartbeat task at startup — an `interval` loop (~90s; 6+ beats
per stale window, margin chosen because heartbeat/poll/release all contend on the same serialized-writer SQLite
DB and a beat can be delayed under write contention; each write is a single small statement, so multi-beat
starvation is not expected — the wider margin is cheap insurance) executing one statement per beat, cancelled by
the same shutdown token the poll loop uses (shutdown then runs the existing `release_all_claims`).

**The heartbeat refreshes only claims with a live task, never all owned rows** (contrarian-driven — the naive
owner-wide refresh creates a *new, non-self-healing* leak: `release_claim` runs inside the spawned closure, a
panicking executor skips it, the `join_next` error arm only logs, and an owner-wide heartbeat would then keep
the dead task's claim fresh forever — strictly worse than today's 600s self-heal). Mechanism: an in-memory
live-task set (`Arc<parking_lot::Mutex<HashSet<Uuid>>>`) — inserted at `join_set.spawn`, removed by a **drop
guard** inside the spawned closure so removal happens on normal completion, timeout-cancel, and panic alike.
The beat executes
`update_many().col_expr(LockedAt, now).filter(LockedBy.eq(controller_id)).filter(Id.is_in(live_ids))` (empty
set → skip the query). A panicked/cancelled task drops out of the set, its `locked_at` goes stale, and
`recover_stale_claims` heals it exactly as today. One task, one query per beat, honors the
batch-over-per-item-loops rule. Two implementation notes: live-set removal and `release_claim` need **no**
ordering between them — staleness heal (600s) dominates the beat interval (~90s) by ~6×, so the last refresh
keeps the claim safe long past the release that follows microseconds later; do not engineer an ordering. And
the drop guard's `Drop` takes the live-set `parking_lot` lock for a plain `remove` only — no `.await`, no
nested locks (under this workspace's `panic = "abort"` a panicking task kills the whole instance anyway, and
the 600s heal on other instances is the recovery path).

Prerequisite audit (named because `tokio::time::timeout` only cancels futures at await points): verify no
registered `TaskExecutor::execute` does blocking IO/CPU work outside `spawn_blocking` — a blocking executor
escapes the 2h timeout, and with a live-set heartbeat its claim stays fresh while it blocks. If any executor
blocks, fixing it is a prerequisite bug in this spec's scope, not deferred. `STALE_CLAIM_SECONDS` stays 600, keeping the 10-minute crash recovery the
current comment promises — now actually safe, because staleness means "no heartbeat for 10 minutes" (instance
dead or partitioned), not "task ran longer than 10 minutes". Update the `STALE_CLAIM_SECONDS` doc comment to
describe the heartbeat contract.

Pre-existing gap, inherited knowingly: the `Scheduler` doc comment claims "a slow executor cannot block other
due tasks" — true within one cycle, false across cycles (a slow task blocks the *next* tick's claiming and
stale recovery). This spec does not restructure the loop (that is scheduling-behavior change, out of scope);
it corrects the doc comment to state the cross-cycle limitation.

**Rejected alternatives:** heartbeat inside the poll tick — structurally unreachable, above. Raising
`STALE_CLAIM_SECONDS` above 2h — trivial but trades the real defect for a >2-hour crash-recovery stall on every
crashed-mid-task instance (controller ids are per-boot on both the embedded path (`Uuid::now_v7()` at boot) and
the standalone scheduler (enrollment-assigned service id) — verified; a restarted instance cannot resume its
old claims). Per-task heartbeat tasks — N interval tasks and cancellation plumbing for a signal one global
statement provides. Restructuring `run()`/`poll_cycle` so ticking doesn't gate on task drain — larger
scheduling-semantics change; the dedicated task achieves liveness without touching claim/execution ordering.

### 3. Clock injection for testability

`recover_stale_claims` (and the new heartbeat fn) compute cutoffs from `OffsetDateTime::now_utc()` internally —
untestable without backdating rows, which testing.md forbids. Give the claim functions an explicit
`now: OffsetDateTime` parameter (callers in `scheduler.rs` pass `OffsetDateTime::now_utc()`; tests pass
fabricated times). **Stated deviation from the canonical pattern:** testing.md's documented clock injection is a
struct-held `Arc<dyn Fn() -> OffsetDateTime>` + `#[cfg(test)] with_clock` constructor, used by the stateful
OAuth/rate-limit stores. `claim.rs` is free functions with no struct to hang a clock on; the nearest analog
(`DeviceFlowService::poll(&self, …, now: OffsetDateTime)`) uses the same parameter shape, and every value is
computed and consumed within one statement — the stale-stored-clock hazard the canonical pattern guards against
cannot arise. Wrapping the module in a `ClaimStore` struct solely to hold a clock is machinery without a second
motivation; if the module ever grows state, revisit.

## Residual risk, stated

Heartbeat converts "long task ⇒ duplicate" into "10-minute DB partition while the task keeps running ⇒ bounded
duplicate window". If an instance can write again after partition, its next `release_claim` is now a scoped
no-op + warning (no clobber cascade — the third-claim scenario is structurally gone). Duplicate execution of an
idempotent-by-design task set (version checks, cleanups) during a partition is accepted; distributed-lock
machinery (fencing tokens, external lease service) is out of scope for a scheduler whose tasks are periodic and
idempotent.

Wedged-task recovery, restated honestly: today a task wedged inside a *live* instance loses its claim after
600s (accidental recovery — with the concurrent-duplicate cost this spec exists to remove). Under this design,
an await-yielding wedged task is bounded by the 2h `TASK_EXECUTION_TIMEOUT` (drop-cancel removes it from the
live set → 600s heal follows); a hypothetical blocking executor would wedge until instance restart — which is
why the blocking-executor audit above is in scope, not residual.

## Tests

Extend the existing in-file test module (in-memory SQLite harness present; all new fns take `now` so no
backdating and no tokio-time APIs — DB-backed tests, no `start_paused`, per the snapshot rules):

1. `release_claim` with a different `controller_id` → returns `Ok(0)`, claim intact, run metadata untouched
   (return-type change: existing `release_claim` tests extend their assertions, not just their parameters).
2. Heartbeat statement refreshes `locked_at` only for the calling controller's claims (two controllers, two
   tasks).
3. `recover_stale_claims(now)` does **not** release a claim whose `locked_at` was heartbeat-refreshed within the
   window, and does release one whose owner stopped heartbeating (drive both via the injected `now`).
4. Full-cycle regression: claim → heartbeat past the old 600s boundary → assert no recovery → scoped release
   returns `Ok(1)`.
5. Heartbeat-task lifecycle: it beats while a (stubbed, slow) task runs — the scenario the poll loop cannot
   cover — and stops on shutdown-token cancellation before `release_all_claims` runs.
6. Live-set drop guard: a task closure that panics (or is timeout-dropped) leaves the live set → next beat does
   not refresh its claim → `recover_stale_claims(now)` with advanced `now` reclaims it (the zombie-claim
   regression test).
7. Existing tests updated for new parameters/return types; behavior otherwise unchanged.

## Documentation deliverables

- Doc comments: `STALE_CLAIM_SECONDS` (heartbeat contract), `release_claim` (ownership scoping + lost-claim
  semantics + new return type), the new heartbeat fn, and the `Scheduler` struct comment (cross-cycle blocking
  limitation, see above).
- `docs/architecture/scheduler.md` (~claim-protocol section): update for the heartbeat contract **and fix the
  dead cross-reference found during review** — it cites `MqttLeaseCoordinator` as the mirrored claim pattern,
  which was removed with the `mqtt_leases` table (migration `m20260329_000001`); `docs/development/
  scheduler-engine.md` repeats the same dead reference (that file's nonexistent-crate framing is the separate
  audit finding — fix only the `MqttLeaseCoordinator` line if touching it, leave the rest to that finding).
- No new ADR: lease-protocol correctness fix inside one subsystem; no architectural surface change.

## Out of scope / deferred

- Fencing tokens / external distributed-lock service (periodic idempotent tasks don't warrant it).
- The nonexistent `scheduler-engine` crate docs-drift (separate audit finding).
- Task cancellation/timeout semantics (`TASK_EXECUTION_TIMEOUT` unchanged).
- Scheduler poll cadence and executor behavior (unchanged).
- External scheduler binary enrollment/credential flow (unchanged).
