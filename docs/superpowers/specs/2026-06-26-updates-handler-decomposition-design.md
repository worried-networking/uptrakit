# Spec: `updates.rs` Handler Decomposition & De-duplication

**Date:** 2026-06-26 **Target file:** `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
**Status:** Approved (alignment via Q&A), pending plan **Type:** Refactor — behavior-preserving. No
functional change.

## Problem

`updates.rs` scores **3.3 / 10 (red)** on CodeScene. It is 3737 lines, 77 functions, with an inline
`#[cfg(test)]` module of ~1150 lines. CodeScene flags:

| Smell                                       | Offenders                                                                                                                                                                                   |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Low Cohesion (LCOM4) + Brain Class (77 fns) | whole file                                                                                                                                                                                  |
| Complex Method (cc > 9)                     | `handle_update_result` (cc=22, 286 LoC), `prepare_pending_replay_messages` (cc=12), `process_single_batch_result` (cc=11), `BatchUpdateAuditSummary::outcome` (cc=11)                       |
| Bumpy Road / Deep Nesting (depth 4)         | `handle_update_result`, `prepare_pending_replay_messages`, `dispatch_next_queued_update_with_notifier`                                                                                      |
| Code Duplication                            | `finalize_post_update_*` pair, `emit_*_audit` pair, `dispatch_next_batch_update*` pair, `resolve_software_item_name`/`resolve_host_name` pair, installed-version update block, test helpers |
| Large Method / Excess Args                  | many                                                                                                                                                                                        |

The dominant drags are **file size + function count** (Low Cohesion / Brain Class). CodeScene scores
per file, so de-duplication alone barely moves these; splitting the file is the primary lever.

## Goal

Raise `updates.rs` (and every file it spawns) out of red — target **green (≥ 9.0)** for the facade
and each new **production** submodule, with no behavioral change. Verified by the existing test
suite plus per-file CodeScene re-scoring.

The optimized metric is **per-file Code Health** (the CodeScene `code_health_score` MCP tool / gate
value), not the hotspot/churn (health × change-frequency) view — a split does not reduce churn, so
the file may remain a hotspot in the temporal view even at green health. Hotspot standing is out of
scope. `updates/tests.rs` is **explicitly exempt** from the ≥ 9.0 gate (see Verification) — test
code is legitimately allowed lower health.

## Non-Goals

- No change to message semantics, ownership/CAS logic, audit outcomes, SSE/MQTT payloads, or DB
  queries.
- No new error types, no API surface change.
- No refactoring of sibling handler files (`messages.rs`, etc.) beyond unavoidable import-path
  updates.
- No new dependencies.

## Approach (approved)

Three coordinated moves, all behavior-preserving:

1. **Split** `updates.rs` → `updates/` subdirectory with a `mod.rs` facade.
2. **Move** the inline `#[cfg(test)]` module to `updates/tests.rs`.
3. **De-dup + faithfully extract** the flagged complex methods into named helpers _without changing
   logic_.

Rejected alternatives (from alignment Q&A):

- _In-file dedup only_ — leaves Low Cohesion / Brain Class untouched; score stays red.
- _Split + dedup only_ — leaves cc=22 / depth-4 smells in their new homes.
- _Flat sibling files_ — would churn `mod updates;` and break the stable `updates::` re-export path;
  rejected for the path-stable subdir.

This matches the codebase's active direction toward smaller handler units (the `handler/`
`session_authenticated`/`session_enrolled` extraction, spec
`2026-06-25-service-ws-handler-mod-split-design.md`). Note those are **flat siblings**, not a
subdir; the in-repo precedent for the `foo/mod.rs` submodule-directory pattern chosen here is
`crates/ui/web-api/src/routes/oauth/` and `.../routes/software_items/`.

## Module Layout

`updates.rs` becomes `updates/mod.rs` (facade) plus topical submodules. Proposed cut (final grouping
to be refined in the plan; each file targets < 70-LoC functions, cc < 9, well under the brain-class
threshold):

| Module           | Contents                                                                                                                                                                                                                                                                                                 |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `updates/mod.rs` | `mod` declarations; **facade re-exports** (below); file-level `#![expect(...)]` attrs; shared consts (`RECOVERY_FINALIZATION_TIMEOUT`); shared small types (`ReconnectSuccessorDispatchMode`, `ReplayPreparationNotifier`)                                                                               |
| `ownership.rs`   | `validate_host_link_visibility`                                                                                                                                                                                                                                                                          |
| `replay.rs`      | `PendingUpdateRecords`, `load_pending_update_records`, `recover_owned_updates_on_connect_with_dispatch_mode`, `prepare_pending_replay_messages` (+ extracted per-record helper), `fail_unreplayable_pending_update`, `build_execute_payload`, `merged_plugin_config`, `build_plugin_assignment_nullable` |
| `started.rs`     | `handle_update_started`, `broadcast_update_started_events`, `UpdateStartedInfo`                                                                                                                                                                                                                          |
| `output.rs`      | `handle_update_output`                                                                                                                                                                                                                                                                                   |
| `result.rs`      | `handle_update_result` (+ Stage-3 helpers _if_ that conditional stage runs — see Sequencing), `final_status_str`, `select_best_output`, `truncate_to_char_boundary`, `update_installed_version_on_success`, `emit_update_completed_event`, `dispatch_update_notification`                                |
| `batch.rs`       | `handle_batch_update_result`, `process_single_batch_result`, `BatchUpdateAuditSummary`, `BatchResultDisposition`, `handle_batch_completion`, `emit_batch_progress_event`, `emit_batch_progress_from_db`                                                                                                  |
| `dispatch.rs`    | `dispatch_next_batch_update` (+ `_for_replay`, `_with_notifier`), `dispatch_next_queued_update` (+ `_for_replay`, `_with_notifier`), `notify_failed_reconnect_update`                                                                                                                                    |
| `audit.rs`       | `UpdateLifecycleAuditCtx`, `emit_service_update_lifecycle_audit`, `emit_update_finalized_audit`, `emit_batch_update_finalized_audit`, `emit_stdin_attention_audit` (imports `super::result::final_status_str` for its one use)                                                                           |
| `lookups.rs`     | `resolve_software_item_name`, `resolve_host_name`                                                                                                                                                                                                                                                        |
| `finalize.rs`    | unified `finalize_post_update_best_effort`                                                                                                                                                                                                                                                               |
| `stdin.rs`       | `handle_stdin_attention`                                                                                                                                                                                                                                                                                 |
| `tests.rs`       | the moved `#[cfg(test)]` module                                                                                                                                                                                                                                                                          |

### Facade re-exports (visibility contract)

These items are consumed by sibling handler modules and MUST stay reachable at the
`handler::updates::` path. `updates/mod.rs` re-exports them with `pub(super) use` (or
`pub(crate) use` for `dispatch_next_batch_update`, which `handler/mod.rs` re-exports `pub(crate)`):

| Item                                                                                                                                                                                                                                    | Consumer(s)                                                        |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `ReconnectSuccessorDispatchMode`                                                                                                                                                                                                        | `session_authenticated.rs`, `reconnect.rs`, `message_processor.rs` |
| `recover_owned_updates_on_connect_with_dispatch_mode`                                                                                                                                                                                   | same three                                                         |
| `load_pending_update_records`, `PendingUpdateRecords` (type re-exported for return-type accessibility only — `session_authenticated.rs` discards the value with `let _` and never names the type, but it must resolve at the call site) | `session_authenticated.rs`                                         |
| `prepare_pending_replay_messages`                                                                                                                                                                                                       | `reconnect.rs`                                                     |
| `handle_update_started`, `handle_update_output`, `handle_update_result`, `handle_batch_update_result`, `handle_stdin_attention`                                                                                                         | `message_processor.rs`                                             |
| `resolve_software_item_name`, `resolve_host_name`, `emit_batch_progress_event`, `emit_batch_progress_from_db`, `handle_batch_completion`                                                                                                | `messages.rs`                                                      |
| `dispatch_next_batch_update` (`pub(crate)`)                                                                                                                                                                                             | `handler/mod.rs` re-export                                         |

External call sites (`message_processor.rs`, `session_authenticated.rs`, `reconnect.rs`,
`messages.rs`, `mod.rs`) keep their existing `updates::Foo` / `super::updates::Foo` paths
**unchanged** — the facade preserves them. `dispatch_next_batch_update` stays `pub(crate)` because
it is also reached from outside `handler/` — `update_reaper.rs` calls it as
`crate::routes::service_ws::handler::dispatch_next_batch_update` via the existing `handler/mod.rs`
`pub(crate) use`.

Visibility note: items keep their **current** declared visibility in their new submodule
(`pub(super)`/`pub(crate)`); the facade re-export sets the effective reach. `handle_stdin_attention`
is currently `pub(crate)` but has no consumer outside `handler/`, so a `pub(super) use` re-export is
sufficient — keep the item `pub(super)` in `stdin.rs`. This is a deliberate, safe tightening, not an
accident; the implementer should not preserve the wider `pub(crate)` by reflex.

### Internal path convention

Match the existing handler convention: every current sibling imports handler-level modules via
`super::` (e.g. `updates.rs` line 21 `use super::shared_types::{...}`; `messages.rs`
`super::audit_service::...`). Preserve that depth-1 `super::` style — do **not** introduce absolute
`crate::routes::service_ws::handler::...` paths (those are used in-repo only for things _outside_
`handler/`, e.g. `crate::AppState`).

Mechanism: `updates/mod.rs` (whose `super` is `handler`) brings the needed siblings into the
`updates` module with a private re-import:

```rust
use super::{shared_types, audit_service};
```

Each submodule (whose `super` is the `updates` facade) then writes the familiar
`use super::shared_types::{HandlerError, HandlerResult, MAX_UPDATE_OUTPUT_BYTES, ProcessorResponse, load_linked_host_ids};`
and `use super::audit_service::ingest_service_audit_event;` — a child module may reach its parent's
private `use` aliases, so no extra visibility is required. Intra-`updates` references use
`super::<submodule>` (e.g. `super::audit::emit_update_finalized_audit`). `updates/tests.rs` uses
`use super::*` via the facade plus explicit submodule imports as needed.

## De-duplication (all behavior-preserving)

1. **`finalize_post_update_best_effort` + `finalize_post_update_with_recovery_timeout_best_effort`**
   → one fn taking `recovery_timeout: Option<Duration>`. The shared `plugin-ops` hook block runs
   once; the protection call branches: `None` → `finalize_post_update`, `Some(t)` →
   `finalize_post_update_with_timeout(.., t)`. Warning messages parameterized by a `&str` context
   label. (High value, clearly harmless.)

2. **Installed-version update** — the 3-column `host_software_item` setter is duplicated in
   `update_installed_version_on_success` (filter by `host_id`+`software_item_id`) and inline in
   `process_single_batch_result` (filter by `host_software_item_id`). Extract a shared col-setter
   taking the filter `Condition`/closure; the two callers pass their filter. (Medium value.)

3. **`emit_update_finalized_audit` + `emit_stdin_attention_audit`** — both resolve
   `software_name`+`host_name`, build `target_display`, attach optional `reason_code`, and call
   `emit_service_update_lifecycle_audit`. Extract a shared scaffold helper that resolves names +
   display and injects `reason_code` into a caller-supplied `details` object. (Medium-high value.)

4. **`dispatch_next_batch_update_for_replay`** — collapses into the public
   `dispatch_next_batch_update`/`_with_notifier` family; the replay variant differs only by passing
   `ReplayPreparationNotifier`. Keep the `pub(crate)` `dispatch_next_batch_update` wrapper
   (re-exported); the replay path stays a thin internal caller. _(Low value, harmless; co-locating
   in `dispatch.rs` already removes the CodeScene pair flag.)_

5. **`resolve_software_item_name` / `resolve_host_name`** — _judgment call._ They look up different
   entities, map different fields (`name` vs `friendly_name`), and use different fallback strings. A
   generic helper would need closures and read _worse_. **Decision: do NOT force a generic** (per
   "idiomatic by default" / don't fight a tool with a bad abstraction). Caveat: CodeScene's
   duplication flag is computed _between the two bodies_ and travels with them regardless of which
   file they live in — co-locating in `lookups.rs` does **not** by itself clear that flag, and a
   two-function file with no shared state can itself trip Low Cohesion. Accept a possible yellow on
   `lookups.rs` rather than a contrived merge; if the dry-run (below) shows it red, fold the two
   bodies into one `resolve_name<E>(...)` generic after all. This is the one module whose grouping
   is explicitly allowed to stay below the green target.

## Faithful complex-method extraction (no logic change)

Stage mapping (see Sequencing): item **1** (`handle_update_result`) is **Stage 3 — conditional**;
items **2–5** are **Stage 1** (low-risk, always done). Item **5** (`process_single_batch_result`)
needs no dedicated extraction — it simplifies automatically once dedup #2 lifts the
installed-version block out of it.

1. **`handle_update_result` (cc=22, 286 LoC)** — extract three blocks verbatim into named async
   helpers in `result.rs`:
   - resumable-interception block (`InProgress → AwaitingRestart` CAS) →
     `try_intercept_resumable(...) -> bool` (true = handled, return early).
   - the `updated == 0` stale/unowned handling → `finalize_unowned_result(...) -> bool` (true =
     fully handled, caller returns early; false = fell through, e.g. `fail_pending_unowned_update`
     succeeded with rows > 0 and post-finalization side-effects must still run). Plain `bool` — no
     new enum; the helper name carries the meaning (do **not** introduce an `UnownedOutcome` /
     `ControlFlow` type for a private two-state return).
   - trailing dispatch + completion-event + notification + audit → `emit_result_side_effects(...)`.
     The top-level fn becomes a linear sequence; control flow and early-returns preserved exactly.
     The outer `updated` binding (the CAS row count) must remain in the top-level fn so the existing
     `if updated > 0 { finalize_post_update_best_effort(...) }` gate (current lines 1603–1613) still
     fires after the fall-through path; the extracted helper does not own or mutate `updated`.

2. **`prepare_pending_replay_messages` (cc=12, depth 4)** — extract the per-record loop body into
   `prepare_single_pending_record(...) -> PendingRecordOutcome` where
   `enum PendingRecordOutcome { Message(Box<ExecuteUpdatePayload>), Skipped, SpawnedOrchestrator, Failed }`.
   The outer loop matches on the outcome (push / continue / set `failed_any`). Flattens nesting from
   4 to ≤2.

3. **`BatchUpdateAuditSummary::outcome` (cc=11, complex conditionals)** — extract the boolean
   sub-expressions into named predicate methods (`is_total_success()`, `is_all_stale()`,
   `is_all_finalize_error()`, `has_partial_signal()`, etc.). Same truth table, named pieces — kills
   the "complex conditional" smell without altering logic. Apply equivalently to `reason_code()`.

4. **`dispatch_next_queued_update_with_notifier` (bumpy)** — extract the `load_target_for_dispatch`
   failure path (CAS-won record → mark Failed) into `fail_dispatch_target_load(...)`. Loop body
   flattens.

5. **`process_single_batch_result` (cc=11)** — after dedup #2 lands, the installed-version block
   leaves the function; the remaining record-lookup + finalize stays as one linear pass.

Each extraction keeps the original `#[tracing::instrument]` spans on the public entry points;
helpers are private (`fn`/`async fn`, no `pub`).

## File-level lint attributes

`updates.rs` carries `#![expect(clippy::indexing_slicing, ...)]` and
`#![expect(clippy::string_slice, ...)]`. After the split, these are needed only by the module that
owns `truncate_to_char_boundary` (the slice/index site) — move them to that submodule (`result.rs`)
as `#![expect(...)]`, not onto the facade. The test module's
`#![expect(clippy::panic / unwrap_used / expect_used)]` move with the tests to `updates/tests.rs`.
Per snapshot rule, every `expect` keeps its `reason = "..."`.

## Sequencing (risk-gated)

The work lands in stages so the high-risk extraction is **conditional**, not mandatory:

1. **Stage 1 — mechanical split + test move + safe dedups.** Create `updates/mod.rs` + submodules,
   move the production functions, move the test module to `updates/tests.rs`, and apply de-dups
   #1–#4 (the four clearly-safe merges) plus the low-risk extractions
   (`prepare_pending_replay_messages` per-record helper,
   `BatchUpdateAuditSummary::outcome`/`reason_code` predicate methods,
   `dispatch_next_queued_update_with_notifier` failure-path helper). Run the full gate.
2. **Stage 2 — re-score each production submodule.** Severity-3 wins (Low Cohesion, Brain Class)
   come entirely from the split and are realized here. Complex Method (severity 2) on
   `handle_update_result` may already be green once `result.rs` is ~400 lines instead of 3737.
3. **Stage 3 — conditional `handle_update_result` extraction.** Perform the cc=22 decomposition
   (`try_intercept_resumable`, `finalize_unowned_result`, `emit_result_side_effects`) **only if**
   `result.rs` is still red/yellow after Stage 2. This is the riskiest change on the hottest path
   (interleaved CAS row-counts, ownership early-returns, `#[cfg(feature = "plugin-ops")]` sub-blocks
   whose helper signature may differ across feature permutations); gating it behind the re-score
   converts a mandatory high-risk step into an optional one that may be skipped entirely.

**Cohesion dry-run (before finalizing the cut):** LCOM4 links functions by shared data members or
call edges, so a topical group of unrelated free functions can itself trip Low Cohesion. Before
committing the module boundaries, run `code_health_score` on the riskiest small/low-connectivity
candidates (`lookups.rs`, `audit.rs`) as throwaway files. Decide the final grouping by call-graph
connectivity, not topic labels: if a 2-function file keeps a duplication/cohesion flag, either merge
the bodies or accept yellow there explicitly.

## Risks & Mitigations

| Risk                                                            | Mitigation                                                                                                                                                                               |
| --------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Visibility breakage when items move one level deeper            | Facade re-export table above; `cargo check --all-features` + `--no-default-features --features db-sqlite` must both pass.                                                                |
| `#[cfg(feature = "plugin-ops")]` blocks split across modules    | Keep each cfg-gated block intact within one function; verify both feature permutations compile.                                                                                          |
| `handle_update_result` extraction regresses the update hot path | Gated behind Stage 3 — only attempted if needed; extract by _move_, not rewrite; integration suite (required) exercises CAS-loser × plugin-ops × fall-through paths unit tests may miss. |
| Topical grouping relocates Low Cohesion instead of clearing it  | Cohesion dry-run (above) on `lookups.rs`/`audit.rs` before committing the cut; group by call connectivity, not topic.                                                                    |
| Subtle logic drift during extraction                            | Extract by _move_, not rewrite; rely on the existing ~1150-line test suite (now `updates/tests.rs`) + the required integration suite.                                                    |
| CodeScene dup-flag gaming (splitting a pair across files)       | Genuine unifications (#1–#4) are real merges; #5 is an explicit, justified non-merge, not a hide.                                                                                        |
| Per-file score still yellow after split                         | Plan includes a CodeScene re-score gate per **production** file; iterate grouping if any file < 9.0. `tests.rs` exempt.                                                                  |

## Verification

Behavior-preserving refactor — green gates are the contract.

- `cargo fmt --all`
- `cargo check --no-default-features --features db-sqlite`
- `cargo check --all-features`
- `cargo clippy --all-targets --no-default-features --features db-sqlite`
- `cargo clippy --all-targets --all-features`
- `cargo test --all-features` (the moved test module must pass unchanged)
- `cargo deny check` (snapshot quality gate; no dependency change expected, so this is a no-op
  confirmation, but the gate is run for completeness)
- CodeScene `code_health_score` on `updates/mod.rs` and every new **production** submodule. The
  **enforceable floor is "none red" (≥ 4.0)**; green (≥ 9.0) is the aspirational target. A
  production file landing yellow-but-not-red — e.g. `result.rs` after a performed Stage 3, or
  `lookups.rs` per its caveat — is **not** a gate failure once max extraction is already spent.
  **`updates/tests.rs` is exempt** from this gate entirely: CodeScene counts test code, and the
  existing test module's flagged smells (duplicated helper pairs, 152/99/80-LoC test fns, a 7-arg
  insert helper) travel with the move; requiring "pass unchanged" and "≥ 9.0" of the same file is
  contradictory. Test code is allowed lower health. Test-helper de-dup / large-test splitting is
  **deferred** (out of scope here; see Deferred).
- **Docker system integration suite — required.** This file owns update-lifecycle / service-WS
  message handlers, which the snapshot's binding rule classes as service-lifecycle code
  ("Enrollment/wire/service lifecycle changes MUST run full system integration tests with Docker").
  The rule has no carve-out for behavior-preserving refactors, and integration tests are precisely
  what catch a regression that unit tests miss after a large extraction. Run:
  `docker build -f docker/Dockerfile.test -t uptrakit-test:latest . && cargo test -p uptrakit-integration-tests -- --ignored`.

## Documentation Deliverables

- **Root `AGENTS.md`** — line 1271 carries the `handler/updates.rs` table row in the handler-module
  table (added/maintained by commit `b5db727aa`). **Required:** replace that single row with rows
  for `updates/mod.rs` + the new submodules (mirroring how the `session_*` rows at lines 1263–1264
  were added for the prior split).
- No `README.md`, `CONTEXT.md`, ADR, or API-doc changes: this is an internal module reorganization
  with **no externally observable behavior, surface, config, or architecture change**. Explicitly
  out of scope.
- Module-level `//!` doc comment from the old file header is distributed: a short facade `//!` in
  `updates/mod.rs` plus a one-line `//!` per submodule describing its slice.

## Deferred / Out of Scope

- **Test-module health** — de-duplicating the test helper pairs and splitting the 152/99/80-LoC test
  functions in `updates/tests.rs`. The module moves _unchanged_ here and is exempt from the ≥ 9.0
  gate; raising its own health is a separate, optional follow-up.
- Splitting `messages.rs` (4469 lines) — separate effort.
- Reducing function _argument counts_ (the "Excess Arguments" smell on `emit_*` / `handle_*`) beyond
  what the audit-scaffold dedup (#3) naturally achieves — the remaining 5-arg handlers are
  message-processor entry points whose signatures are dictated by the dispatch layer; changing them
  is not harmless. Deferred.
- Any change to the `update_batches` / `update_dispatch` query modules.
