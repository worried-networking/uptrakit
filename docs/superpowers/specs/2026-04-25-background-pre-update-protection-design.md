# Background Pre-Update Protection

**Date:** 2026-04-25  
**Status:** Approved

## Problem

`trigger_update_for_host()` calls `prepare_pre_update_protection()` synchronously before
returning. For Proxmox-backed hosts this blocks the HTTP handler for up to 120 seconds while
a snapshot or backup runs. The user waits at the trigger button with no feedback.

## Goal

- HTTP trigger returns immediately after the `update_history` record is created.
- Protection (snapshot/backup) runs in a background `tokio::spawn` task.
- Record transitions `Pending → InProgress` when protection starts, not when the agent confirms.
- Every protection output line is persisted to `update_output_line` and broadcast live.
- Agent dispatch happens only after protection succeeds.
- Queued-update promotion goes through the same orchestrator path.
- Agent offline at trigger time: record stays `Pending`, orchestrator spawns on reconnect.
- Crash recovery: all orchestrator-owned `InProgress` records marked `Failed` on agent reconnect.

---

## `update_history` State Machine

| Phase | `status` | `execution_owner_service_id` | `pre_update_protection_status` |
| --- | --- | --- | --- |
| Created | `Pending` | `NULL` | `NULL` |
| Orchestrator running protection | `InProgress` | **`NULL`** | `"in_progress"` |
| Protection done, dispatch in flight | `InProgress` | **`NULL`** | `"protected"` / `"skipped"` |
| Agent confirmed start | `InProgress` | `<service_id>` | `"protected"` / `"skipped"` |
| Terminal | `Completed` / `Failed` | `<service_id>` | final value |

**`execution_owner_service_id = NULL` + `status = InProgress`** is the orchestrator-owned
sentinel. No new columns required.

---

## New Types (`web-api-queries/queries/update_triggers.rs`)

```rust
/// All data the orchestrator needs to run protection and dispatch.
pub struct PendingProtectionWork {
    pub target: ValidatedUpdateTarget,   // ValidatedUpdateTarget gains #[derive(Clone)]
    pub update_history_id: Uuid,
    pub to_version: String,
    pub release_info: Option<ReleaseInfo>,
    pub interactive: bool,               // fully resolved (incl. prefer_interactive)
}

pub struct TriggerUpdateResult {
    pub update_history_id: Uuid,
    pub initial_status: update_history::UpdateStatus,
    /// Present when initial_status == Pending; caller must spawn protection+dispatch.
    /// None when Queued (host busy).
    pub pending_protection_work: Option<Box<PendingProtectionWork>>,
}
```

---

## `trigger_update_for_host` Refactor

Signature change — `DispatchContext` removed:

```rust
pub async fn trigger_update_for_host(
    db: &DatabaseConnection,
    params: TriggerUpdateParams<'_>,
) -> Result<TriggerUpdateResult>
```

For the `Pending` case: resolve interactive flag, build `PendingProtectionWork`, return it.
No inline protection or dispatch. `DispatchContext` struct can be removed from the public API.

`actions/software_items.rs::trigger_update` drops its `protection` parameter — the
orchestrator fetches protection from `state.controller_update_protection()` directly.

---

## Orchestrator (`web-api/src/update_orchestrator.rs`)

Single public entry point:

```rust
pub fn spawn_protection_and_dispatch(
    state: Arc<AppState>,
    work: PendingProtectionWork,
)
```

Spawns `run_protection_and_dispatch(state, work)` via `tokio::spawn`.

### `run_protection_and_dispatch` steps

1. **Check agent connected** via `state.service_connections.is_connected(&work.target.agent.id)`.  
   If not connected: return early. Record stays `Pending`; reconnect recovery spawns the
   orchestrator when the agent comes back.

2. **Create broadcast channel**: `broadcaster.create_channel(update_history_id)`.

3. **Transition to InProgress**: `set_inprogress_for_orchestrator(db, id)` sets
   `status = InProgress`, `pre_update_protection_status = "in_progress"`,
   `execution_owner_service_id = NULL`. If this CAS returns 0 rows affected (record already
   gone or raced), log and return.

4. **Push MQTT states** so connected agents see the host as in-progress.

5. **Emit `AdminEvent::UpdateProtectionStarted`** so the frontend transitions to the
   In Progress state immediately.

6. **Create output channel**: `mpsc::unbounded_channel::<Vec<u8>>()`.

7. **Spawn `forward_protection_output`** task: reads from the mpsc receiver, inserts each
   line into `update_output_line` (no ownership check), and broadcasts via
   `broadcaster.send_line`. Sequence numbers tracked by the forwarder with an atomic counter.

8. **Run protection**: `prepare_pre_update_protection(db, protection, &target, id, Some(tx))`.
   The plugin uses `ctx.output_tx` to stream lines during its poll loop.
   On return the plugin has already written the final `pre_update_protection_status`.

9. **Match outcome**:
   - `Failed`: push MQTT states, return. Record already set to `Failed` by
     `fail_before_agent_dispatch`.
   - `Proceed`: call `dispatch_update_to_agent(notifier, &target, params)`, push MQTT states.
     `AdminEvent::UpdateStarted` fires later from `handle_update_started` when the agent
     confirms. If `dispatch_update_to_agent` itself errors, call `fail_before_agent_dispatch`
     and push states.

---

## `ControllerProtectionContext` Extension

New field in `infrastructure/core/src/roles.rs`:

```rust
pub output_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
```

`new()` signature unchanged; defaults to `None`. Builder:

```rust
pub fn with_output_tx(mut self, tx: UnboundedSender<Vec<u8>>) -> Self {
    self.output_tx = Some(tx);
    self
}
```

`prepare_pre_update_protection` gains one parameter:

```rust
pub async fn prepare_pre_update_protection(
    db: &DatabaseConnection,
    protection: Option<Arc<dyn ControllerUpdateProtection>>,
    target: &ValidatedUpdateTarget,
    update_history_id: Uuid,
    output_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
) -> Result<PreUpdateProtectionOutcome>
```

Existing callers (batch path, recovery) pass `None`. Orchestrator passes `Some(tx)`.

Proxmox `update_protection.rs` uses `ctx.output_tx.as_ref()` in the snapshot/backup poll
loop to stream status lines as `Vec<u8>`.

`#[non_exhaustive]` on `ControllerProtectionContext` ensures external plugin implementations
using `..` spread patterns are not broken by the new field.

---

## WS Handler Changes

### `claim_or_replay_update_start_db` — new case

Added before the `Rejected` fallthrough. Handles the agent confirming an update whose
record was already set to `InProgress` by the orchestrator:

```rust
if record.status == InProgress && record.execution_owner_service_id.is_none() {
    let txn = db.begin().await.context_to()?;
    let claimed = UpdateHistory::update_many()
        .filter(Column::Id.eq(record.id))
        .filter(Column::Status.eq(InProgress))
        .filter(Column::ExecutionOwnerServiceId.is_null()) // CAS guard
        .col_expr(ExecutionOwnerServiceId, Expr::value(Some(service_id)))
        .col_expr(ExecutionOwnerInstanceId, Expr::value(runtime_instance_id))
        .col_expr(Interactive, Expr::value(interactive))
        .exec(&txn).await.context_to()?;

    if claimed.rows_affected == 0 {
        txn.rollback().await.context_to()?;
        return Ok(ClaimExecutionOutcome::Rejected);
    }

    txn.commit().await.context_to()?;
    return Ok(ClaimExecutionOutcome::Claimed(claim_execution_info(&record)));
}
```

**No `UpdateOutputLine::delete_many()`** — protection output lines are kept.

### `handle_update_started` — Claimed arm

```rust
ClaimExecutionOutcome::Claimed(_) => {
    // get_or_create_channel: orchestrator may have pre-created the channel
    state.broadcast.update_output_broadcaster
        .get_or_create_channel(payload.update_history_id).await;
    broadcast_update_started_events(state, service_id, payload, &info).await;
}
```

`create_channel` → `get_or_create_channel` preserves existing subscriber connections from
the protection-output phase.

### Reconnect recovery

After existing owned-InProgress recovery, add:

```sql
mark_orchestrator_inprogress_as_failed_on_reconnect(db, host_ids):
  UPDATE update_history
  SET status = Failed,
      completed_at = now(),
      output = "Protection interrupted: controller restarted",
      pre_update_protection_status = "failed"
  WHERE status = InProgress
    AND execution_owner_service_id IS NULL
    AND host_id IN (linked_host_ids)
```

Covers both `"in_progress"` (protection was mid-run) and `"protected"` (dispatch was lost)
cases. User re-triggers; Proxmox protection re-runs.

### Pending-on-reconnect

`prepare_pending_replay_messages` currently builds `ExecuteUpdatePayload` directly for
Pending records. In the new design, Pending records with `pre_update_protection_status = NULL`
have not had protection run. Instead of building a raw replay payload, call
`spawn_protection_and_dispatch(Arc::clone(state), work)` where `work` is reconstructed from
the Pending record's data. Agent is now connected, so the orchestrator proceeds normally.

---

## Call Sites in `web-api`

### REST trigger (`routes/software_items/mod.rs`)

```rust
let result = item_actions::trigger_update(&tenant_db, &ctx, params).await?;
if let Some(work) = result.pending_protection_work {
    update_orchestrator::spawn_protection_and_dispatch(Arc::clone(&state), *work);
}
```

### WS trigger (`handler/update_tracking.rs`)

Same pattern with `Arc::clone(state)`.

### Queued promotion (`handler/updates.rs`)

`dispatch_next_queued_update_with_notifier` gains `state: Arc<AppState>`. On promotion,
calls `spawn_protection_and_dispatch` instead of inline dispatch.

---

## New `AdminEvent`

```rust
AdminEvent::UpdateProtectionStarted {
    update_history_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
}
```

Emitted by orchestrator at step 5 (after InProgress transition). Frontend shows In Progress
state during protection phase. `AdminEvent::UpdateStarted` remains exclusive to
`handle_update_started` — fires when agent confirms.

---

## Files Changed

| File | Change |
| --- | --- |
| `infrastructure/core/src/roles.rs` | `output_tx` field + `with_output_tx()` on `ControllerProtectionContext` |
| `infrastructure/proxmox/src/update_protection.rs` | Use `ctx.output_tx` in poll loop |
| `web-api-queries/src/queries/update_dispatch.rs` | `Clone` on `ValidatedUpdateTarget`; `set_inprogress_for_orchestrator`; `output_tx` param on `prepare_pre_update_protection`; `insert_protection_output_line`; `fail_before_agent_dispatch` as `pub(crate)` |
| `web-api-queries/src/queries/update_triggers.rs` | Remove `DispatchContext` param; add `PendingProtectionWork`; update `TriggerUpdateResult`; Pending case returns work bundle |
| `web-api-queries/src/queries/update_batches/dispatch.rs` | Orchestrator-InProgress CAS case in `claim_or_replay_update_start_db`; `mark_orchestrator_inprogress_as_failed_on_reconnect` |
| `web-api/src/update_orchestrator.rs` | **NEW**: `spawn_protection_and_dispatch`, `run_protection_and_dispatch`, `forward_protection_output` |
| `web-api/src/lib.rs` | `pub(crate) mod update_orchestrator` |
| `web-api/src/routes/service_ws/handler/updates.rs` | Claimed arm → `get_or_create_channel`; reconnect recovery for orchestrator InProgress; Pending-on-reconnect → spawn orchestrator; `dispatch_next_queued_update_with_notifier` gains `state: Arc<AppState>` + calls `spawn_protection_and_dispatch` |
| `web-api/src/routes/software_items/mod.rs` | Spawn orchestrator when `pending_protection_work.is_some()` |
| `web-api/src/routes/service_ws/handler/update_tracking.rs` | Spawn orchestrator when `pending_protection_work.is_some()` |
| `web-api/src/actions/software_items.rs` | Drop `protection` param from `trigger_update` |
| `web-api-types/src/events.rs` | Add `AdminEvent::UpdateProtectionStarted` |

---

## Out of Scope

- Re-dispatch on crash-after-protection (Option B recovery) — deferred.
- Persisting protection output to `update_history.output` concatenated column — protection
  lines go to `update_output_line` only; the concatenated column is populated by agent output
  as today.
- Batch update protection — batch items remain inline (batch items are always Queued first;
  their promotion goes through the orchestrator via `dispatch_next_queued_update_with_notifier`).
