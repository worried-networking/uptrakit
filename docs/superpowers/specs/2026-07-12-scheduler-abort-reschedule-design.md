# Scheduler Hard-Abort Reschedules Interrupted Task Immediately — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `crates/core/scheduler-runtime/src/scheduler.rs` (one line in the hard-abort branch + its comment, and
one added assertion in the existing `cancellation_releases_claim` test). No ADR, no deps, no wire, no external doc
change.

## Problem

Audit `audit-2026-07-11` L923 (MEDIUM · stability · effort S · core-mqtt-scheduler · verified):
`Scheduler::poll_cycle`'s per-task hard-abort branch (`scheduler.rs:264-290`) releases the claim of a task that
was **interrupted mid-run by shutdown** but computes `next_run_at = compute_next_run_at(now, interval, jitter)`
(`scheduler.rs:274`) — the **same** value the success path uses (`:315`). So an interrupted run is deferred by a
**full interval**. For `FetchReleases` (6 h) or `DetectVersion` (24 h) this silently skips an entire period.

The branch's own comment (`:270-272`) says it releases the claim "so other scheduler instances can pick up the
task immediately rather than waiting for the stale-claim recovery window (up to 10 minutes)". But writing a
full-interval `next_run_at` makes the released task **not due** for hours — strictly worse than the 10-minute
stale recovery the comment claims to beat, and directly contradicting the stated intent. Releasing the claim buys
nothing if the row is simultaneously marked "not due until +interval".

## Verified current reality (byte-checked, 2026-07-12)

- The abort branch (`scheduler.rs:262-290`) is a `biased` `tokio::select!` arm on `abort.cancelled()`. It computes
  `let now = …now_utc();` (`:273`), then
  `let next_run_at = interval::compute_next_run_at(now, task.interval_seconds, task.jitter_seconds);` (`:274`), then
  calls `claim::release_claim(&db, task.id, next_run_at, &Err("scheduler shutdown during execution".to_string()))`
  (`:275-281`) and `return`s.
- The **success path** (`:313-320`) computes the *same* `next_run_at` expression (`:315`) and calls the same
  `release_claim`. That advance is **correct there** — the run completed, so the next run is genuinely one interval
  away. The bug is that the abort path (run did **not** complete) reuses that advance.
- `interval::compute_next_run_at(now, interval_seconds, jitter_seconds)` (`interval.rs:9`) returns
  `now + interval_seconds + rand(0..=jitter_seconds)` — i.e. strictly in the future by at least `interval_seconds`.
- `release_claim(db, task_id, next_run_at, result)` (`claim.rs`) writes `next_run_at`, clears the lock
  (`locked_by`/`locked_at` → NULL), and records run metadata; the `next_run_at` it stores is exactly the value
  passed. So passing `now` stores `now` — the released task is immediately due on any instance's next poll.
- The seeded task in the existing abort test uses `interval_seconds = 300`, `jitter_seconds = 30`,
  `next_run_at = now − 1 min` (`scheduler.rs` `cancellation_releases_claim`, seed at `:590-600`).

## Approach (chosen — reschedule the interrupted task to "now", YAGNI)

In the abort branch, pass **`now`** as `next_run_at` instead of the full-interval advance — delete the
`compute_next_run_at(...)` call at `:274` and pass the already-computed `now` (`:273`) straight through:

```rust
_ = abort.cancelled() => {
    tracing::debug!(
        task_id = %task.id,
        task_type = ?task.task_type,
        "scheduler hard-abort during task execution; releasing claim for immediate re-run"
    );
    // The run was interrupted (never completed), so do NOT advance next_run_at by a
    // full interval like the success path does. Reschedule to `now` so another
    // scheduler instance — or this one on restart — re-runs the interrupted task on
    // its next poll cycle, instead of skipping an entire interval (6 h / 24 h for
    // FetchReleases / DetectVersion). Releasing the claim only helps if the row is
    // also due.
    let now = time::OffsetDateTime::now_utc();
    if let Err(e) = claim::release_claim(
        &db,
        task.id,
        now,
        &Err("scheduler shutdown during execution".to_string()),
    )
    .await
    {
        tracing::warn!(
            task_id = %task.id,
            error = %e,
            "failed to release task claim on scheduler shutdown"
        );
    }
    return;
}
```

**Why `now` and not "preserve the row's existing `next_run_at`":** the row's current `next_run_at` was ≤ now (the
task was due, which is why it got claimed), so preserving it is *also* immediately-eligible and behaviorally
near-identical. `now` is chosen because it is the simpler, self-evident "re-run ASAP" value, requires no extra
column read, and the cadence anchor is not lost either way — the next **successful** run recomputes `next_run_at`
from *its* completion time (`:315`), so a one-off "run at now" does not compound or drift the schedule. This
matches the finding's primary suggested fix.

**No re-run storm:** the abort fires once per shutdown. The interrupted task runs once promptly (on another
instance, or on restart), completes normally, and its `next_run_at` returns to `now + interval`. There is no loop.

**`last_error` and `result` unchanged (deliberate):** the branch keeps passing
`&Err("scheduler shutdown during execution")`, so `last_error` still records *why* the run ended. That breadcrumb
is accurate (the run was interrupted) and orthogonal to the scheduling bug — not touched here.

## Interaction with the in-flight Scheduler Claim Lease Integrity spec (sequencing note)

`2026-07-11-scheduler-claim-lease-design.md` also edits this exact abort-branch `release_claim` call, but changes
a **different argument set**: it adds a `controller_id` parameter to `release_claim`, changes its return type to
`error::Result<u64>`, and makes both call sites inspect the rows-affected count. This spec changes **only the
`next_run_at` argument value** (`now` instead of the interval advance). The two edits are disjoint on the same
call and **rebase cleanly in either order**:

- If claim-lease lands first, this spec's edit simply passes `now` for the `next_run_at` arg of the new
  (`controller_id`-carrying) signature.
- If this spec lands first, claim-lease adds `controller_id` + count-inspection around the already-corrected
  `next_run_at` value.

No coordination required beyond a trivial rebase; the plan should apply against whichever is HEAD. Do **not** fold
the two — claim-lease is a HIGH-severity duplicate-execution fix with its own heartbeat machinery; keeping this
one-line MEDIUM reschedule fix standalone keeps each independently reviewable and revertable.

## Tests (extend the existing abort test — no new scaffolding)

`crates/core/scheduler-runtime/src/scheduler.rs` already has `cancellation_releases_claim`, which drives the
abort-branch-during-execution path deterministically: it registers a `BlockingExecutor` (oneshot-gated), waits for
the executor to start (task claimed), cancels `abort`, unblocks, waits for shutdown, and asserts `locked_by` is
`NULL` after release. This is the exact seam.

**Add one assertion** to that test, after the existing `locked_by.is_none()` check, on the same reloaded row:

```rust
// Regression (audit L923): an interrupted run must be rescheduled to ~now so it
// re-runs on the next poll — NOT deferred like a completed run. The 60s bound is a
// "near now" guard: it sits far above the test's realistic ~0-2s seed→abort gap
// (permitted ceiling ~10s: the two existing 5s tokio::time::timeout safety bounds)
// and far below the seeded 300s interval, so it catches ANY erroneous deferral.
let seed_now = now; // the `now` used to seed next_run_at = now - 1 min
assert!(
    after.next_run_at < seed_now + time::Duration::seconds(60),
    "interrupted task must be rescheduled to ~now, not deferred; got next_run_at = {} (seed_now = {})",
    after.next_run_at,
    seed_now,
);
```

- **Fails pre-fix, passes post-fix:** pre-fix the abort branch stores `now + 300 + jitter[0..=30]` (≈
  `seed_now + 300..=330`), far over the `60s` bound → fails; post-fix it stores a fresh `now` (≈ `seed_now`, at
  most a few seconds later, well under `60s`) → passes.
- **"Near now", not "under one interval":** the bound is deliberately a small fixed tolerance (`60s`), **not**
  `seed_now + interval_seconds` — this is a strictly tighter regression guard (matching the repo's own near-now
  assertion style, e.g. the `trigger_immediate` test in `claim.rs` asserting `next_run_at <= now`) that also
  catches a hypothetical partial/smaller-but-still-wrong deferral, and it does not duplicate the seed's
  `interval_seconds` magic number.
- **No flake, no timer dependency:** the test is oneshot-driven (the existing `tokio::time::timeout`s are safety
  bounds, not logic), and the assertion compares stored timestamps against a fixed `seed_now + 60s` bound — the
  real seed→abort elapsed is ~0-2s (permitted ceiling ~10s, the two 5s safety timeouts), leaving ≥50 s of
  headroom — so it does not depend on how fast the test runs. Consequently
  **no `start_paused`** is added: this test has no timer-driven logic (consistent with its current form and the
  testing rule — `start_paused` is required only when logic calls a `tokio::time::*` API for correctness).
  `time::Duration` (the `time` crate) is used throughout `scheduler.rs`/`claim.rs`, matching the seed's
  `time::Duration::minutes(1)`.
- **Success path already covered:** existing poll-cycle tests exercise the completed-run `next_run_at = now +
  interval` advance; this spec does not change that path, so no new success-path test is warranted (do not test
  `compute_next_run_at` in isolation — that is upstream/arithmetic behavior already unit-tested in `interval.rs`).

Covers the failure-path regression (interrupted → not deferred) directly. This is the one behavior the change
introduces.

## Deliverables

- `crates/core/scheduler-runtime/src/scheduler.rs` — abort branch (`:264-290`): pass `now` to `release_claim`
  (drop the `compute_next_run_at` call at `:274`); update the branch comment to state the interrupted-run
  reschedule-to-now rationale. Extend the `cancellation_releases_claim` test with the `next_run_at` assertion.

### Documentation deliverables

- **No external doc change.** The behavior is an internal scheduler-shutdown detail with no user-facing surface,
  config, or API; the rationale lives in the updated inline branch comment. `docs/development/scheduler-engine.md`
  is separately stale and is the subject of its own docs-drift spec
  (`2026-07-12-developer-docs-drift-sweep-design.md`) — do **not** touch it here.
- **No ADR** (bug fix, not an architectural decision). **No wire/OpenAPI/frontend/dependency change** —
  `scheduled_task.next_run_at` column, `release_claim` shape (per this spec), and the poll loop are otherwise
  unchanged.

## Alternatives considered

- **Preserve the row's existing `next_run_at`** (pass `task.next_run_at` unchanged) — viable and near-identical
  (the value is already ≤ now), but `now` is simpler, needs no reasoning about the seeded value, and is the
  finding's primary suggestion. Rejected as marginally more indirect for zero behavioral gain.
- **Leave `next_run_at` advanced but shorten the stale-claim recovery window instead** — rejected: that is the
  claim-lease spec's territory (staleness/heartbeat) and does not fix *this* bug; even with instant recovery, a
  released-but-not-due row is not picked up until its `next_run_at`. The reschedule is the direct fix.
- **Record the interruption as success (`&Ok(())`) instead of an error** — rejected: out of scope (a `last_error`
  semantics question, not the scheduling bug) and arguably wrong (the run *was* interrupted). Left unchanged.
- **Fold into the claim-lease spec** — rejected: different severity (HIGH vs MEDIUM), different concern (claim
  ownership/liveness vs run scheduling); keeping them separate preserves independent review/revert. See the
  sequencing note.

## Out of scope

Other unspecced Medium+ findings (web-api-routes L1246 Update/PATCH `Validate` gap, and the short-term-backlog
tier) — separate specs. Claim ownership scoping, the heartbeat/staleness mechanism, and the `release_claim`
signature change all belong to `2026-07-11-scheduler-claim-lease-design.md`. The `scheduler-engine.md` doc drift
belongs to the docs-drift sweep spec. No change to the success-path `next_run_at`, the `last_error`/`result`
value, the top-level `run()` abort arm, or tick-executor handling.
