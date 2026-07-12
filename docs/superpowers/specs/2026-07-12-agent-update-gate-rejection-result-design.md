# Agent Update-Gate Rejection Result (single `ExecuteUpdate`) — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `crates/core/agent-runtime/` `ExecuteUpdate` arm + its tests. No controller change, no wire-protocol
change, no ADR, no deps. (The `ExecuteBatchUpdate` half of audit L859 is **split out** — see § Out of scope.)

## Problem

Audit `audit-2026-07-11` L859 (MEDIUM · stability · core-agent · verified): when an agent's update gate rejects
an `ExecuteUpdate`, the message is **silently dropped** after only an audit event — no `UpdateResult(Failed)` is
sent back. The controller created the `update_history` record **before** dispatch (status `Pending`), so it sits
`in_progress`/pending until the update reaper fires (`REAPER_INTERVAL` 60s + `REAPER_GRACE` 300s). The user sees
a stuck update for 5+ minutes with **no failure reason**. Three gate reasons hit this: machine-id mismatch,
update freeze active, and the 5-second post-update cooldown.

**Why single-path only (batch is split out).** The finding also names the `ExecuteBatchUpdate` arm, but review
established two things that make the batch case a distinct, riskier change best specced separately:

- The controller **does not dispatch wire `ExecuteBatchUpdate` to WS agents** — grep of `crates/ui` +
  `controller-runtime` finds no sender. WS batch items are dispatched as **individual `ExecuteUpdate`** messages
  (`update_batches/mod.rs:288`, `dispatch_update_to_agent`). So the WS-agent `ExecuteBatchUpdate` arm
  (`lib.rs:317`) is a currently-cold defensive path; the live `BatchUpdateResult` senders are the **SSH runtime**
  (`agent-ssh-runtime/src/client.rs:909/933/962`) and `agent-core::run_execute_batch_update`.
- The controller's **batch** result handler (`process_single_batch_result`, `updates/batch.rs:214`) has **no**
  `Pending`/`Queued` fallthrough (unlike the single path — below), so a batch `Failed` on a non-`InProgress` row
  no-ops to `Stale`. Adding that fallthrough means firing `fail_pending_unowned_update`, which runs **post-update
  hooks + protection-restore** (`dispatch.rs:1050/1071`) — semantically questionable for a never-ran item, and a
  decision that deserves its own spec rather than riding on the agent fix.

## Verified current reality (byte-checked, 2026-07-12)

- `crates/core/agent-runtime/src/lib.rs:293-307` — **ExecuteUpdate** arm:

  ```rust
  ControllerMessage::ExecuteUpdate(payload) => {
      let allowed = self.machine_id_matches("ExecuteUpdate", &payload.host_machine_id)
          && !self.execution_frozen("ExecuteUpdate").await
          && self.accept_update_execution("ExecuteUpdate");
      if allowed {
          uptrakit_agent_core::handle_execute_update(*payload, /* runtime */, &mut self.in_flight_update, transport, &self.ctx).await;
      }
  }
  ```

  On `!allowed` the whole block is skipped — nothing is sent.
- Gate helpers (`lib.rs:488-536`), invoked left-to-right by the short-circuit `&&`:
  - `machine_id_matches` — sync; on mismatch emits `audit_emitter.machine_id_validate(...)`, returns `false`.
  - `execution_frozen` — async; when frozen emits `audit_emitter.update_gate(msg, "freeze", …)`, returns `true`.
  - `accept_update_execution` — sync; on reject emits `update_gate(msg, "cooldown", …)`; **on accept it has a
    side effect** — sets `self.last_update_accepted = Some(Instant::now())` (`std::time::Instant`). The
    short-circuit means this runs (and the cooldown timer resets) **only when machine + freeze both pass**.
- `ExecuteUpdatePayload` has `update_history_id: Uuid`, `host_machine_id: String`. `UpdateResultPayload`
  (`wire/payloads.rs:523`): `update_history_id, status: UpdateFinalStatus, from_version: Option, to_version:
  Option, output: String, error: Option<String>, resumable: Option<bool>`. Wire message
  `ServiceMessage::UpdateResult(UpdateResultPayload)` (`wire/messages.rs:47`). `UpdateFinalStatus::Failed`
  (`wire/shared_types.rs:26`) is the terminal-failed variant the normal result path already uses.
- `transport: &mut dyn ServiceTransport` is in scope in the arm; the trait exposes `transport_send_best_effort(msg)`
  (`wire/transport.rs:71`), used elsewhere in this file (`drain_audit_events`, `:540`).

**Controller closes the record on receiving the reply — the single path already has the fallthrough:**
`handle_update_result` (`updates/result.rs:223`) tries `finalize_update_result_if_owned` (`:298`, owner +
`InProgress` guard) and, on 0 rows, **falls through** to `fail_pending_unowned_update` (`:342`), which closes a
`Status == Pending` + null-owner row to `Failed` (`update_batches/dispatch.rs:981`; filters `Status.eq(Pending)`
and owner-null at `:1013-1017`, id-scoped on `Column::Id.eq(update_history_id)`). So the agent reply below
genuinely closes the record with **no controller change**. The reaper stays as the crash/disconnect backstop.

## Approach (chosen — root-cause once, YAGNI)

The rejection reply is fully constructible from the request payload alone — no plugin/runtime execution. Carry
the **rejection reason** out of the gate check (instead of a bool), then send the existing terminal-`Failed`
message best-effort.

### 1. Gate-check returns the reason

```rust
async fn check_update_gates(
    &mut self,
    msg_name: &str,
    host_machine_id: &str,
) -> Result<(), UpdateGateRejection> {
    if !self.machine_id_matches(msg_name, host_machine_id) {
        return Err(UpdateGateRejection::MachineIdMismatch);
    }
    if self.execution_frozen(msg_name).await {
        return Err(UpdateGateRejection::Frozen);
    }
    if !self.accept_update_execution(msg_name) {
        return Err(UpdateGateRejection::Cooldown);
    }
    Ok(())
}
```

Same order, same short-circuit. `accept_update_execution` — and its `last_update_accepted` reset — is reached
**only** after machine + freeze pass, exactly as today. Audit emits stay **inside** the helpers; the new enum
only carries the reason to the caller. **No double-emit.**

### 2. Internal rejection enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateGateRejection { MachineIdMismatch, Frozen, Cooldown }

impl UpdateGateRejection {
    fn as_str(self) -> &'static str {
        match self {
            Self::MachineIdMismatch => "update target machine-id mismatch on agent",
            Self::Frozen => "update frozen on agent",
            Self::Cooldown => "update rejected: agent cooldown active",
        }
    }
}

impl std::fmt::Display for UpdateGateRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
```

**Internal type** — never serialized as an enum over the wire (only its `as_str()` string travels, inside the
existing `error: Option<String>` field). So: no `#[non_exhaustive]`, no `Other(String)`, exhaustively matched.
The `as_str(self) -> &'static str` + `impl Display` shape and the `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`
follow the repo's documented internal-discriminator convention (`docs/development/coding-standards.md` § "Typed
Enum Parameters for Internal Write APIs", lines 436-487) and match the live `ActorType`/`BatchType`/
`DisconnectReason` enums. `Copy` is safe (no payload); `PartialEq`/`Eq` are needed for the test `assert_eq!`s.

### 3. Send the terminal-Failed reply on rejection

```rust
match self.check_update_gates("ExecuteUpdate", &payload.host_machine_id).await {
    Ok(()) => { uptrakit_agent_core::handle_execute_update(*payload, …, transport, &self.ctx).await; }
    Err(reason) => {
        transport.transport_send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
            update_history_id: payload.update_history_id,
            status: UpdateFinalStatus::Failed,
            output: String::new(),
            error: Some(reason.as_str().to_string()),
            from_version: None,
            to_version: None,
            resumable: None,
        })).await;
    }
}
```

**Best-effort** send (not `transport_send`): a rejection reply must never fail/tear down the connection. If the
send is dropped (connection already gone), the reaper is still the backstop. `Ok`/`Err` are mutually exclusive,
so there is no double-reply — the happy path's own `UpdateResult` is sent only under `Ok`.

## Machine-id mismatch: reply Failed (decision)

Reply `Failed` for **all three** reasons, including machine-id mismatch. A re-imaged host (new machine_id vs the
controller's stored old id) is exactly a case that otherwise hangs 5+ minutes; replying `Failed` surfaces it
immediately, and the `machine_id_validate` audit event still records the mismatch.

**Why this is safe (the boundary is controller-side, not agent-side).** One agent connection multiplexes many
hosts (`service_id` → many `host_id` via `service_host`), so the agent *can* be handed and reflect an
`update_history_id` for a different host. The safety guarantee is **not** agent-side id-reflection — it is the
controller's `validate_host_link_visibility` (`updates/ownership.rs:14`) plus the owner CAS on finalize, which
reject any record whose `host_id` is not linked to this connection's `service_id`. A misrouted/buggy dispatch
therefore cannot fail a record outside the connection's linked hosts. (Do not remove that controller-side guard
on the assumption the agent self-limits — it does not; the guard is the boundary.)

**Rejected alternative:** treat machine-id mismatch as a silent guard (reply only for freeze/cooldown). Rejected
— it re-introduces the silent 5-minute hang for the re-image case, the exact bug being fixed.

## Failed vs "refused" (conscious decisions)

- **Freeze/cooldown are semantically "refused," surfaced as `Failed`-with-reason.** ADR-0024 terminal states are
  only `Completed`/`Failed`/`Interrupted` — there is no "Refused" state, and adding one is a wire/ADR change this
  fix deliberately avoids. `Failed` + the reason string is the only terminal option; the audit `update_gate`
  event already records the policy nuance. Operators will see these refusals as `Failed`-with-reason — intended.
- **Cooldown is a deliberate behavior change** from *silent drop + 5-min reap* to *immediate visible `Failed`*.
  For a legitimate rapid back-to-back dispatch the second update now shows `Failed` immediately instead of
  hanging then being reaped `Interrupted`. Immediate + reasoned is strictly better operator feedback than a
  silent hang; accepted as the intended new behavior.

## Tests

Use `MockTransport` (`crates/shared/service-sdk/src/test_support.rs`), whose `send_log: Vec<ServiceMessage>`
records every send. This is the exact harness adjacent agent-runtime tests already use — e.g.
`machine_id_mismatch_forwards_service_audit_event` (`lib.rs:840+`) drives
`runtime.handle_controller_message(ControllerMessage::…, &mut transport)` against a `MockTransport` and inspects
`send_log`. (`RecordingForwarder` at `lib.rs:774+` is a `RuntimeAuditForwarder` capturing audit events only —
**not** a transport; wrong capture point here.) Cover:

- **ExecuteUpdate rejected** by (a) machine-id mismatch (wrong `host_machine_id`), (b) freeze-file present,
  (c) cooldown → assert exactly one `ServiceMessage::UpdateResult` captured, `status == Failed`, matching
  `update_history_id`, non-empty `error`.
- **Happy path** (all gates pass) → assert **no** rejection message and normal dispatch occurs.
- **Ordering/side-effect** → assert `last_update_accepted` is **not** reset when machine-id or freeze rejects
  (rejection before the cooldown check must not touch the timer).
- **Cooldown without time injection** → the cooldown uses `std::time::Instant` (`last.elapsed() <
  UPDATE_COOLDOWN`), which `tokio::time` pause does **not** control. Drive the reject by **back-to-back**
  invocation: the first accepts + sets the timer; the second, within the same instant, is `elapsed ≈ 0 <
  cooldown` → rejected. Deterministic, no `start_paused`, no clock injection. **Do not** add `start_paused`
  (repo rule: only for tests calling a tokio time API).

## Deliverables

- `crates/core/agent-runtime/src/lib.rs` — the `UpdateGateRejection` enum (+ `as_str`/`Display`/derives), the
  `check_update_gates` method, the rewritten `ExecuteUpdate` arm, and the tests above.

### Documentation deliverables

- If a doc describes the agent update-dispatch gate / reaper flow (Explore to confirm — candidates:
  `docs/api/services-operations.md`, `docs/architecture/update-history-entity.md`, agent docs), add one line:
  `ExecuteUpdate` gate rejections (machine-id / freeze / cooldown) now send an immediate terminal `Failed` result
  so the controller closes the `update_history` record without waiting for the reaper; the reaper remains the
  crash/disconnect backstop. If no such doc section exists, state "no doc impact" — agent-internal reply behavior,
  not a new API/config surface.
- **No `asyncapi.yaml` / wire-protocol change** — reuses the existing `UpdateResult` message and
  `UpdateFinalStatus::Failed`; no new payload, field, or message.
- **No ADR, dependency, OpenAPI, or frontend change** (agent↔controller wire only).

## Verification

- `cargo test -p uptrakit-agent-runtime` green (new cases).
- `cargo clippy --all-targets --all-features` clean — no `#[allow]`, `UpdateGateRejection` exhaustively matched.
- Grep the `ExecuteUpdate` arm: no `if allowed { … }` with an empty else — every `!allowed` path sends a `Failed`
  reply via `transport_send_best_effort` (not `transport_send`).

## Alternatives considered

- **Per-reason patches** (send Failed from each gate helper) — rejected: scatters the reply across three helpers
  shared by non-update callers and duplicates the message-build. One gate-check + one reply site is the
  root-cause fix.
- **Change the reaper to close records faster** — rejected: treats the symptom, not the silent-drop; the reaper
  must stay lenient for genuine crashes. This fix makes the reaper irrelevant to the gate-rejection case.
- **Bundle the batch fix here** — rejected (split out): the WS-agent batch arm is currently cold (no controller
  sender), and closing a batch record requires a controller-side fallthrough that fires post-update
  hooks/protection-restore for a never-ran item — a semantic decision that belongs in its own spec.

## Out of scope — split to a follow-up spec

**`ExecuteBatchUpdate` gate-rejection + controller batch-close (the batch half of L859).** A separate spec must:
(1) decide whether to reply on the currently-cold WS-agent `ExecuteBatchUpdate` arm; (2) add the controller-side
`fail_pending_unowned_update` fallthrough to `process_single_batch_result` so a `BatchUpdateResult(Failed)` on a
`Pending`/`Queued` row (today from the **SSH** runtime) closes instead of no-oping to `Stale`; (3) broaden
`fail_pending_unowned_update` to `Status IN (Pending, Queued)` — note this also affects the single path (batch
items return via `UpdateResult`), safe only because the filter is `Column::Id`-scoped (closes the reported record
only, never a `Queued` sibling successor); (4) decide whether `fail_pending_unowned_update`'s post-update
hook/protection-restore should fire for a never-ran gate-rejected item, and cover the Queued-successor-untouched
invariant with a test.

Also out of scope: other unspecced immediate-Medium findings in different subsystems (core-agent-ssh L876,
core-controller L894, core-mqtt-scheduler L911, plugins-infra L1042, ui-cli-surface-proxy L1093/1110/1126,
web-api-routes L1226). No change to reaper timings, gate **policies** (freeze/cooldown/machine-id semantics
unchanged — only the *reply* on rejection is added), controller-side record creation, or terminal-status set.
