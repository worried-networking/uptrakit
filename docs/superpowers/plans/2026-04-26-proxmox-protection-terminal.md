# Proxmox Update Protection Terminal Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the interactive WebSocket terminal work from the moment Proxmox backup/snapshot starts, streaming
protection output and seamlessly upgrading to interactive mode once the agent claims the update.

**Architecture:** Add `BroadcastEvent::AgentClaimed` to signal the agent handoff mid-session; relax `interactive_ws`
to accept connections when `execution_owner_service_id` is NULL; close broadcaster channels on all orchestrator
failure paths; defer frontend `openLiveModal` until `update_protection_started` SSE fires.

**Tech Stack:** Rust (tokio, axum, sea-orm, parking_lot), SvelteKit 5, TypeScript

---

## File Map

| File | Change |
| --- | --- |
| `crates/ui/web-api/src/update_output_broadcaster.rs` | Add `AgentClaimed` variant + `send_agent_claimed` |
| `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` | Call `send_agent_claimed` in `handle_update_started` |
| `crates/ui/web-api/src/update_orchestrator.rs` | Call `send_completed` on all three failure paths |
| `crates/ui/web-api/src/routes/interactive_ws.rs` | Allow `service_id: Option<Uuid>`, handle `AgentClaimed`, fix audit |
| `frontend/src/lib/sse.ts` | Add `'update_protection_started'` to `AdminEventType` |
| `frontend/src/routes/software/[id]/+page.svelte` | Defer `openLiveModal` to SSE event |
| `frontend/src/routes/history/+page.svelte` | Handle `update_protection_started` SSE |

---

### Task 1: Add `AgentClaimed` variant and `send_agent_claimed` to broadcaster

**Files:**

- Modify: `crates/ui/web-api/src/update_output_broadcaster.rs`

- [ ] **Step 1: Write the failing test**

  Add to the `#[cfg(test)]` block in `crates/ui/web-api/src/update_output_broadcaster.rs`:

  ```rust
  #[tokio::test]
  async fn send_agent_claimed_delivers_event_without_removing_channel() {
      let broadcaster = UpdateOutputBroadcaster::new();
      let id = Uuid::now_v7();
      let service_id = Uuid::now_v7();

      broadcaster.create_channel(id).await;
      let mut rx = broadcaster.subscribe(id).await.expect("subscriber");

      broadcaster.send_agent_claimed(id, service_id).await;

      let event = rx.recv().await.expect("event");
      match event {
          BroadcastEvent::AgentClaimed { service_id: sid } => {
              assert_eq!(sid, service_id);
          }
          _ => panic!("expected AgentClaimed, got {event:?}"),
      }

      // Channel must still exist after AgentClaimed.
      let rx2 = broadcaster.subscribe(id).await;
      assert!(rx2.is_some(), "channel must survive AgentClaimed");
  }
  ```

- [ ] **Step 2: Run to confirm failure**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite send_agent_claimed 2>&1 | tail -20
  ```

  Expected: compile error — `AgentClaimed` variant and `send_agent_claimed` do not exist yet.

- [ ] **Step 3: Add the variant and method**

  In `crates/ui/web-api/src/update_output_broadcaster.rs`, add `AgentClaimed` to the `BroadcastEvent` enum after `StdinAttention`:

  ```rust
  /// The agent has claimed the update; stdin forwarding is now possible.
  AgentClaimed { service_id: Uuid },
  ```

  Add the method to `impl UpdateOutputBroadcaster` after `send_stdin_attention`:

  ```rust
  /// Notify subscribers that the agent has claimed this update.
  ///
  /// Called from `handle_update_started` (both `Claimed` and `Replay` outcomes).
  /// Does not remove the channel.
  pub async fn send_agent_claimed(&self, update_history_id: Uuid, service_id: Uuid) {
      let channels = self.channels.read().await;
      if let Some(entry) = channels.get(&update_history_id) {
          let _ = entry.tx.send(BroadcastEvent::AgentClaimed { service_id });
      }
  }
  ```

- [ ] **Step 4: Run the test**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite send_agent_claimed 2>&1 | tail -20
  ```

  Expected: `test update_output_broadcaster::tests::send_agent_claimed_delivers_event_without_removing_channel ... ok`

- [ ] **Step 5: Run full broadcaster tests**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite update_output_broadcaster 2>&1 | tail -20
  ```

  Expected: all existing tests still pass.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/ui/web-api/src/update_output_broadcaster.rs
  git commit -m "feat(broadcaster): add AgentClaimed event and send_agent_claimed"
  ```

---

### Task 2: Emit `AgentClaimed` from `handle_update_started`

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`

- [ ] **Step 1: Locate the two call sites**

  Find the match block on `ClaimExecutionOutcome` in `handle_update_started`. It has three arms:
  `Claimed`, `Replay`, `Rejected`. Both `Claimed` and `Replay` call `get_or_create_channel`.
  Add `send_agent_claimed` immediately after each `get_or_create_channel` call.

- [ ] **Step 2: Add the calls**

  In the `Claimed` arm, insert `send_agent_claimed` between `get_or_create_channel` and `broadcast_update_started_events`. The arm must read:

  ```rust
  crate::queries::update_batches::ClaimExecutionOutcome::Claimed(info) => {
      let info = UpdateStartedInfo {
          batch_id: info.batch_id,
          host_id: info.host_id,
          software_item_id: info.software_item_id,
          tenant_id: info.tenant_id,
      };
      state
          .broadcast
          .update_output_broadcaster
          .get_or_create_channel(payload.update_history_id)
          .await;
      state
          .broadcast
          .update_output_broadcaster
          .send_agent_claimed(payload.update_history_id, service_id)
          .await;
      broadcast_update_started_events(state, service_id, payload, &info).await;
  }
  ```

  In the `Replay` arm, add after `get_or_create_channel` (no `broadcast_update_started_events` call in this arm):

  ```rust
  crate::queries::update_batches::ClaimExecutionOutcome::Replay(_) => {
      state
          .broadcast
          .update_output_broadcaster
          .get_or_create_channel(payload.update_history_id)
          .await;
      // NEW
      state
          .broadcast
          .update_output_broadcaster
          .send_agent_claimed(payload.update_history_id, service_id)
          .await;
  }
  ```

  Note: `service_id` is the outer function parameter `service_id: uuid::Uuid`.
  `ClaimExecutionOutcome::Replay` carries `ClaimExecutionInfo` which does not include
  `service_id` — the function parameter is the correct source.

- [ ] **Step 3: Build check**

  ```bash
  cargo check --no-default-features --features db-sqlite -p uptrakit-web-api 2>&1 | tail -20
  ```

  Expected: no errors.

- [ ] **Step 4: Run service_ws handler tests**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite handle_update_started 2>&1 | tail -30
  ```

  Expected: all pass (or no tests match — either is fine; no regressions).

- [ ] **Step 5: Commit**

  ```bash
  git add crates/ui/web-api/src/routes/service_ws/handler/updates.rs
  git commit -m "feat(service-ws): emit AgentClaimed after update claimed or replayed"
  ```

---

### Task 3: Close broadcaster channel on all orchestrator failure paths

**Files:**

- Modify: `crates/ui/web-api/src/update_orchestrator.rs`

- [ ] **Step 1: Write a broadcaster contract test**

  This test verifies the broadcaster contract (channel removed after `send_completed`) that the orchestrator
  will rely on. It passes immediately — that is expected. The orchestrator code changes in Step 3 are the
  actual fix; this test documents the invariant.

  `update_orchestrator.rs` has no existing `#[cfg(test)]` block. Add one at the very bottom of the file:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::update_output_broadcaster::UpdateOutputBroadcaster;

      #[tokio::test]
      async fn send_completed_called_on_failed_protection_closes_channel() {
          let broadcaster = UpdateOutputBroadcaster::new();
          let id = uuid::Uuid::now_v7();
          broadcaster.create_channel(id).await;
          let mut rx = broadcaster.subscribe(id).await.expect("subscriber");

          broadcaster
              .send_completed(id, "failed".to_string(), None)
              .await;

          let event = rx.recv().await.expect("completed event");
          assert!(
              matches!(
                  event,
                  crate::update_output_broadcaster::BroadcastEvent::Completed {
                      ref status, ..
                  } if status == "failed"
              ),
              "expected Completed(failed)"
          );

          // Channel must be gone after send_completed.
          assert!(
              broadcaster.subscribe(id).await.is_none(),
              "channel must be removed after send_completed"
          );
      }
  }
  ```

- [ ] **Step 2: Run to confirm it compiles and passes**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite send_completed_called_on_failed 2>&1 | tail -20
  ```

  Expected: `ok` — broadcaster contract already works. This is not a TDD red step; it documents the invariant the orchestrator must uphold.

- [ ] **Step 3: Add `send_completed` to the three failure paths**

  **Path 1 — `prepare_pre_update_protection` returns `Err(e)`:**

  After the `fail_before_agent_dispatch` block and before `push_software_states_for_tenant`, add:

  ```rust
  state
      .broadcast
      .update_output_broadcaster
      .send_completed(update_history_id, "failed".to_string(), None)
      .await;
  ```

  **Path 2 — `PreUpdateProtectionOutcome::Failed`:**

  After entering the `Failed` arm and before `push_software_states_for_tenant`, add:

  ```rust
  state
      .broadcast
      .update_output_broadcaster
      .send_completed(update_history_id, "failed".to_string(), None)
      .await;
  ```

  **Path 3 — `dispatch_update_to_agent` returns `Err(e)`:**

  After the `fail_before_agent_dispatch` block and before `push_software_states_for_tenant`, add:

  ```rust
  state
      .broadcast
      .update_output_broadcaster
      .send_completed(update_history_id, "failed".to_string(), None)
      .await;
  ```

  In all three paths, `send_completed` must be called unconditionally — do not gate it on `fail_before_agent_dispatch` succeeding.

- [ ] **Step 4: Build check**

  ```bash
  cargo check --no-default-features --features db-sqlite -p uptrakit-web-api 2>&1 | tail -20
  ```

  Expected: no errors.

- [ ] **Step 5: Run orchestrator tests**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite orchestrator 2>&1 | tail -20
  ```

  Expected: all pass.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/ui/web-api/src/update_orchestrator.rs
  git commit -m "fix(orchestrator): close broadcaster channel on all three failure paths"
  ```

---

### Task 4: Allow `interactive_ws` to connect during protection phase

**Files:**

- Modify: `crates/ui/web-api/src/routes/interactive_ws.rs`

- [ ] **Step 1: Write a failing test**

  Add to the test block in `crates/ui/web-api/src/routes/interactive_ws.rs`:

  ```rust
  #[cfg(feature = "db-sqlite")]
  #[tokio::test]
  async fn interactive_ws_connects_during_protection_phase_service_id_null() {
      let (base_url, app) = serve_app().await;
      let token = crate::test_harness::fixtures::register_and_get_token(&app.client()).await;
      let host = crate::test_harness::fixtures::insert_host(&app.db, app.tenant_id).await;
      let software_item = insert_software_item(&app.db, app.tenant_id).await;
      let update = insert_update_history_row(
          &app.db,
          app.tenant_id,
          host.id,
          software_item.id,
          UpdateStatus::InProgress,
          None, // execution_owner_service_id = NULL (protection phase)
      )
      .await;
      app.state
          .broadcast
          .update_output_broadcaster
          .create_channel(update.id)
          .await;

      let result = tokio_tungstenite::connect_async(format!(
          "{base_url}/api/v1/update-history/{}/interactive?token={token}",
          update.id
      ))
      .await;

      assert!(
          result.is_ok(),
          "WS must connect when service_id is NULL (protection phase)"
      );
      let (mut ws, _) = result.unwrap();
      ws.close(None).await.expect("close");
  }
  ```

- [ ] **Step 2: Run to confirm failure**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite interactive_ws_connects_during_protection 2>&1 | tail -20
  ```

  Expected: test fails — server returns 409 (execution_owner_missing).

- [ ] **Step 3: Change step 6 — allow `None` service_id**

  Find step 6 in `interactive_ws.rs` (the block checking `execution_owner_service_id`). Replace the hard-reject with:

  ```rust
  // 6. Resolve the executing agent's service_id from the update record.
  //
  // NULL during the protection phase (set by set_inprogress_for_orchestrator).
  // Allow connection with service_id = None — session is read-only until
  // AgentClaimed fires.
  let service_id: Option<Uuid> = record.execution_owner_service_id;
  ```

  Delete the entire old `match` block that returned 409 on `None`.

- [ ] **Step 4: Change step 7 — skip connectivity check when `service_id` is `None`**

  Replace the connectivity check with:

  ```rust
  // 7. Verify the agent is still connected (only when service_id is known).
  if let Some(sid) = service_id {
      if !state.service_connections.is_connected(&sid).await {
          emit_interactive_session_audit(
              InteractiveAuditCtx {
                  state: &state,
                  actor: audit_actor,
              },
              record_id,
              Some(sid),
              uptrakit_audit_log::AuditOutcome::Denied,
              Some("service_not_connected"),
              None,
          );
          state
              .interactive_sessions
              .release(record_id, auth_user.user_id);
          return error_response(StatusCode::CONFLICT, "Agent is not connected");
      }
  }
  ```

- [ ] **Step 5: Fix step 8 audit call — pass `service_id` directly**

  Find the step-8 audit call. Change `Some(service_id)` to `service_id` (it is already `Option<Uuid>`):

  ```rust
  emit_interactive_session_audit(
      InteractiveAuditCtx {
          state: &state,
          actor: audit_actor,
      },
      record_id,
      service_id,   // Option<Uuid> — was Some(service_id)
      uptrakit_audit_log::AuditOutcome::Success,
      None,
      None,
  );
  ```

- [ ] **Step 6: Update `ws.on_upgrade` — pass `service_id: Option<Uuid>`**

  ```rust
  ws.max_message_size(MAX_INTERACTIVE_WS_MESSAGE_SIZE)
      .on_upgrade(move |socket| {
          handle_interactive_session(socket, state, record_id, service_id, auth_user, audit_actor)
      })
  ```

- [ ] **Step 7: Update `handle_interactive_session` signature**

  Change:

  ```rust
  async fn handle_interactive_session(
      socket: WebSocket,
      state: Arc<AppState>,
      update_history_id: Uuid,
      service_id: Option<Uuid>,   // was: Uuid
      user: AuthenticatedUser,
      audit_actor: InteractiveAuditActor,
  ) {
  ```

  At the top of the body:

  ```rust
  let mut local_service_id: Option<Uuid> = service_id;
  ```

  Replace all subsequent uses of the bare `service_id` variable with `local_service_id`.

- [ ] **Step 8: Add `AgentClaimed` arm to the WS broadcast loop**

  In the `tokio::select!` arm that receives from the broadcast channel, add:

  ```rust
  Ok(BroadcastEvent::AgentClaimed { service_id: claimed_id }) => {
      if local_service_id.is_none() {
          if state.service_connections.is_connected(&claimed_id).await {
              local_service_id = Some(claimed_id);
              tracing::debug!(
                  service_id = %claimed_id,
                  %update_history_id,
                  "agent claimed update — stdin forwarding enabled"
              );
          }
          // If not connected: keep None. Agent reconnect re-sends
          // UpdateStarted → Replay → send_agent_claimed fires again.
      }
      // Do NOT send any message to the WS client — capability upgrade is
      // internal state only. Per spec: AgentClaimed "does not remove the
      // channel" and only signals that stdin forwarding is now possible.
  }
  ```

- [ ] **Step 9: Guard stdin helpers against `None` service_id**

  Update `handle_client_message` signature and add early return:

  ```rust
  async fn handle_client_message(
      state: &AppState,
      text: &str,
      update_history_id: Uuid,
      service_id: Option<Uuid>,   // was: Uuid
      audit_actor: InteractiveAuditActor,
  ) {
      let Some(sid) = service_id else {
          tracing::debug!(%update_history_id, "stdin during protection phase — skipping");
          return;
      };
      // rest of function uses `sid`
  ```

  Update `forward_interactive_stdin` the same way, or add the guard at the call site.

  For the binary stdin path in the WS loop:

  ```rust
  Some(Ok(Message::Binary(data))) => {
      let Some(sid) = local_service_id else {
          tracing::debug!(%update_history_id, "binary stdin during protection phase — skipping");
          continue;
      };
      // use `sid` for forwarding
  }
  ```

- [ ] **Step 10: Build check**

  ```bash
  cargo check --no-default-features --features db-sqlite -p uptrakit-web-api 2>&1 | tail -30
  ```

  Expected: no errors. Fix any remaining `service_id` type mismatches.

- [ ] **Step 11: Run the new test**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite interactive_ws_connects_during_protection 2>&1 | tail -20
  ```

  Expected: `ok`.

- [ ] **Step 12: Run all interactive_ws tests**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite interactive_ws 2>&1 | tail -40
  ```

  Expected: all pass.

- [ ] **Step 13: Commit**

  ```bash
  git add crates/ui/web-api/src/routes/interactive_ws.rs
  git commit -m "feat(interactive-ws): allow connection during protection phase, handle AgentClaimed"
  ```

---

### Task 5: Frontend — add `update_protection_started` to `AdminEventType`

**Files:**

- Modify: `frontend/src/lib/sse.ts`

- [ ] **Step 1: Add the type**

  In `frontend/src/lib/sse.ts`, find the `AdminEventType` union. Add `'update_protection_started'` immediately before `'update_started'`:

  ```typescript
  | 'update_protection_started'
  | 'update_started'
  ```

- [ ] **Step 2: Type-check**

  ```bash
  cd frontend && npm run check 2>&1 | tail -20
  ```

  Expected: no errors.

- [ ] **Step 3: Commit**

  ```bash
  git add frontend/src/lib/sse.ts
  git commit -m "feat(sse): add update_protection_started to AdminEventType"
  ```

---

### Task 6: Frontend history page — react to `update_protection_started`

**Files:**

- Modify: `frontend/src/routes/history/+page.svelte`

- [ ] **Step 1: Add the SSE handler**

  In `frontend/src/routes/history/+page.svelte`, find the `update_started` SSE handler. Add a new handler block directly before it:

  ```typescript
  } else if (eventType === 'update_protection_started') {
      const historyId = data.update_history_id as string;
      items = items.map((i) =>
          i.id === historyId ? { ...i, status: 'in_progress' as const } : i
      );
      if (
          !items.some((i) => i.id === historyId) &&
          currentPage === 1 &&
          (statusFilter === 'all' || statusFilter === 'in_progress')
      ) {
          loadHistory(1);
      }
  ```

- [ ] **Step 2: Type-check**

  ```bash
  cd frontend && npm run check 2>&1 | tail -20
  ```

  Expected: no errors.

- [ ] **Step 3: Run frontend tests**

  ```bash
  cd frontend && npm run test 2>&1 | tail -30
  ```

  Expected: all pass.

- [ ] **Step 4: Commit**

  ```bash
  git add frontend/src/routes/history/+page.svelte
  git commit -m "feat(history): show in_progress on update_protection_started SSE"
  ```

---

### Task 7: Frontend software detail — defer `openLiveModal` to SSE

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte`

- [ ] **Step 1: Add pending state variables**

  After the existing live terminal modal state declarations, add:

  ```typescript
  let pendingLiveHistoryId: string | null = $state(null);
  let pendingLiveHostName: string = $state('');
  ```

- [ ] **Step 2: Replace immediate `openLiveModal` call**

  Find `openLiveModal(res.update_history_id, hostName);` in the trigger response handler. Replace with:

  ```typescript
  pendingLiveHistoryId = res.update_history_id;
  pendingLiveHostName = hostName;
  ```

- [ ] **Step 3: Add `update_protection_started` SSE handler**

  After the existing `subscribeToEvent` calls in `onMount`, add:

  ```typescript
  subscribeToEvent('update_protection_started', (data) => {
      if (data.update_history_id === pendingLiveHistoryId && pendingLiveHistoryId) {
          const histId = pendingLiveHistoryId;
          const hostName = pendingLiveHostName;
          pendingLiveHistoryId = null;
          pendingLiveHostName = '';
          openLiveModal(histId, hostName);
      }
      if (data.software_item_id === id) loadItem(true);
  }),
  ```

- [ ] **Step 4: Update `update_started` handler — add fallback modal open**

  Find `subscribeToEvent('update_started', ...)`. Add at the top of its callback (non-Proxmox path where protection is skipped):

  ```typescript
  subscribeToEvent('update_started', (data) => {
      if (data.update_history_id === pendingLiveHistoryId && pendingLiveHistoryId) {
          const histId = pendingLiveHistoryId;
          const hostName = pendingLiveHostName;
          pendingLiveHistoryId = null;
          pendingLiveHostName = '';
          openLiveModal(histId, hostName);
      }
      if (data.software_item_id === id) loadItem(true);
  }),
  ```

- [ ] **Step 5: Type-check**

  ```bash
  cd frontend && npm run check 2>&1 | tail -20
  ```

  Expected: no errors.

- [ ] **Step 6: Run frontend tests**

  ```bash
  cd frontend && npm run test 2>&1 | tail -30
  ```

  Expected: all pass.

- [ ] **Step 7: Commit**

  ```bash
  git add frontend/src/routes/software/[id]/+page.svelte
  git commit -m "feat(software-detail): defer live modal until protection-started SSE fires"
  ```

---

### Task 8: Full quality gate

- [ ] **Step 1: Rust format**

  ```bash
  cargo fmt --all
  ```

- [ ] **Step 2: Clippy**

  ```bash
  cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error" | head -20
  cargo clippy --all-targets --all-features 2>&1 | grep "^error" | head -20
  ```

  Expected: no errors.

- [ ] **Step 3: Full Rust test suite**

  ```bash
  cargo test --all-features 2>&1 | tail -40
  ```

  Expected: all pass.

- [ ] **Step 4: Frontend full check**

  ```bash
  cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build 2>&1 | tail -30
  ```

  Expected: all pass.

- [ ] **Step 5: Commit formatting if needed**

  ```bash
  git add -p
  git commit -m "chore: formatting fixes from quality gate"
  ```
