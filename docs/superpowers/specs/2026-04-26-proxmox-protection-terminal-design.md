# Proxmox Update Protection — Terminal Integration Design

**Date:** 2026-04-26
**Status:** Approved

## Problem

When a Proxmox update is triggered, the orchestrator transitions the record to `InProgress` and
starts protection (snapshot or backup) before dispatching to the agent. The agent sets
`execution_owner_service_id` only after it claims the update via `handle_update_started`.

The `interactive_ws` endpoint hard-requires `execution_owner_service_id` to be non-null before
accepting the WebSocket upgrade. During protection this field is always `NULL`
(`set_inprogress_for_orchestrator` explicitly writes `NULL` as the orchestrator ownership
sentinel). Any WS connection attempt during this window gets a 409 "No agent has claimed this
update yet" before the upgrade handshake, breaking the terminal.

On the software detail page, `openLiveModal` is called immediately after the trigger REST
response — at which point the record may still be `Pending` — producing a second failure mode:
409 "Update is not in progress".

After either failure, the broadcaster channel is never cleaned up if protection subsequently
fails (`fail_before_agent_dispatch` marks the DB row Failed but does not call
`send_completed`), leaving a leaked channel and any future subscriber waiting forever.

## Root Cause Summary

| Failure | Cause |
| --- | --- |
| WS 409 during protection | `interactive_ws` rejects `execution_owner_service_id = NULL` |
| WS 409 on immediate open | Frontend opens WS before orchestrator transitions `Pending → InProgress` |
| Channel leak on protection failure | `fail_before_agent_dispatch` never calls `send_completed` |
| History page can't attach terminal | Status is InProgress but `execution_owner_service_id` still NULL |

## Design — Option A (chosen)

Single unbroken WS session per update. Server drives capability upgrade from read-only
(protection phase) to interactive (agent phase) via a new `AgentClaimed` broadcast event.

### Backend

#### 1. `BroadcastEvent::AgentClaimed` (new variant)

```rust
// crates/ui/web-api/src/update_output_broadcaster.rs
pub enum BroadcastEvent {
    Line { id, text, stream, timestamp, seq },
    Completed { status, error },
    StdinAttention { hint },
    AgentClaimed { service_id: Uuid },   // NEW
}
```

Does not remove the channel. Signals that stdin forwarding is now possible.

#### 2. `UpdateOutputBroadcaster::send_agent_claimed`

New method on `UpdateOutputBroadcaster`. Sends `AgentClaimed { service_id }` to all
subscribers without touching the channel entry.

```rust
pub async fn send_agent_claimed(&self, update_history_id: Uuid, service_id: Uuid)
```

#### 3. `handle_update_started` — emit `AgentClaimed`

After `get_or_create_channel` for both `Claimed` and `Replay` outcomes, call:

```rust
state.broadcast.update_output_broadcaster
    .send_agent_claimed(payload.update_history_id, service_id)
    .await;
```

`service_id` is the outer function parameter (the calling agent's service ID) — it is
present in both outcomes. `ClaimExecutionOutcome::Replay(info)` carries `ClaimExecutionInfo`
(batch/host/tenant metadata) but not `service_id`; use the function-level `service_id` param.

This notifies any WS subscriber connected during the protection phase that stdin forwarding
is now possible.

#### 4. `run_protection_and_dispatch` — close channel on failure

Three failure paths leave the channel open:

- `prepare_pre_update_protection` returns `Ok(PreUpdateProtectionOutcome::Failed)` — record is
  already marked Failed by the inner `fail_before_agent_dispatch` call inside
  `prepare_pre_update_protection`; orchestrator must call
  `broadcaster.send_completed(update_history_id, "failed".to_string(), None)` afterward.

- `prepare_pre_update_protection` returns `Err(e)` — orchestrator calls `fail_before_agent_dispatch`
  (which may itself fail); `send_completed` must fire unconditionally regardless of whether
  `fail_before_agent_dispatch` succeeds.

- `dispatch_update_to_agent` returns `Err(e)` — orchestrator calls `fail_before_agent_dispatch`
  then returns, but the broadcaster channel created in step 3 is never closed.
  Must also call `send_completed` here before returning.

All three paths must call `send_completed` so that any connected WS subscriber exits cleanly
and the channel is removed from the registry. Call `send_completed` unconditionally — do not
gate it on `fail_before_agent_dispatch` succeeding.

#### 5. `interactive_ws` — allow connection when `execution_owner_service_id` is NULL

**Step 6 (current — hard reject):** change to soft-allow. If the field is `None`, proceed
with `service_id: Option<Uuid> = None`. Do not reject.

**Step 7 (agent connectivity check):** skip when `service_id` is `None`. No agent to check
yet.

**`handle_interactive_session` signature:** accept `service_id: Option<Uuid>` instead of
`Uuid`. Store as a plain `let mut local_service_id: Option<Uuid>` — the session is a single
`async fn` with no cross-task sharing, so no mutex is needed.

**WS loop — new `AgentClaimed` arm:**

```rust
Ok(BroadcastEvent::AgentClaimed { service_id: claimed_id }) => {
    if local_service_id.is_none() {
        if state.service_connections.is_connected(&claimed_id).await {
            local_service_id = Some(claimed_id);
            tracing::debug!(%claimed_id, "agent claimed update — stdin forwarding enabled");
        }
        // If not connected: keep None. When the agent reconnects it will
        // re-send UpdateStarted → handle_update_started → Replay outcome →
        // send_agent_claimed fires again — WS session picks it up on the
        // next AgentClaimed event.
    }
}
```

**Stdin forwarding when `service_id` is `None`:** silently skip. No error to client.
The user may have the xterm focused during protection; ignoring is correct.

**Audit logging:** the step-8 audit call (session established, before WS upgrade) passes
`Some(service_id)` today. After the change, `service_id` is `Option<Uuid>`; pass it directly
(not wrapped in `Some`). All inner audit helpers already accept `Option<Uuid>`.

### Frontend

#### 1. `sse.ts` — extend `AdminEventType`

Add `'update_protection_started'` to the `AdminEventType` union literal type. The backend
already emits this event (step 5 of `run_protection_and_dispatch` in `update_orchestrator.rs`
calls `event_broadcaster.send(UpdateProtectionStarted { ... })`; `AdminEvent::event_name()`
returns `"update_protection_started"`). No backend change needed for emission.

#### 2. `/software/[id]/+page.svelte` — deferred `openLiveModal`

**Current behaviour:** `openLiveModal(res.update_history_id, hostName)` called immediately
after the trigger REST response while status may be `Pending`.

**New behaviour:** after triggering, store the pending `update_history_id` locally. Open the
modal only when `update_protection_started` (or `update_started` for non-Proxmox updates)
SSE fires for that ID. This ensures the WS connects only after the orchestrator has
transitioned the record to `InProgress` and the broadcast channel exists.

The SSE handler adds:

```typescript
subscribeToEvent('update_protection_started', (data) => {
    if (data.update_history_id === pendingLiveHistoryId) {
        openLiveModal(pendingLiveHistoryId, pendingLiveHostName);
        pendingLiveHistoryId = null;
    }
})
```

`update_started` SSE handler on this page currently calls `loadItem(true)` — it does not
open the modal. New code must be added: if `data.update_history_id === pendingLiveHistoryId`,
call `openLiveModal` and clear `pendingLiveHistoryId`. This covers the non-Proxmox path where
protection is skipped (`DoNothing`) and the agent starts before `update_protection_started`
resolves to a WS open.

#### 3. `/history/+page.svelte` — react to `update_protection_started`

Add SSE handler for `update_protection_started` that updates the matching list item's status
to `'in_progress'`, mirroring the existing `update_started` handler. This makes the
"Attach terminal" button appear as soon as protection begins, before the agent claims.

## Data Flow

```text
Orchestrator: Pending→InProgress, create_channel(id)
Orchestrator: emit UpdateProtectionStarted SSE → frontend opens WS

[WS connects — protection phase]
interactive_ws: status=InProgress ✓, execution_owner_service_id=None → allowed (read-only)
WS session: replay DB lines, subscribe to broadcaster
Protection output → broadcaster → WS → client

[Agent dispatched]
handle_update_started → send_agent_claimed(id, service_id)
WS session: receives AgentClaimed → stores service_id → stdin enabled

[Agent phase]
Agent output → broadcaster → WS → client  (same connection, no gap)
Agent done → send_completed → WS receives Completed → closes

[Protection failure]
fail_before_agent_dispatch → broadcaster.send_completed("failed")
WS subscriber receives Completed → closes cleanly
```

## Error Handling

| Scenario | Behaviour |
| --- | --- |
| `AgentClaimed` arrives but agent already disconnected | Keep `service_id = None`; reconnect recovery re-sends `AgentClaimed` when agent reconnects |
| WS connects after update already completed | No channel in broadcaster → "No active output stream" error sent, WS closes (existing path, unchanged) |
| Client sends stdin during protection phase | Silently ignored; no error to client |
| Protection fails | `send_completed("failed")` closes subscriber and removes channel |
| Dispatch to agent fails | `fail_before_agent_dispatch` then `send_completed("failed")` — same closure path |

## Testing

- Unit: `send_agent_claimed` delivers event to subscriber without removing channel
- Unit: `send_completed` after `fail_before_agent_dispatch` removes channel
- Integration: WS connects during protection (service_id=None), receives output lines,
  receives `AgentClaimed`, stdin forwarding activates
- Integration: protection failure → `send_completed` → subscriber exits with Completed event
- Integration: dispatch to agent fails → `send_completed` closes channel and subscriber
- Frontend: `update_protection_started` SSE updates history list item to `in_progress`
- Frontend: software detail page defers `openLiveModal` until `update_protection_started` fires

## Files Changed

| File | Change |
| --- | --- |
| `crates/ui/web-api/src/update_output_broadcaster.rs` | Add `AgentClaimed` variant + `send_agent_claimed` |
| `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` | Call `send_agent_claimed` after channel create/get in `handle_update_started` |
| `crates/ui/web-api/src/update_orchestrator.rs` | Call `send_completed` on all three failure paths (protection fail, protection error, dispatch error) |
| `crates/ui/web-api/src/routes/interactive_ws.rs` | Allow `service_id = None`, handle `AgentClaimed`, skip stdin when None |
| `frontend/src/lib/sse.ts` | Add `'update_protection_started'` to `AdminEventType` |
| `frontend/src/routes/software/[id]/+page.svelte` | Defer `openLiveModal` until protection-started/update-started SSE |
| `frontend/src/routes/history/+page.svelte` | Handle `update_protection_started` SSE → status update |
