# updates.rs Handler Decomposition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lift `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` (3737 lines, CodeScene
3.3/red) out of red by splitting it into an `updates/` subdir + `mod.rs` facade, moving the inline
test module out, and applying behavior-preserving de-duplication + conditional complex-method
extraction.

**Architecture:** `updates.rs` becomes `updates/mod.rs` (a thin facade: `mod` declarations +
`pub(super) use`/`pub(crate) use` re-exports + shared consts/types) plus ~11 topical submodules.
Production code moves verbatim (`git mv` then cut/paste), references fixed at move time so the build
stays green at every commit. Four genuine de-dups and low-risk extractions land in Stage 1; the
risky `handle_update_result` (cc=22) extraction is **conditional** on a Stage-2 re-score.

**Tech Stack:** Rust 2024, `uptrakit-web-api`, SeaORM, axum, tokio, `rootcause::Report`,
`parking_lot`, CodeScene MCP (`code_health_score`).

## Global Constraints

Copied verbatim from the spec — every task implicitly includes these:

- **Behavior-preserving.** No change to message semantics, ownership/CAS logic, audit outcomes,
  SSE/MQTT payloads, or DB queries. Extract by **move, not rewrite**.
- **No new dependencies. No new error types. No public API surface change.**
- **Import convention:** handler-level siblings reached via `super::` (depth-1) through a facade
  re-import — NOT absolute `crate::routes::service_ws::handler::...` paths. Intra-`updates`
  references use `super::<submodule>`.
- **Lint attrs:** every suppression is `#[expect(lint, reason = "...")]` — never bare `#[allow]`
  (workspace denies `allow_attributes_without_reason`). Each `#[expect]` keeps its existing
  `reason`.
- **Sync locks:** `parking_lot::Mutex` only; drop guard before any `.await` (already the case —
  preserve it).
- **Commits:** Conventional Commits, `refactor(web-api): …` scope. End every commit message with the
  `Co-Authored-By` trailer below.
- **Quality gates (full suite before declaring done):** `cargo fmt --all`;
  `cargo check`/`cargo clippy --all-targets` on BOTH `--no-default-features --features db-sqlite`
  AND `--all-features`; `cargo test --all-features`; `cargo deny check`; CodeScene re-score; Docker
  integration suite (required — see Task 16); `markdownlint` on any `.md` touched.
- **Score contract:** enforceable floor is **"none red" (≥ 4.0)** per production file; green (≥ 9.0)
  aspirational. `updates/tests.rs` is **exempt** from the score gate.

Commit trailer (append to every commit body):

```text
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

Snapshot binding: rules above derive from `.superpowers/standards-snapshot.md` (coding-standards.md:
`#[expect(reason=)]`, parking_lot, non_exhaustive; quality-gates.md: gate suite + Docker rule;
commit-messages.md: Conventional Commits).

---

## File Structure

Target layout under `crates/ui/web-api/src/routes/service_ws/handler/updates/`:

| File           | Responsibility                                                                                                                                                                                                                                                                                         |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `mod.rs`       | Facade: `mod` decls, re-exports, shared consts/types (`RECOVERY_FINALIZATION_TIMEOUT`, `ReconnectSuccessorDispatchMode`, `ReplayPreparationNotifier`), private re-import `use super::{shared_types, audit_service};`, short crate `//!` doc.                                                           |
| `ownership.rs` | `validate_host_link_visibility`                                                                                                                                                                                                                                                                        |
| `lookups.rs`   | `resolve_software_item_name`, `resolve_host_name`                                                                                                                                                                                                                                                      |
| `finalize.rs`  | unified `finalize_post_update_best_effort` (dedup #1)                                                                                                                                                                                                                                                  |
| `audit.rs`     | `UpdateLifecycleAuditCtx`, `emit_service_update_lifecycle_audit`, `emit_update_finalized_audit`, `emit_batch_update_finalized_audit`, `emit_stdin_attention_audit` (dedup #3)                                                                                                                          |
| `replay.rs`    | `PendingUpdateRecords`, `load_pending_update_records`, `recover_owned_updates_on_connect_with_dispatch_mode`, `prepare_pending_replay_messages` + extracted per-record helper, `fail_unreplayable_pending_update`, `build_execute_payload`, `merged_plugin_config`, `build_plugin_assignment_nullable` |
| `started.rs`   | `handle_update_started`, `broadcast_update_started_events`, `UpdateStartedInfo`                                                                                                                                                                                                                        |
| `output.rs`    | `handle_update_output`                                                                                                                                                                                                                                                                                 |
| `result.rs`    | `handle_update_result` (+ Stage-3 helpers if run), `final_status_str`, `select_best_output`, `truncate_to_char_boundary`, `set_installed_version` (dedup #2), `update_installed_version_on_success`, `emit_update_completed_event`, `dispatch_update_notification`                                     |
| `dispatch.rs`  | `dispatch_next_batch_update` (+`_for_replay`,`_with_notifier`), `dispatch_next_queued_update` (+`_for_replay`,`_with_notifier`), `notify_failed_reconnect_update`                                                                                                                                      |
| `batch.rs`     | `handle_batch_update_result`, `process_single_batch_result`, `BatchUpdateAuditSummary` (+ predicate methods, dedup of complex conditional), `BatchResultDisposition`, `handle_batch_completion`, `emit_batch_progress_event`, `emit_batch_progress_from_db`                                            |
| `stdin.rs`     | `handle_stdin_attention`                                                                                                                                                                                                                                                                               |
| `tests.rs`     | the moved `#[cfg(all(test, feature = "db-sqlite"))]` module                                                                                                                                                                                                                                            |

Line ranges referenced below are from the **pre-refactor** `updates.rs` (3737 lines).

---

## Phase 1 — Stage 1: split, move, safe de-dup, low-risk extraction

### Task 1: Establish the `updates/` directory (zero code change)

**Files:**

- Rename: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` →
  `.../handler/updates/mod.rs`

**Interfaces:**

- Produces: nothing new — `mod updates;` in `handler/mod.rs` continues to resolve (Rust resolves
  `mod updates;` to either `updates.rs` or `updates/mod.rs`).

- [ ] **Step 1: Move the file with history preserved**

```bash
cd /Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/service_ws/handler
mkdir updates
git mv updates.rs updates/mod.rs
```

- [ ] **Step 2: Verify the build is unchanged**

Run: `cargo check -p uptrakit-web-api --no-default-features --features db-sqlite` Expected: PASS (no
errors; `handler/mod.rs` still finds the `updates` module).

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "refactor(web-api): move updates.rs into updates/mod.rs

Pure rename to establish the submodule directory; no code change.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Extract leaf utilities — `ownership.rs` + `lookups.rs`

**Files:**

- Create: `.../handler/updates/ownership.rs`
- Create: `.../handler/updates/lookups.rs`
- Modify: `.../handler/updates/mod.rs`

**Interfaces:**

- Produces:
  - `ownership::validate_host_link_visibility(db, service_id, update_history_id, linked_host_ids) -> HandlerResult<update_history::Model>`
  - `lookups::resolve_software_item_name(state, item_id) -> String`
  - `lookups::resolve_host_name(state, host_id) -> String`
  - Facade re-exports them at `updates::` for siblings (`resolve_*` consumed by `messages.rs`).

- [ ] **Step 1: Add the private sibling re-import to the facade**

In `updates/mod.rs`, directly below the existing `use` block (after the
`use super::shared_types::{...}` line, currently line 21–23), add:

```rust
// Bring handler-level siblings into the `updates` module so submodules can
// reach them via `super::` (depth-1 convention), not absolute crate paths.
use super::audit_service;
```

(`shared_types` is already imported at the facade; this adds `audit_service`, used later by
`started.rs`.)

- [ ] **Step 2: Create `ownership.rs`**

Cut `validate_host_link_visibility` (lines 63–94, including its `#[tracing::instrument]` and doc
comment) out of `mod.rs` into a new `ownership.rs`. Prepend the module doc + imports:

```rust
//! Host-link ownership validation for incoming update messages.

use std::collections::HashSet;

use rootcause::prelude::*;

use super::shared_types::{HandlerError, HandlerResult};

// <-- paste validate_host_link_visibility verbatim here -->
```

- [ ] **Step 3: Create `lookups.rs`**

Cut `resolve_software_item_name` (2195–2206) and `resolve_host_name` (2209–2217) into `lookups.rs`:

```rust
//! Display-name lookups for software items and hosts (best-effort, infallible).

use std::sync::Arc;

use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::{host, software_item};

use crate::AppState;

// <-- paste resolve_software_item_name and resolve_host_name verbatim here -->
```

Note: keep both `pub(super)`. Per spec dedup #5 they are deliberately NOT merged.

- [ ] **Step 4: Wire modules + re-exports in the facade**

In `updates/mod.rs`, add module declarations (alongside any others) and re-exports:

```rust
mod lookups;
mod ownership;

pub(super) use lookups::{resolve_host_name, resolve_software_item_name};
pub(super) use ownership::validate_host_link_visibility;
```

- [ ] **Step 5: Fix internal callers**

Anywhere still in `mod.rs` that called these unqualified now resolves through the `pub(super) use`
(same crate-path `validate_host_link_visibility` / `resolve_*`). No call-site edits needed inside
`mod.rs`. Confirm `messages.rs` still uses `super::updates::resolve_software_item_name` etc.
(unchanged).

- [ ] **Step 6: Build + test**

Run: `cargo check -p uptrakit-web-api --no-default-features --features db-sqlite` Expected: PASS.
Run: `cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected:
PASS (existing tests still green).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(web-api): extract updates ownership + lookups submodules

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `finalize.rs` + de-dup #1 (unified post-update finalization)

**Files:**

- Create: `.../handler/updates/finalize.rs`
- Modify: `.../handler/updates/mod.rs`

**Interfaces:**

- Consumes:
  `crate::queries::update_dispatch::{finalize_post_update, finalize_post_update_with_timeout, finalize_post_update_hook}`.
- Produces:
  `finalize::finalize_post_update_best_effort(state, record, recovery_timeout: Option<Duration>)`.
  Replaces BOTH old functions. Callers pass `None` for the normal path,
  `Some(RECOVERY_FINALIZATION_TIMEOUT)` for the reconnect-recovery path.

- [ ] **Step 1: Create `finalize.rs` with the merged function**

Cut `finalize_post_update_best_effort` (96–127) and
`finalize_post_update_with_recovery_timeout_best_effort` (129–164) out of `mod.rs`. Replace with one
merged function in `finalize.rs`:

```rust
//! Best-effort post-update finalization (hook scale-down + protection finalize).

use std::sync::Arc;
use std::time::Duration;

use uptrakit_shared_db::entity::update_history;

use crate::AppState;

/// Run post-update finalization best-effort: the resource-restore hook first
/// (when `plugin-ops` is enabled), then protection finalization.
///
/// `recovery_timeout`:
/// - `None` — normal completion path (`finalize_post_update`).
/// - `Some(t)` — reconnect-recovery path (`finalize_post_update_with_timeout`).
///
/// `context` distinguishes the two paths in warning logs.
pub(super) async fn finalize_post_update_best_effort(
    state: &Arc<AppState>,
    record: &update_history::Model,
    recovery_timeout: Option<Duration>,
) {
    let context = if recovery_timeout.is_some() {
        " during reconnect recovery"
    } else {
        ""
    };

    // Hook first (scale down) — must run before protection finalization.
    #[cfg(feature = "plugin-ops")]
    if let Err(error) = crate::queries::update_dispatch::finalize_post_update_hook(
        state.db(),
        state.controller_update_hook(),
        state.plugin.plugin_ops.as_ref(),
        record,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            update_id = %record.id,
            "post-update hook (resource restore) failed{context}"
        );
    }

    // Then protection finalization.
    let result = match recovery_timeout {
        Some(timeout) => {
            crate::queries::update_dispatch::finalize_post_update_with_timeout(
                state.db(),
                state.controller_update_protection(),
                record,
                timeout,
            )
            .await
        }
        None => {
            crate::queries::update_dispatch::finalize_post_update(
                state.db(),
                state.controller_update_protection(),
                record,
            )
            .await
        }
    };
    if let Err(error) = result {
        tracing::warn!(
            error = %error,
            update_id = %record.id,
            "post-update finalization failed{context}"
        );
    }
}
```

- [ ] **Step 2: Update all callers**

Find call sites of the two old functions and rewrite:

```bash
grep -rn "finalize_post_update_best_effort\|finalize_post_update_with_recovery_timeout_best_effort" updates/
```

- `finalize_post_update_best_effort(state, &rec)` →
  `finalize::finalize_post_update_best_effort(state, &rec, None)` (in `replay.rs` future home /
  `result.rs` / `batch.rs`; for now they are still in `mod.rs` so call
  `finalize::finalize_post_update_best_effort(state, &rec, None)`).
- `finalize_post_update_with_recovery_timeout_best_effort(state, record)` (called at line 589) →
  `finalize::finalize_post_update_best_effort(state, record, Some(RECOVERY_FINALIZATION_TIMEOUT))`.

- [ ] **Step 3: Wire module + re-export**

In `updates/mod.rs`:

```rust
mod finalize;

pub(super) use finalize::finalize_post_update_best_effort;
```

(The `pub(super) use` keeps the unqualified name working for the not-yet-moved callers in `mod.rs`.)

- [ ] **Step 4: Build both feature sets**

Run: `cargo check -p uptrakit-web-api --no-default-features --features db-sqlite` Expected: PASS.
Run: `cargo check -p uptrakit-web-api --all-features` Expected: PASS (verifies the
`#[cfg(feature = "plugin-ops")]` block compiles under both permutations).

- [ ] **Step 5: Test**

Run: `cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected:
PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(web-api): unify post-update finalization into finalize module

Merge finalize_post_update_best_effort and the recovery-timeout variant into
one fn taking Option<Duration>; shared plugin-ops hook block runs once.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: `audit.rs` + de-dup #3 (shared audit scaffold)

**Files:**

- Create: `.../handler/updates/audit.rs`
- Modify: `.../handler/updates/mod.rs`

**Interfaces:**

- Consumes: `lookups::resolve_software_item_name`, `lookups::resolve_host_name`,
  `super::result::final_status_str` (interim: `final_status_str` is still in `mod.rs` until Task 8 —
  see Step 4).
- Produces:
  - `audit::emit_update_finalized_audit(state, service_id, record, status, outcome, output_truncated, reason_code)`
  - `audit::emit_batch_update_finalized_audit(state, service_id, tenant_id, batch_id, summary)`
  - `audit::emit_stdin_attention_audit(state, service_id, record, hint, outcome, reason_code)`
  - `audit::emit_service_update_lifecycle_audit(...)`, `audit::UpdateLifecycleAuditCtx`
  - New private helper `resolve_target_display(state, record) -> (String, String, String)` returning
    `(software_name, host_name, "<sw> on <host>")`.

- [ ] **Step 1: Create `audit.rs` and move the audit functions**

Cut into `audit.rs`: `UpdateLifecycleAuditCtx` (166–171), `emit_service_update_lifecycle_audit`
(173–199), `emit_update_finalized_audit` (201–239), `emit_batch_update_finalized_audit` (291–324),
`emit_stdin_attention_audit` (326–362). Header:

```rust
//! Update-lifecycle audit emission (semantic Audit V2 `emit_event` path).

use std::sync::Arc;

use uptrakit_shared_db::entity::update_history;
use uptrakit_wire::UpdateFinalStatus;

use super::lookups::{resolve_host_name, resolve_software_item_name};
use crate::AppState;

// <-- paste UpdateLifecycleAuditCtx, emit_service_update_lifecycle_audit verbatim -->
```

- [ ] **Step 2: Add the shared target-display helper (dedup #3)**

`emit_update_finalized_audit` and `emit_stdin_attention_audit` both resolve names and build
`"<software_name> on <host_name>"`. Add this private helper to `audit.rs`:

```rust
/// Resolve the `(software_name, host_name, "<sw> on <host>")` triple used as
/// the audit `target_display` for update-lifecycle events.
async fn resolve_target_display(
    state: &Arc<AppState>,
    record: &update_history::Model,
) -> (String, String, String) {
    let software_name = resolve_software_item_name(state, record.software_item_id).await;
    let host_name = resolve_host_name(state, record.host_id).await;
    let display = format!("{software_name} on {host_name}");
    (software_name, host_name, display)
}
```

- [ ] **Step 3: Rewrite the two emit fns to use the helper (move, not behavior change)**

In `emit_update_finalized_audit`, replace the first three lines of the body:

```rust
    let software_name = resolve_software_item_name(state, record.software_item_id).await;
    let host_name = resolve_host_name(state, record.host_id).await;
    let mut details = serde_json::json!({ ... });
```

with:

```rust
    let (_software_name, _host_name, target_display) = resolve_target_display(state, record).await;
    let mut details = serde_json::json!({
        "batch_id": record.batch_id,
        "dispatch_mode": if record.batch_id.is_some() { "batch" } else { "queued" },
        "host_id": record.host_id,
        "interactive": record.interactive,
        "output_truncated": output_truncated,
        "software_item_id": record.software_item_id,
        "status": final_status_str(status),
    });
```

and replace the later `Some(format!("{software_name} on {host_name}"))` argument with
`Some(target_display)`. Apply the analogous change to `emit_stdin_attention_audit` (it has no
`final_status_str`/`output_truncated`/`status` keys — keep its existing `details` body, only swap
the name-resolution preamble and the `target_display` argument).

Verify the emitted JSON keys and ordering are byte-identical to the originals (the only change is
sharing the name lookups).

- [ ] **Step 4: Handle `final_status_str` (interim import)**

`emit_update_finalized_audit` calls `final_status_str`, which still lives in `mod.rs` until Task 8.
For now add to `audit.rs`:

```rust
use super::final_status_str;
```

(reaches the `mod.rs`-level fn via `super::`). Task 8 changes this to
`use super::result::final_status_str;`.

- [ ] **Step 5: Wire module + re-exports + fix callers**

In `updates/mod.rs`:

```rust
mod audit;

pub(super) use audit::{
    emit_batch_update_finalized_audit, emit_service_update_lifecycle_audit,
    emit_stdin_attention_audit, emit_update_finalized_audit,
};
```

`UpdateLifecycleAuditCtx` and `BatchUpdateAuditSummary` are constructed inside `mod.rs` callers;
ensure `UpdateLifecycleAuditCtx` is `pub(super)` in `audit.rs` and add
`pub(super) use audit::UpdateLifecycleAuditCtx;` if any not-yet-moved code names it (it is named
only inside the emit fns — confirm with grep; if not externally named, no re-export needed).

- [ ] **Step 6: Build + test (both feature sets for check)**

Run: `cargo check -p uptrakit-web-api --no-default-features --features db-sqlite` Run:
`cargo check -p uptrakit-web-api --all-features` Run:
`cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected: all
PASS. The audit-assertion tests (`broadcast_update_started_emits_semantic_audit_event`,
`handle_update_result_emits_update_finalized_audit_event`,
`handle_batch_update_result_emits_batch_update_finalized_audit_summary`) must pass unchanged — they
guard the JSON-shape invariant.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(web-api): extract updates audit submodule with shared display helper

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: `replay.rs` + low-risk extraction of the per-record loop body

**Files:**

- Create: `.../handler/updates/replay.rs`
- Modify: `.../handler/updates/mod.rs`

**Interfaces:**

- Consumes: `finalize::finalize_post_update_best_effort(.., None)`,
  `dispatch::notify_failed_reconnect_update` (still in `mod.rs` until Task 7 — reach via
  `super::notify_failed_reconnect_update`), `crate::queries::update_dispatch::*`,
  `crate::queries::update_triggers::PendingProtectionWork`.
- Produces:
  `replay::{PendingUpdateRecords, load_pending_update_records, recover_owned_updates_on_connect_with_dispatch_mode, prepare_pending_replay_messages, build_execute_payload}`;
  new private `enum PendingRecordOutcome` + `prepare_single_pending_record`.

- [ ] **Step 1: Move the replay functions**

Cut into `replay.rs`: `PendingUpdateRecords` (370–379), `load_pending_update_records` (386–536),
`recover_owned_updates_on_connect_with_dispatch_mode` (538–608), `prepare_pending_replay_messages`
(610–702), `fail_unreplayable_pending_update` (704–769), `build_execute_payload` (777–922),
`merged_plugin_config` (2423–2432), `build_plugin_assignment_nullable` (2434–2449). Header:

```rust
//! Reconnect recovery + pending-update replay preparation.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use time::OffsetDateTime;

use rootcause::prelude::*;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, service, software_item,
    update_history,
};
use uptrakit_wire::{ControllerMessage, ExecuteUpdatePayload, PluginAssignment};

use super::finalize::finalize_post_update_best_effort;
use super::shared_types::{HandlerError, HandlerResult, load_linked_host_ids};
use super::{ReconnectSuccessorDispatchMode, notify_failed_reconnect_update};
use crate::AppState;
```

Update the two `finalize_post_update_best_effort(state, &failed_record).await` and
`...with_recovery_timeout...` calls inside the moved code to the new `(.., None)` /
`(.., Some(RECOVERY_FINALIZATION_TIMEOUT))` signatures (line 589 →
`Some(super::RECOVERY_FINALIZATION_TIMEOUT)`; line 743 → `None`). Import
`RECOVERY_FINALIZATION_TIMEOUT` via `use super::RECOVERY_FINALIZATION_TIMEOUT;`.

- [ ] **Step 2: Extract the per-record loop body (depth-4 → ≤2)**

Add to `replay.rs` an outcome enum and helper, then rewrite the
`for update_record in &records.pending_updates` loop in `prepare_pending_replay_messages` to
delegate:

```rust
/// Result of preparing one pending record during reconnect replay.
enum PendingRecordOutcome {
    /// Replay this message to the agent.
    Message(Box<ExecuteUpdatePayload>),
    /// Skip silently (already-dispatched batch host, or orchestrator spawned).
    Skip,
    /// The record could not be reconstructed and was failed; retry the outer loop.
    Failed,
}

/// Prepare a single pending record. Mirrors the original inline loop body
/// exactly — no behavior change.
async fn prepare_single_pending_record(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    update_record: &update_history::Model,
    records: &PendingUpdateRecords,
    dispatched_batch_hosts: &mut HashSet<(uuid::Uuid, uuid::Uuid)>,
) -> HandlerResult<PendingRecordOutcome> {
    // Unprotected Pending -> spawn orchestrator instead of direct replay.
    if update_record.pre_update_protection_status.is_none() {
        let target = match crate::queries::update_dispatch::load_target_for_dispatch(
            state.db(),
            update_record.tenant_id,
            update_record.host_id,
            update_record.software_item_id,
        )
        .await
        {
            Ok(target) => target,
            Err(e) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    error = %e,
                    "could not load target for unprotected Pending update on reconnect; failing record"
                );
                if fail_unreplayable_pending_update(state, service_id, update_record).await? {
                    return Ok(PendingRecordOutcome::Failed);
                }
                return Ok(PendingRecordOutcome::Skip);
            }
        };
        let work = crate::queries::update_triggers::PendingProtectionWork {
            target,
            update_history_id: update_record.id,
            to_version: update_record.to_version.clone().unwrap_or_default(),
            release_info: None,
            interactive: update_record.interactive,
        };
        state.update_dispatcher.spawn_pending_protection(work);
        return Ok(PendingRecordOutcome::Skip);
    }

    if let Some(batch_id) = update_record.batch_id {
        let key = (batch_id, update_record.host_id);
        if !dispatched_batch_hosts.insert(key) {
            return Ok(PendingRecordOutcome::Skip);
        }
    }

    let Some(execute_payload) = build_execute_payload(update_record, records) else {
        if fail_unreplayable_pending_update(state, service_id, update_record).await? {
            return Ok(PendingRecordOutcome::Failed);
        }
        return Ok(PendingRecordOutcome::Skip);
    };

    tracing::info!(
        update_id = %update_record.id,
        %service_id,
        software = %records
            .sw_items_map
            .get(&update_record.software_item_id)
            .map(|i| i.name.as_str())
            .unwrap_or("?"),
        "prepared pending update on reconnect"
    );
    Ok(PendingRecordOutcome::Message(Box::new(execute_payload)))
}
```

Then the loop becomes:

```rust
        let mut dispatched_batch_hosts: HashSet<(uuid::Uuid, uuid::Uuid)> = HashSet::new();
        let mut failed_any = false;
        let mut messages = Vec::new();

        for update_record in &records.pending_updates {
            match prepare_single_pending_record(
                state,
                service_id,
                update_record,
                &records,
                &mut dispatched_batch_hosts,
            )
            .await?
            {
                PendingRecordOutcome::Message(payload) => {
                    messages.push(ControllerMessage::ExecuteUpdate(payload));
                }
                PendingRecordOutcome::Skip => {}
                PendingRecordOutcome::Failed => failed_any = true,
            }
        }

        if !failed_any {
            return Ok(messages);
        }
```

Verify: the original wrapped the payload as
`ControllerMessage::ExecuteUpdate(Box::new(execute_payload))` — `Message(Box<...>)` preserves the
single `Box` allocation exactly.

- [ ] **Step 3: Wire module + re-exports**

In `updates/mod.rs`:

```rust
mod replay;

pub(super) use replay::{
    PendingUpdateRecords, load_pending_update_records, prepare_pending_replay_messages,
    recover_owned_updates_on_connect_with_dispatch_mode,
};
```

(`build_execute_payload`, `merged_plugin_config`, `build_plugin_assignment_nullable`,
`fail_unreplayable_pending_update` stay private to `replay.rs` — confirm no external caller via
grep.)

- [ ] **Step 4: Build + test**

Run: `cargo check -p uptrakit-web-api --no-default-features --features db-sqlite` Run:
`cargo check -p uptrakit-web-api --all-features` Run:
`cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected: all
PASS. Replay tests (`insert_replayable_queued_update`, `insert_pending_update_without_assignment`,
replay-failure protection tests) guard this.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(web-api): extract updates replay submodule; flatten per-record loop

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: `started.rs` + `output.rs`

**Files:**

- Create: `.../handler/updates/started.rs`, `.../handler/updates/output.rs`
- Modify: `.../handler/updates/mod.rs`

**Interfaces:**

- Consumes: `ownership::validate_host_link_visibility`, `lookups::resolve_*`, `audit::*` is NOT used
  here; `super::audit_service::ingest_service_audit_event`, `batch::emit_batch_progress_event`
  (still in `mod.rs` until Task 9 — reach via `super::emit_batch_progress_event`).
- Produces: `started::{handle_update_started, broadcast_update_started_events, UpdateStartedInfo}`,
  `output::handle_update_output`.

- [ ] **Step 1: Create `started.rs`**

Cut `UpdateStartedInfo` (930–935), `broadcast_update_started_events` (939–1055),
`handle_update_started` (1060–1144). Header:

```rust
//! `UpdateStarted` message handling and the started-event broadcast.

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::service;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::{AuditEventPayload, UpdateStartedPayload};

use super::lookups::{resolve_host_name, resolve_software_item_name};
use super::shared_types::ProcessorResponse;
use super::{audit_service, emit_batch_progress_event, validate_host_link_visibility};
use crate::AppState;
```

(`emit_batch_progress_event` is reached through the `pub(super) use` in `mod.rs`; it is added in
Task 9 — for now it still resolves because the fn is defined in `mod.rs`. Confirm the
`super::emit_batch_progress_event` path resolves; if not yet re-exported, call
`super::emit_batch_progress_event` directly to the `mod.rs` item.)

- [ ] **Step 2: Create `output.rs`**

Cut `handle_update_output` (1153–1223). Header:

```rust
//! `UpdateOutput` message handling (owner-safe persist + broadcast).

use std::collections::HashSet;
use std::sync::Arc;

use uptrakit_wire::UpdateOutputPayload;

use super::shared_types::ProcessorResponse;
use super::validate_host_link_visibility;
use crate::AppState;
```

- [ ] **Step 3: Wire modules + re-exports**

```rust
mod output;
mod started;

pub(super) use output::handle_update_output;
pub(super) use started::handle_update_started;
```

(`broadcast_update_started_events` / `UpdateStartedInfo` stay private unless a test names them —
grep `tests` mod: `broadcast_update_started_emits_semantic_audit_event` calls
`broadcast_update_started_events`. Add `pub(super) use started::broadcast_update_started_events;`
and `pub(super) use started::UpdateStartedInfo;` so the test module — and Task 11's `use super::*` —
keeps resolving.)

- [ ] **Step 4: Build + test**

Run: `cargo check -p uptrakit-web-api --all-features` Run:
`cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(web-api): extract updates started + output submodules

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: `dispatch.rs` + de-dup #4 (collapse the replay wrapper)

**Files:**

- Create: `.../handler/updates/dispatch.rs`
- Modify: `.../handler/updates/mod.rs`, `.../handler/mod.rs` (re-export path unchanged — verify
  only)

**Interfaces:**

- Consumes: `lookups`, `batch::handle_batch_completion` + `batch::emit_batch_progress_*` (still in
  `mod.rs` until Task 9 — reach via `super::`), `ReplayPreparationNotifier`,
  `crate::queries::update_batches::dispatch_next_in_batch`.
- Produces, in `dispatch`: `dispatch_next_batch_update` (`pub(crate)`),
  `dispatch_next_queued_update`, `notify_failed_reconnect_update`,
  `dispatch_next_batch_update_for_replay`, `dispatch_next_queued_update_for_replay`.

- [ ] **Step 1: Move dispatch functions**

Cut into `dispatch.rs`: `notify_failed_reconnect_update` (1742–1797), `dispatch_next_batch_update`
(1808–1823), `dispatch_next_batch_update_for_replay` (1825–1841),
`dispatch_next_batch_update_with_notifier` (1843–1893), `dispatch_next_queued_update_for_replay`
(1954–1997), `dispatch_next_queued_update` (2004–2010), `dispatch_next_queued_update_with_notifier`
(2012–2123). Header:

```rust
//! Dispatch of successor updates (batch + queue) and reconnect-failure notify.

use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uptrakit_shared_db::entity::{service, update_history};
use uptrakit_web_api_types::events::AdminEvent;

use super::{
    ReconnectSuccessorDispatchMode, ReplayPreparationNotifier, emit_batch_progress_from_db,
    handle_batch_completion,
};
use crate::AppState;
```

- [ ] **Step 2: De-dup #4 — drop the standalone `_for_replay` batch wrapper**

`dispatch_next_batch_update_for_replay` only differs from `dispatch_next_batch_update` by passing
`ReplayPreparationNotifier`. Keep `dispatch_next_batch_update_with_notifier` as the single
implementation. Replace `dispatch_next_batch_update_for_replay`'s body so it is a one-line delegate
(it already is) — leave it as the named entry point used by `notify_failed_reconnect_update`'s
`ReplayPrepared` arm. No behavior change; this is co-location, the dup flag clears because both
wrappers now live beside `_with_notifier` in one file. (Do NOT delete `dispatch_next_batch_update` —
it is `pub(crate)` and re-exported.)

- [ ] **Step 3: Wire module + re-exports; verify external path**

In `updates/mod.rs`:

```rust
mod dispatch;

pub(crate) use dispatch::dispatch_next_batch_update;
pub(super) use dispatch::notify_failed_reconnect_update;
```

`handler/mod.rs` already has `pub(crate) use updates::dispatch_next_batch_update;` — verify it still
compiles (the name now resolves through `updates`'s `pub(crate) use`). Confirm
`update_reaper.rs:106` (`crate::routes::service_ws::handler::dispatch_next_batch_update`) still
resolves.

- [ ] **Step 4: Build + test**

Run: `cargo check -p uptrakit-web-api --all-features` Run:
`cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(web-api): extract updates dispatch submodule

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: `result.rs` + de-dup #2 (shared installed-version setter) — NO Stage-3 extraction yet

**Files:**

- Create: `.../handler/updates/result.rs`
- Modify: `.../handler/updates/mod.rs`, `.../handler/updates/audit.rs`

**Interfaces:**

- Consumes: `ownership`, `lookups`, `finalize::finalize_post_update_best_effort`,
  `audit::emit_update_finalized_audit`,
  `dispatch::{dispatch_next_batch_update, dispatch_next_queued_update}`,
  `batch::emit_batch_progress_event` (via `super::`), `crate::queries::update_batches::*`.
- Produces, in `result`: `handle_update_result`, `final_status_str`, `select_best_output`,
  `truncate_to_char_boundary`, `update_installed_version_on_success`, `set_installed_version`,
  `emit_update_completed_event`, `dispatch_update_notification`.
  - `result::set_installed_version(state, filter: sea_orm::Condition, version: &str)` — shared
    col-setter (dedup #2), consumed by `batch.rs` in Task 9.

- [ ] **Step 1: Move the result functions verbatim**

Cut into `result.rs`: `final_status_str` (1230–1235), `truncate_to_char_boundary` (1237–1247),
`select_best_output` (1254–1293), `update_installed_version_on_success` (1296–1326),
`emit_update_completed_event` (1329–1350), `dispatch_update_notification` (1353–1404),
`handle_update_result` (1409–1732). Header:

```rust
//! `UpdateResult` message handling: finalize, output selection, side-effects.
#![expect(clippy::indexing_slicing, reason = "index is computed to be in bounds")]
#![expect(
    clippy::string_slice,
    reason = "slice index is at a validated char boundary"
)]

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder};
use time::OffsetDateTime;

use uptrakit_shared_db::entity::{host, host_software_item, service, software_item, update_history, update_output_line};
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::{UpdateFinalStatus, UpdateResultPayload};

use super::audit::emit_update_finalized_audit;
use super::finalize::finalize_post_update_best_effort;
use super::lookups::{resolve_host_name, resolve_software_item_name};
use super::shared_types::{MAX_UPDATE_OUTPUT_BYTES, ProcessorResponse};
use super::validate_host_link_visibility;
use super::{dispatch_next_batch_update, emit_batch_progress_event};
use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
```

The two file-level `#![expect]` attrs move here from `mod.rs` (this module owns the only slice/index
site, `truncate_to_char_boundary`). **Remove them from `mod.rs`.**

- [ ] **Step 2: Move the `#![expect]` attrs off the facade**

In `updates/mod.rs`, delete the top-of-file:

```rust
#![expect(clippy::indexing_slicing, reason = "index is computed to be in bounds")]
#![expect(
    clippy::string_slice,
    reason = "slice index is at a validated char boundary"
)]
```

(They now live on `result.rs`.) Verify `cargo clippy` does not complain about an unfulfilled
`expect` on `mod.rs` — if it does, that means another slice site remains in `mod.rs`; grep `\[\.\.`
/ indexing and relocate accordingly.

- [ ] **Step 3: De-dup #2 — shared installed-version setter**

Replace `update_installed_version_on_success`'s body to delegate to a new shared setter; add
`set_installed_version`:

```rust
/// Set `host_software_item.installed_version` (+ detected_at, last_updated_at)
/// for rows matching `filter`. Shared by the standalone and batch result paths.
pub(super) async fn set_installed_version(
    state: &Arc<AppState>,
    filter: Condition,
    to_version: &str,
) {
    let now = time::OffsetDateTime::now_utc();
    if let Err(e) = host_software_item::Entity::update_many()
        .col_expr(
            host_software_item::Column::InstalledVersion,
            sea_orm::sea_query::Expr::value(Some(to_version.to_string())),
        )
        .col_expr(
            host_software_item::Column::InstalledVersionDetectedAt,
            sea_orm::sea_query::Expr::value(Some(now)),
        )
        .col_expr(
            host_software_item::Column::LastUpdatedAt,
            sea_orm::sea_query::Expr::value(Some(now)),
        )
        .filter(filter)
        .exec(state.db())
        .await
    {
        tracing::warn!(error = %e, "failed to update host_software_item installed_version");
    }
}

/// Update installed version for the standalone (host_id + software_item_id) path.
async fn update_installed_version_on_success(
    state: &Arc<AppState>,
    host_id: uuid::Uuid,
    software_item_id: uuid::Uuid,
    to_version: &str,
) {
    set_installed_version(
        state,
        Condition::all()
            .add(host_software_item::Column::HostId.eq(host_id))
            .add(host_software_item::Column::SoftwareItemId.eq(software_item_id)),
        to_version,
    )
    .await;
}
```

Verify the column set, values (`Some(version)`, `now`, `now`), and warn message match the originals
exactly.

- [ ] **Step 4: Update `audit.rs`'s `final_status_str` import**

In `audit.rs`, change `use super::final_status_str;` → `use super::result::final_status_str;`.

- [ ] **Step 5: Update the inline finalize-call signature inside `handle_update_result`**

The moved `handle_update_result` calls `finalize_post_update_best_effort(state, &finalized_record)`
(line ~1612) and `fail_pending_unowned_update` etc. Update the finalize call to
`finalize_post_update_best_effort(state, &finalized_record, None)`. Leave all
CAS/ownership/early-return control flow byte-identical — **no Stage-3 extraction in this task.**

- [ ] **Step 6: Wire module + re-exports**

In `updates/mod.rs`:

```rust
mod result;

pub(super) use result::handle_update_result;
```

(`final_status_str` is now `result::final_status_str`; `audit.rs` imports it directly. If the test
module references `final_status_str` or `select_best_output`, add
`pub(super) use result::{final_status_str, select_best_output};` — grep tests to confirm.)

- [ ] **Step 7: Build both feature sets + clippy + test**

Run: `cargo check -p uptrakit-web-api --no-default-features --features db-sqlite` Run:
`cargo check -p uptrakit-web-api --all-features` Run:
`cargo clippy -p uptrakit-web-api --all-targets --all-features` Expected: PASS, no
unfulfilled-`expect` warnings. Run:
`cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected: PASS
(incl. `handle_update_result_emits_update_finalized_audit_event`).

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(web-api): extract updates result submodule; share installed-version setter

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: `batch.rs` + predicate extraction + consume shared setter

**Files:**

- Create: `.../handler/updates/batch.rs`
- Modify: `.../handler/updates/mod.rs`

**Interfaces:**

- Consumes: `result::set_installed_version` (dedup #2), `finalize`, `lookups`,
  `dispatch::dispatch_next_batch_update`, `audit::emit_batch_update_finalized_audit`,
  `crate::batch_progress_broadcaster::*`, `crate::queries::update_batches::*`.
- Produces:
  `batch::{handle_batch_update_result, handle_batch_completion, emit_batch_progress_event, emit_batch_progress_from_db, BatchUpdateAuditSummary, BatchResultDisposition}`.

- [ ] **Step 1: Move the batch functions**

Cut into `batch.rs`: `BatchUpdateAuditSummary` (241–282), `BatchResultDisposition` (284–289),
`handle_batch_completion` (1897–1945), `emit_batch_progress_event` (2130–2140),
`emit_batch_progress_from_db` (2143–2192), `process_single_batch_result` (2225–2346),
`handle_batch_update_result` (2352–2410). Header:

```rust
//! `BatchUpdateResult` handling, batch completion, and progress emission.

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use time::OffsetDateTime;

use uptrakit_shared_db::entity::{host_software_item, service, update_history};
use uptrakit_shared_types::BatchStatus;
use uptrakit_wire::{BatchUpdateResultPayload, UpdateFinalStatus};

use super::audit::emit_batch_update_finalized_audit;
use super::finalize::finalize_post_update_best_effort;
use super::result::set_installed_version;
use super::shared_types::ProcessorResponse;
use super::{dispatch_next_batch_update, validate_host_link_visibility};
use crate::AppState;
use crate::notifications::events::NotificationEvent;
```

Update `finalize_post_update_best_effort(state, &finalized_record)` → `(.., None)`.

- [ ] **Step 2: De-dup #2 consume — replace the inline installed-version block**

In `process_single_batch_result`, replace the inline
`host_software_item::Entity::update_many()...filter(...Id.eq(result.host_software_item_id))...`
block (originally lines 2315–2338) with:

```rust
    if result.status == UpdateFinalStatus::Completed
        && let Some(ref new_version) = result.installed_version
    {
        set_installed_version(
            state,
            sea_orm::Condition::all()
                .add(host_software_item::Column::Id.eq(result.host_software_item_id)),
            new_version,
        )
        .await;
    }
```

Verify columns/values identical to the original block.

- [ ] **Step 3: Predicate extraction on `BatchUpdateAuditSummary` (kill complex-conditional smell)**

Add named predicate methods and rewrite `outcome()`/`reason_code()` to use them — same truth table,
no logic change:

```rust
impl BatchUpdateAuditSummary {
    fn is_total_success(&self) -> bool {
        self.result_count == 0
            || (self.completed_count == self.result_count
                && self.failed_count == 0
                && self.stale_count == 0
                && self.finalize_error_count == 0)
    }

    fn has_partial_signal(&self) -> bool {
        self.completed_count > 0
            || (self.failed_count > 0 && self.stale_count > 0)
            || self.finalize_error_count > 0
    }

    fn all_stale(&self) -> bool {
        self.result_count > 0 && self.stale_count == self.result_count
    }

    fn all_finalize_error(&self) -> bool {
        self.result_count > 0 && self.finalize_error_count == self.result_count
    }

    fn all_failed(&self) -> bool {
        self.result_count > 0 && self.failed_count == self.result_count
    }

    fn outcome(&self) -> uptrakit_audit_log::AuditOutcome {
        if self.is_total_success() {
            uptrakit_audit_log::AuditOutcome::Success
        } else if self.has_partial_signal() {
            uptrakit_audit_log::AuditOutcome::Partial
        } else if self.stale_count == self.result_count {
            uptrakit_audit_log::AuditOutcome::Denied
        } else {
            uptrakit_audit_log::AuditOutcome::Failed
        }
    }

    fn reason_code(&self) -> Option<&'static str> {
        if self.all_stale() {
            Some("not_owned")
        } else if self.all_finalize_error() {
            Some("finalization_error")
        } else if self.all_failed() {
            Some("agent_reported_failure")
        } else {
            None
        }
    }
}
```

Cross-check against the original `outcome()` (251–269) and `reason_code()` (271–281): the `Denied`
arm still uses `stale_count == result_count` (not `all_stale()`, to keep the `result_count == 0`
edge identical — at that point `is_total_success()` already returned `Success`, so the difference is
unreachable but preserve the original expression to be safe).

- [ ] **Step 4: Wire module + re-exports**

```rust
mod batch;

pub(super) use batch::{
    emit_batch_progress_event, emit_batch_progress_from_db, handle_batch_completion,
    handle_batch_update_result,
};
```

(`BatchUpdateAuditSummary` / `BatchResultDisposition` stay private to `batch.rs` unless the test
module names them — `handle_batch_update_result_emits_batch_update_finalized_audit_summary`
exercises via the public handler, so likely private. Grep to confirm.)

- [ ] **Step 5: Build both feature sets + test**

Run: `cargo check -p uptrakit-web-api --all-features` Run:
`cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected: PASS
(batch audit-summary test guards `outcome`/`reason_code`).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(web-api): extract updates batch submodule; name audit-summary predicates

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 10: `stdin.rs`

**Files:**

- Create: `.../handler/updates/stdin.rs`
- Modify: `.../handler/updates/mod.rs`

**Interfaces:**

- Consumes: `ownership::validate_host_link_visibility`, `audit::emit_stdin_attention_audit`,
  `crate::queries::update_batches::touch_stdin_attention_if_owned`.
- Produces: `stdin::handle_stdin_attention` (kept `pub(super)`, see Global Constraints — no consumer
  outside `handler/`).

- [ ] **Step 1: Move `handle_stdin_attention`**

Cut `handle_stdin_attention` (2455–2582) into `stdin.rs`. Change its declaration from
`pub(crate) async fn` to `pub(super) async fn` (deliberate, safe tightening — only
`message_processor.rs` inside `handler/` calls it). Header:

```rust
//! `StdinAttention` message handling (broadcast + notify + audit).

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::{host, update_history};

use super::audit::emit_stdin_attention_audit;
use super::shared_types::ProcessorResponse;
use super::validate_host_link_visibility;
use crate::AppState;
```

- [ ] **Step 2: Wire module + re-export**

```rust
mod stdin;

pub(super) use stdin::handle_stdin_attention;
```

Verify `message_processor.rs:379` (`updates::handle_stdin_attention`) still resolves.

- [ ] **Step 3: Build + test**

Run: `cargo check -p uptrakit-web-api --all-features` Run:
`cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor(web-api): extract updates stdin submodule

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 11: Move the test module to `updates/tests.rs`

**Files:**

- Create: `.../handler/updates/tests.rs`
- Modify: `.../handler/updates/mod.rs`

**Interfaces:**

- Consumes (now from submodules): every production item the tests exercise — public handlers via the
  facade, plus `broadcast_update_started_events`, `select_best_output`, etc., reached through facade
  re-exports added in earlier tasks.

- [ ] **Step 1: Move the test module**

Cut the entire `#[cfg(all(test, feature = "db-sqlite"))] mod tests { ... }` block (2584–3737) out of
`mod.rs` into `updates/tests.rs`. Strip the outer `mod tests {` / closing `}` wrapper — `tests.rs`
IS the module body. Keep the inner
`#![expect(clippy::panic / unwrap_used / expect_used, reason = ...)]` inner attributes at the top of
`tests.rs`.

- [ ] **Step 2: Declare the module in the facade**

In `updates/mod.rs`:

```rust
#[cfg(all(test, feature = "db-sqlite"))]
mod tests;
```

- [ ] **Step 3: Fix imports in `tests.rs`**

The block starts with `use super::*;`. After the split, `super` from `tests.rs` is the `updates`
facade, so `use super::*;` pulls in all facade re-exports — most names resolve. Build and add
explicit imports for any production item the tests name that is NOT re-exported through the facade:

Run:
`cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates::tests --no-run`
Expected: may FAIL with `cannot find function/type ... in this scope`. For each unresolved
production name, add `use super::<submodule>::<name>;` to `tests.rs` (e.g.
`use super::started::broadcast_update_started_events;`, `use super::result::select_best_output;`).
Re-run until it compiles. Prefer explicit submodule imports over widening facade re-exports, unless
the item is already part of the documented facade surface.

- [ ] **Step 4: Run the full updates test suite**

Run: `cargo test -p uptrakit-web-api --no-default-features --features db-sqlite updates` Expected:
PASS — same test count as before the move. Run:
`cargo test -p uptrakit-web-api --all-features updates` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(web-api): move updates test module to updates/tests.rs

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 12: Finalize the facade + full gate

**Files:**

- Modify: `.../handler/updates/mod.rs`

**Interfaces:**

- Produces: the complete, documented facade. No production logic remains in `mod.rs` except shared
  consts/types + re-exports.

- [ ] **Step 1: Audit what remains in `mod.rs`**

`mod.rs` should now contain ONLY: the crate `//!` doc, `use` imports actually needed by the
consts/types, `RECOVERY_FINALIZATION_TIMEOUT` (38), `ReconnectSuccessorDispatchMode` (40–44),
`ReplayPreparationNotifier` + its `ServiceNotifier` impl (46–53), all `mod` declarations, all
`pub(super) use`/`pub(crate) use` re-exports, and the `use super::{shared_types, audit_service};`
private re-import. Move any stray helper still present into its topical submodule.

- [ ] **Step 2: Write the facade module doc**

Replace the old file header doc with a concise facade `//!`:

```rust
//! Update delivery, ownership, reconnect recovery, and update-lifecycle message
//! handlers, split across topical submodules. This facade wires the submodules
//! and re-exports the surface consumed by sibling handler modules.
//!
//! - [`ownership`] — host-link visibility checks
//! - [`replay`] — reconnect recovery + pending replay
//! - [`started`] / [`output`] / [`result`] — per-message handlers
//! - [`batch`] — batch result handling + progress
//! - [`dispatch`] — successor dispatch (batch/queue)
//! - [`audit`] / [`lookups`] / [`finalize`] / [`stdin`] — cross-cutting helpers
```

Add a one-line `//!` to any submodule that lacks one (Tasks 2–10 added them; verify).

- [ ] **Step 3: Full quality gate**

Run each; all must PASS:

```bash
cargo fmt --all
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite
cargo check -p uptrakit-web-api --all-features
cargo clippy -p uptrakit-web-api --all-targets --no-default-features --features db-sqlite
cargo clippy -p uptrakit-web-api --all-targets --all-features
cargo test -p uptrakit-web-api --all-features
```

Expected: PASS, zero warnings (workspace denies warnings).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor(web-api): finalize updates facade module

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 2 — Stage 2: re-score and decide on Stage 3

### Task 13: CodeScene re-score + cohesion dry-run (decision gate)

**Files:** none (measurement only — produces a go/no-go for Task 14).

- [ ] **Step 1: Score every production submodule**

Use the CodeScene MCP `code_health_score` tool on each:

```text
updates/mod.rs, updates/ownership.rs, updates/lookups.rs, updates/finalize.rs,
updates/audit.rs, updates/replay.rs, updates/started.rs, updates/output.rs,
updates/result.rs, updates/dispatch.rs, updates/batch.rs, updates/stdin.rs
```

Record each score. **Floor: none red (≥ 4.0).** `updates/tests.rs` is exempt.

- [ ] **Step 2: Cohesion dry-run on the small/low-connectivity modules**

`lookups.rs` (2 unrelated fns) and `audit.rs` are the Low-Cohesion / duplication-flag risks. If
either is red:

- `lookups.rs` red → fold the two bodies into one `resolve_name<E: EntityTrait>(...)` generic (spec
  dedup #5 fallback), or accept yellow per the spec caveat (yellow is NOT a failure).
- `audit.rs` red → regroup (e.g. keep the `emit_*` family but verify the shared
  `resolve_target_display` call-edge raises LCOM4 connectivity).

Apply any regrouping as a small follow-up commit; re-score.

- [ ] **Step 3: Decide Stage 3**

- If `updates/result.rs` is **green (≥ 9.0)** → **skip Task 14** entirely (the split alone sufficed;
  the riskiest change is avoided).
- If `updates/result.rs` is **yellow or red** → proceed to Task 14.

Record the decision (and the `result.rs` score) in the commit message of whichever task follows.

---

### Task 14 (CONDITIONAL): Extract `handle_update_result` (cc=22)

> Run ONLY if Task 13 Step 3 says `result.rs` is not green. Skip otherwise.

**Files:**

- Modify: `.../handler/updates/result.rs`

**Interfaces:**

- Produces (private helpers in `result.rs`):
  - `async fn try_intercept_resumable(state, payload, record, agent_truncated, service_id) -> bool`
    — true = AwaitingRestart CAS won + audit emitted; caller returns early.
  - `async fn finalize_unowned_result(state, service_id, payload, record, final_status, final_output) -> bool`
    — true = fully handled (caller returns early); false = fell through (rows>0, side-effects still
    run).
  - `async fn emit_result_side_effects(state, service_id, payload, record, final_status, final_output, agent_truncated)`
    — trailing dispatch + completed-event + notification + audit.

- [ ] **Step 1: Extract `try_intercept_resumable`**

Move the resumable-interception block (originally lines 1450–1482) into a helper. It returns `true`
when the `transition_to_awaiting_restart` CAS wins (rows > 0): emit the `Partial`/`awaiting_restart`
audit and signal early return. On CAS-loss it logs and returns `false`. Caller:

```rust
    if matches!(final_status, UpdateFinalStatus::Completed)
        && payload.resumable == Some(true)
        && try_intercept_resumable(state, &payload, &record, agent_truncated, service_id).await
    {
        return ProcessorResponse::cont();
    }
```

Preserve the exact warning log + audit args.

- [ ] **Step 2: Extract `finalize_unowned_result`**

Move the `if updated == 0 { ... }` block (originally 1520–1601) into a helper returning `bool`:

- returns `true` (caller returns early) for: Completed-but-not-owned (Denied audit),
  `fail_pending_unowned_update` Ok(0) (Denied audit), Err (Failed audit).
- returns `false` (fall through) for: `fail_pending_unowned_update` Ok(rows>0) — the "agent
  pre-start failure" path that must still run post-finalization side-effects.

Caller keeps the `updated` binding in scope and gates on it:

```rust
    if updated == 0
        && finalize_unowned_result(state, service_id, &payload, &record, &final_status, &final_output).await
    {
        return ProcessorResponse::cont();
    }
    // updated > 0 (normal) OR fell-through unowned path: run side-effects.
    if updated > 0 {
        // existing finalized_record + finalize_post_update_best_effort(.., None) block, unchanged
    }
```

**Critical:** the outer `updated` (CAS row count) stays in the top-level fn — the helper neither
owns nor mutates it (spec note). The `if updated > 0 { finalize_post_update_best_effort }` block
remains gated by the original variable.

- [ ] **Step 3: Extract `emit_result_side_effects`**

Move the trailing tail (originally 1657–1730: push-software-states, batch/queue dispatch,
completed-event, notification, finalized-audit) into a helper. Caller calls it once after the
`updated` gate. Preserve the `svc_tenant_id` resolution and the `output_truncated` update
(1615–1623) ordering exactly.

- [ ] **Step 4: Build both feature sets + clippy + test**

```bash
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite
cargo check -p uptrakit-web-api --all-features
cargo clippy -p uptrakit-web-api --all-targets --all-features
cargo test -p uptrakit-web-api --all-features updates
```

Expected: PASS — `handle_update_result_emits_update_finalized_audit_event` and the resumable/unowned
tests guard every branch.

- [ ] **Step 5: Re-score `result.rs`**

CodeScene `code_health_score` on `result.rs`. Expected: improved; floor is none-red. If still
yellow, that is acceptable (max extraction spent — see spec).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(web-api): decompose handle_update_result into intercept/unowned/side-effect helpers

Behavior-preserving extraction (move not rewrite); outer CAS row-count binding
retained to gate post-finalization side-effects.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 3 — Docs + final verification

### Task 15: Update `AGENTS.md` handler-module table

**Files:**

- Modify: `AGENTS.md` (repo root, line ~1271)

**Interfaces:** doc-only. Spec deliverable: the handler-module table must list the new submodules
instead of the single `updates.rs` row.

- [ ] **Step 1: Replace the `updates.rs` row**

Find the row (root `AGENTS.md`, currently line 1271):

```text
| `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`               | Update lifecycle handlers (return `ProcessorResponse`)                                                                                                                                                                              |
```

Replace it with rows for the facade + submodules (mirror the column widths of the surrounding
`session_*` rows at lines 1263–1264), e.g.:

```text
| `.../service_ws/handler/updates/mod.rs`      | Facade: module wiring + re-exports + shared consts/types (`ReconnectSuccessorDispatchMode`, `ReplayPreparationNotifier`) |
| `.../service_ws/handler/updates/{ownership,lookups,finalize,audit}.rs` | Cross-cutting helpers: host-link validation, name lookups, post-update finalize, lifecycle audit emission |
| `.../service_ws/handler/updates/{replay,started,output,result,batch,dispatch,stdin}.rs` | Per-message handlers + reconnect replay + successor dispatch (return `ProcessorResponse`) |
```

(Keep paths consistent with the table's existing full-path style; the `...` is illustrative — write
the full `crates/ui/web-api/src/...` paths.)

- [ ] **Step 2: Lint the doc**

Run: `npx markdownlint --config .markdownlint.json AGENTS.md` Expected: clean (MD013 exempts tables;
if a non-table line trips, fix it).

- [ ] **Step 3: Commit**

```bash
git add AGENTS.md
git commit -m "docs: list updates/ handler submodules in AGENTS.md

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 16: Final full quality-gate run

**Files:** none (verification only).

- [ ] **Step 1: Workspace-wide gates**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
```

Expected: all PASS, zero warnings.

- [ ] **Step 2: Docker system integration suite (required)**

The updates handler is service-lifecycle code — the snapshot binding rule mandates the Docker suite,
no carve-out for refactors.

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored
```

Expected: PASS.

- [ ] **Step 3: Final CodeScene confirmation**

`code_health_score` on every `updates/*.rs` production file. Confirm: **none red**; ideally facade +
most submodules green. `updates/tests.rs` exempt. Record the final scores (before: 3.3 monolith →
after: per-file). If any production file is still red, the refactor is NOT done — regroup that file
(Task 13 Step 2 method) before declaring complete.

- [ ] **Step 4: No commit needed** (verification only). If Steps fixed anything, commit with
      `refactor(web-api): …` per the touched file.

---

## Self-Review

**Spec coverage:**

- Split into `updates/` subdir + facade → Tasks 1–12. ✓
- Facade re-export surface (17 items) → re-exports across Tasks 2–10; verified in Task 12. ✓
- `super::` import convention (not absolute `crate::`) → headers in every move task +
  `use super::{shared_types, audit_service};` in Task 2. ✓
- Move tests to `updates/tests.rs` → Task 11. ✓
- Dedup #1 finalize merge → Task 3. Dedup #2 installed-version → Tasks 8/9. Dedup #3 audit scaffold
  → Task 4. Dedup #4 dispatch wrapper → Task 7. Dedup #5 lookups non-merge → Task 2 (kept separate).
  ✓
- Low-risk extractions: `prepare_pending_replay_messages` per-record → Task 5;
  `BatchUpdateAuditSummary` predicates → Task 9. ✓
- `#![expect]` migration to `result.rs` → Task 8 Steps 1–2. ✓
- Sequencing (Stage 1 → re-score → conditional Stage 3) → Phases 1/2; Tasks 13/14. ✓
- `handle_stdin_attention` `pub(crate)`→`pub(super)` tightening → Task 10 Step 1. ✓
- `final_status_str` home = `result.rs`, audit imports it → Task 8 Step 4. ✓
- `updated`-binding-stays-in-scope for Stage 3 → Task 14 Step 2. ✓
- Docs: `AGENTS.md` → Task 15. No README/CONTEXT/ADR (spec: no externally observable change) —
  carried as justification, no task. ✓
- Verification incl. `cargo deny` + Docker suite → Task 16. ✓
- Score floor "none red", tests.rs exempt → Tasks 13/16. ✓

**Placeholder scan:** no TBD/TODO; dedup/extraction steps carry real code; move steps carry exact
function lists + line ranges + import/re-export code. ✓

**Type consistency:** `finalize_post_update_best_effort(state, record, Option<Duration>)` used
identically in Tasks 3/5/8/9. `set_installed_version(state, Condition, &str)` defined Task 8,
consumed Task 9. `PendingRecordOutcome`/`prepare_single_pending_record` self-contained in Task 5.
`try_intercept_resumable`/`finalize_unowned_result`/`emit_result_side_effects` defined + called
within Task 14. ✓

**New dependencies:** none.
