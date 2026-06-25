# Preserve streamed protection output on pre-dispatch failure

**Date:** 2026-06-25
**Status:** Approved (design)
**Scope:** Backend only (`controller-core`, `web-api-queries`). No frontend change. No schema change. No new API field.

## Problem

When a controller-side pre-update protection step fails _before_ the agent is
dispatched (e.g. a Proxmox snapshot/backup that errors or times out), the
update-history terminal modal renders an empty black terminal showing only a
single generic line:

> `Update failed before agent dispatch: controller pre-update protection failed.`

The real, useful output — the full streamed Proxmox API log plus the actual
error/timeout reason — is captured but never shown.

### Confirmed root cause

The Proxmox protection plugin streams its full log and its terminal error to a
channel (`crates/plugins/infrastructure/proxmox/src/update_protection.rs`, e.g.
lines 418-434, 445-446, 639-650, 666-667). A forwarder task
(`crates/ui/controller-core/src/update/controller.rs:315`,
`forward_protection_output` at 447-485) persists each line into the
`update_output_lines` table.

On failure, `fail_before_agent_dispatch`
(`crates/ui/web-api-queries/src/queries/update_dispatch.rs:555-587`) writes a
generic 77-byte placeholder into `update_history.output` (and `output_bytes`).

The read path consolidates output as:

```rust
// update_history.rs:295-299 (list) and 334-343 (detail)
let output = if record.output.is_empty() {
    load_output_lines(...)        // serve the streamed lines
} else {
    record.output.clone()         // serve the column verbatim
};
```

Because the placeholder makes `record.output` non-empty, the read path serves
the 77-byte string and **ignores every streamed line**. The data survives in
`update_output_lines` (verified in the live DB: the reported n8n failure has 35
lines / 2542 bytes ending in `plugin error: Timed out waiting for Proxmox task
UPID:...`), but is masked.

### Why "just stop writing output" is not enough

`update_history.output` is **authoritative at rest**. The normal agent
completion path consolidates the full output into the column via
`select_best_output` (`crates/ui/web-api/src/routes/service_ws/handler/updates.rs:1253-1292`)
and `finalize` (`update_batches/dispatch.rs:880-925`). The
`update_output_lines` table + the `is_empty()` fallback exist for the
_in-progress streaming window_. Leaving a terminal (failed) row with an empty
`output` column would create a new special-case shape that only works by leaning
on a fallback meant for live updates, and any consumer that reads the `output`
column directly (rather than through the read-path fallback) would see nothing.

The fix therefore makes failed pre-dispatch rows look like every other at-rest
row: the `output` column holds the consolidated streamed output.

## Goal

A pre-dispatch protection failure must return the full streamed protection
output (Proxmox log + the real error/timeout reason) to the frontend, persisted
in the authoritative `update_history.output` column — exactly as a normal
completion does.

## Approach (chosen: B — consolidate into the `output` column)

Mirror the existing finalization idiom. After the protection forwarder has
drained all streamed lines, consolidate them into the `output` column for every
pre-dispatch failure path.

### Constraint: the forwarder is async

`forward_protection_output` runs as a detached `tokio::spawn`
(`controller.rs:315`) and persists lines asynchronously. At the moment
`fail_before_agent_dispatch` currently runs, the lines are **not** flushed — the
sender(s) are still alive (`tx` inside `prepare_pre_update_protection`, and
`hook_tx` held in `run_protection_and_dispatch`). Consolidation therefore cannot
happen inside `fail_before_agent_dispatch`; it must happen after the forwarder
closes (all senders dropped → `rx.recv()` returns `None` → task ends).

### Changes

#### 1. `fail_before_agent_dispatch` — stop clobbering `output`

File: `crates/ui/web-api-queries/src/queries/update_dispatch.rs:555-587`

- Remove the `Output`, `OutputBytes`, and `OutputTruncated` column writes.
- Keep `Status = Failed`, `CompletedAt`, `PreUpdateProtectionStatus`.
- Pass the real summary through (see change 2), keeping
  `PreUpdateProtectionSummary`.
- Delete the now-unused constant `PRE_UPDATE_PROTECTION_FAILURE_OUTPUT`
  (lines 415-416).

#### 2. Preserve the real protection summary

File: `crates/ui/web-api-queries/src/queries/update_dispatch.rs:614-651`
(`prepare_pre_update_protection`)

- **Decision-failed path** (line 649): pass the plugin's own
  `decision.protection_summary` through to `PreUpdateProtectionSummary` instead
  of the generic `PRE_UPDATE_PROTECTION_FAILURE_SUMMARY`. Fall back to the
  generic summary only when the plugin supplied none.
- The generic `PRE_UPDATE_PROTECTION_FAILURE_SUMMARY` constant remains as the
  fallback; it is structured metadata surfaced in the modal's "Details" panel,
  not terminal output.

`PreUpdateProtectionSummary` is shown in the frontend "Details" dropdown
(`history/+page.svelte:527-533`); the terminal body is driven by `output`.

#### 3. Consolidate streamed lines into `output` after the forwarder drains

File: `crates/ui/controller-core/src/update/controller.rs`

- Capture the forwarder `JoinHandle` instead of discarding it
  (`let forwarder = tokio::spawn(forward_protection_output(...))` at line 315).

- **Deadlock invariant (mandatory):** `forwarder.await` returns only after the
  channel closes, which happens only when **every** sender is dropped — both the
  `tx` moved into `prepare_pre_update_protection` and the
  `#[cfg(feature = "plugin-ops")]` `hook_tx` clone (created at line 327, held in
  the outer scope, and moved into `prepare_pre_update_hook` on the `Proceed`
  path). Awaiting the forwarder while any sender is still in scope **hangs
  forever**. Implement this deadlock-free by construction: **confine the senders
  to an inner scope that owns `tx` and `hook_tx` and covers only the protection
  step and (on `Proceed`) the hook step, then returns the outcome.** That scope
  **must close before `dispatch_update_to_agent` is called** — `dispatch` runs
  _outside_ it. (Do **not** pull `dispatch` into the sender scope: if either
  sender's lifetime extends across the dispatch call, the dispatch-`Err`
  consolidation below joins a forwarder whose channel is still open and hangs.)
  Prefer this scoping over scattering `drop(tx)` / `drop(hook_tx)` calls. Any
  explicit `drop(hook_tx)` **must** be `#[cfg(feature = "plugin-ops")]`-gated
  (the binding is absent in non-`plugin-ops` builds).

- **Single consolidation point:** after the senders' scope has closed, branch on
  the outcome. On any pre-dispatch **failure**, join then consolidate:

  1. `if let Err(e) = forwarder.await { tracing::warn!(update_id = %id, error = %e, "protection output forwarder join failed"); }`
     — do **not** write `let _ = forwarder.await.ok()` (the `unused_result_ok`
     lint is DENY; `.ok()` discarding the `Result` fails to compile).
  2. `consolidate_protection_output(&db, update_history_id).await` (logs on its
     own error; failure to consolidate must not panic).

  On **success** (`Proceed` → dispatch ok), leave the forwarder detached as
  today — agent finalization consolidates protection + agent lines later via
  `select_best_output`. Do not join on the success path.

  The three failure outcomes that must consolidate:
  - `prepare_pre_update_protection` returned `Err` — controller.rs:338-359
  - `PreUpdateProtectionOutcome::Failed` — controller.rs:364-373
  - `dispatch_update_to_agent` returned `Err` — controller.rs:415-437 (protection
    already succeeded here; its snapshot log lines must still be consolidated)

- **Exclude the agent-disconnected arm.** `dispatch_update_to_agent` returning
  `Ok(false)` (controller.rs:405-414, agent disconnected between the connectivity
  check and dispatch) is **not** a terminal failure: the record is deliberately
  left `InProgress` for the reconnect-recovery path. This arm must **not**
  consolidate and must **not** call `fail_before_agent_dispatch`. Its protection
  lines stay in `update_output_lines`; because `output` is no longer clobbered,
  the read-path fallback serves them live during the `InProgress` window, and the
  later agent-result `finalize` / `select_best_output` consolidates protection +
  agent lines on reconnect. (Stated explicitly so the pattern is not
  "completed" onto this arm, which would wrongly finalize a live record.)

- Note: `fail_before_agent_dispatch` is invoked from two sites **inside**
  `prepare_pre_update_protection` (the plugin-error path at line 617 and the
  decision-failed path at line 649); both surface to the controller as
  `Ok(PreUpdateProtectionOutcome::Failed)`, so the single `Failed`-branch
  consolidation above covers them. The change-1 edit to `fail_before_agent_dispatch`
  itself applies to every call site uniformly.

#### 4. New query: `consolidate_protection_output`

File: `crates/ui/web-api-queries/src/queries/update_dispatch.rs` (new fn)

- **Truncation flag must be derivable.** The existing `load_output_lines`
  (`update_history.rs:152-171`) returns only `String` — it calls
  `append_output_with_cap` in a loop and `break`s at the cap but **discards** the
  truncation `bool`. To set `OutputTruncated` correctly, factor a shared
  `pub(crate)` helper that returns `(String, bool)` — `(consolidated, was_truncated)` —
  and have both `load_output_lines` and `consolidate_protection_output` call it
  (`load_output_lines` ignores the flag; nothing else changes for it). Do **not**
  just promote `load_output_lines` to `pub(crate)` as-is — it cannot supply the
  flag.
- Cap semantics: the helper uses `append_output_with_cap`, which **partial-appends**
  the final line up to the cap (it does not drop the last line whole, unlike
  `select_best_output`). This is the chosen behaviour for this path; it matches
  the read-path fallback exactly, so a consolidated row and a fallback-served row
  are byte-identical.
- Write via SeaORM `UpdateHistory::update_many().col_expr(...)` (no raw SQL):
  `Output` = consolidated string, `OutputBytes` = its byte length, `OutputTruncated`
  = the returned flag.
- Annotate `#[tracing::instrument(skip_all, fields(update_history_id = %update_history_id))]`;
  return `rootcause::Report` via `context_to()` (no `unwrap`/`expect`).
- **No transaction required:** the forwarder is joined _before_ this runs, so no
  concurrent writer appends lines during the read→write. The read of
  `update_output_lines` and the single `update_history` UPDATE are independent
  autocommit statements (different tables, no read-then-write of the same row).
  If the implementation nonetheless wraps both in one explicit transaction, it
  **must** use `begin_with_options(SqliteTransactionMode::Immediate)` per the
  read-then-write rule. Add a comment at the consolidation call site recording
  the invariant — _"autocommit is safe because `forwarder.await` above guarantees
  no concurrent writer on `update_output_lines`"_ — so the assumption is visible
  if a second forwarder is ever added.
- If there are zero streamed lines (a protection that failed without streaming
  anything), `output` ends up empty; the read-path fallback and the frontend
  `emptyState` ("No output recorded.") handle that gracefully. Optionally the
  error-path (controller.rs:338-359) may stream the raw `error` as one line
  before consolidating, so non-streaming plugins still yield a useful line —
  **deferred** (see below); Proxmox always streams its error today.

### Data flow after the fix

```text
protection plugin --stream--> tx --> forwarder --> update_output_lines  (unchanged)
                                          |
on failure: senders' scope closes -> forwarder.await -> consolidate_protection_output
                                          |
                          update_history.output = concat(lines)   <-- authoritative
                                          |
read path: record.output non-empty -> served verbatim -> frontend terminal renders full log
```

## What does NOT change

- **Frontend** (`history/+page.svelte`, `TerminalOutput.svelte`): zero change.
  With `output` populated, the existing rule
  `showTerminal = isLive || Boolean(output?.trim())` renders the full multi-line
  log in the terminal. The existing `emptyState` remains the fallback for a
  genuinely silent failure.
- **Database schema / migrations:** none.
- **API surface / response types:** none. No new field on
  `UpdateHistoryResponse`.
- **Success / agent-dispatch path:** unchanged — agent completion already
  consolidates protection + agent lines via `select_best_output`.

## Testing

Quality gates (per `docs/development/quality-gates.md`):

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
# DB/API change:
cargo test -p uptrakit-integration-tests --test database -- --ignored
```

New / updated tests:

1. **`fail_before_agent_dispatch` no longer writes `output`** — unit test
   asserting the row's `output` is untouched (stays empty) and `Status=Failed`,
   `PreUpdateProtectionStatus` are set.
2. **`consolidate_protection_output`** — insert several `update_output_lines`
   for an update, run the consolidation, assert `output` equals the ordered
   concatenation and `output_bytes` matches; assert `output_truncated=true` when
   the cap is exceeded; assert empty `output` when no lines exist.
3. **Decision summary preserved** — a `decision` with a non-empty
   `protection_summary` results in that summary on the row (not the generic).
4. **End-to-end read** — a failed pre-dispatch row with streamed lines returns
   the full streamed content (not the placeholder) through `get_update_history`
   / `list_update_history`. SQLite in-memory test DBs enforce FK constraints, so
   insert all required parent rows.
5. **Forwarder join terminates (no deadlock)** — a `run_protection_and_dispatch`
   failure path with both `tx` and (under `plugin-ops`) `hook_tx` must complete
   the `forwarder.await` and consolidate; guard against the hang regression by
   asserting the failure path returns and the row's `output` is populated. Wrap
   in a test timeout so a reintroduced deadlock fails fast rather than hanging
   the suite.

Follow `docs/development/testing.md`: test our consolidation/contract logic only;
no `thiserror` Display-string assertions; use `start_paused = true` only if a
test touches tokio time.

## Documentation deliverables

- **`docs/architecture/update-history-entity.md`** — document the contract:
  `output` is authoritative at rest for _all_ terminal states including
  pre-dispatch protection failures, which consolidate streamed
  `update_output_lines` into `output` (no generic placeholder). State that the
  `is_empty()` read-path fallback is for the in-progress streaming window only.
- No new ADR — this is a bugfix that conforms an existing path to the existing
  finalization pattern; no architectural decision changes.
- Public docstrings on `fail_before_agent_dispatch` and the new
  `consolidate_protection_output` describing the output contract.

## Deferred / out of scope

- Streaming the raw `error` as a synthesized output line in the early
  error-path so non-streaming protection plugins still produce a terminal line
  (Proxmox already streams its error/timeout). Add when a non-streaming
  protection plugin ships.
- Distinguishing timeout vs error visually in the UI.
- Any dedicated structured "failure detail" UI panel separate from the terminal.
- Any new `UpdateHistoryResponse` field or `output_kind` discriminator.

## Snapshot conformance

- `parking_lot` locking, `rootcause::Report` + `bail!`/`report!`, no
  `unwrap`/`expect`, `#[tracing::instrument(skip_all)]` with explicit fields —
  followed in new code.
- `unused_result_ok` (DENY): the forwarder-join result is consumed via
  `if let Err(e) = forwarder.await { … }`, never `.ok()` / `let _ =`.
- No raw SQL: consolidation writes via SeaORM `update_many().col_expr(...)`.
- SQLite read-then-write: consolidation runs **after** the forwarder is joined,
  so the line-read and the single `update_history` UPDATE are independent
  autocommit statements with no concurrent writer — `BEGIN IMMEDIATE` is not
  required. If both are wrapped in one explicit transaction, it must use
  `begin_with_options(SqliteTransactionMode::Immediate)`.
- Feature flags additive: any explicit `drop(hook_tx)` is
  `#[cfg(feature = "plugin-ops")]`-gated; no `#[cfg(not(...))]`.
- Conventional Commits: scopes match crate names (`fix(controller-core): …`,
  `fix(web-api-queries): …`).
