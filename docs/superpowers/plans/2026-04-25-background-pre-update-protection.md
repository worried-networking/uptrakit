# Background Pre-Update Protection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run Proxmox pre-update protection (snapshot/backup) in a background task so the HTTP
trigger returns immediately, and stream protection output to the terminal in real time.

**Architecture:** `trigger_update_for_host` returns a `PendingProtectionWork` bundle instead of
running protection inline. A new `update_orchestrator` module in `web-api` spawns a tokio task
that transitions the record to `InProgress`, runs protection, then dispatches to the agent.
The orchestrator-owned sentinel is `status = InProgress` + `execution_owner_service_id = NULL`.

**Tech Stack:** Rust, SeaORM (PostgreSQL/SQLite), tokio, axum, `uptrakit_shared_db` entities, `rootcause` errors.

---

## File Map

| File | Action | Responsibility |
| --- | --- | --- |
| `crates/plugins/infrastructure/core/src/roles.rs` | Modify | Add `output_tx` field + `with_output_tx()` to `ControllerProtectionContext` |
| `crates/plugins/infrastructure/proxmox/src/update_protection.rs` | Modify | Send status lines to `ctx.output_tx` at protection milestones |
| `crates/shared/web-api-types/src/events.rs` | Modify | Add `AdminEvent::UpdateProtectionStarted` variant |
| `crates/ui/web-api-queries/src/queries/update_dispatch.rs` | Modify | `Clone` on `ValidatedUpdateTarget`; new helpers `set_inprogress_for_orchestrator`, `insert_protection_output_line`; promote `fail_before_agent_dispatch` to `pub`; add `output_tx` param to `prepare_pre_update_protection` |
| `crates/ui/web-api-queries/src/queries/update_triggers.rs` | Modify | Add `PendingProtectionWork`; update `TriggerUpdateResult`; remove inline protection+dispatch from `trigger_update_for_host` |
| `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs` | Modify | New orchestrator-InProgress CAS case in `claim_or_replay_update_start_db`; add `mark_orchestrator_inprogress_as_failed_on_reconnect`; pass `output_tx: None` to updated `prepare_pre_update_protection` call |
| `crates/ui/web-api/src/update_orchestrator.rs` | Create | `spawn_protection_and_dispatch`, `run_protection_and_dispatch`, `forward_protection_output` |
| `crates/ui/web-api/src/lib.rs` | Modify | Declare `pub(crate) mod update_orchestrator` |
| `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` | Modify | `handle_update_started` Claimed arm → `get_or_create_channel`; reconnect recovery for orchestrator InProgress; `prepare_pending_replay_messages` spawns orchestrator for Pending+unprotected; `dispatch_next_queued_update_with_notifier` drops `notifier`/`protection` params |
| `crates/ui/web-api/src/routes/software_items/mod.rs` | Modify | Spawn orchestrator when `pending_protection_work.is_some()` |
| `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs` | Modify | Same pattern |
| `crates/ui/web-api/src/actions/software_items.rs` | Modify | Drop `protection` param from `trigger_update` |

---

## Task 1: Extend `ControllerProtectionContext` with `output_tx`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs:654-681`

- [ ] **Step 1: Add `output_tx` field and `with_output_tx()` builder**

  In `roles.rs`, after `update_history_id: Uuid` in the `ControllerProtectionContext` struct (line 661), add:

  ```rust
  /// Optional channel for streaming protection status lines to the orchestrator.
  /// `None` for batch and recovery callers; `Some` when called from the orchestrator.
  pub output_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
  ```

  In the `impl<'a> ControllerProtectionContext<'a>` block, the `new()` function body sets all fields. Add `output_tx: None` to the struct literal in `new()`:

  ```rust
  pub fn new(
      controller: &'a dyn UpdateProtectionController,
      tenant_id: Uuid,
      host_id: Uuid,
      software_item_id: Uuid,
      update_history_id: Uuid,
  ) -> Self {
      Self {
          controller,
          tenant_id,
          host_id,
          software_item_id,
          update_history_id,
          output_tx: None,
      }
  }
  ```

  Add builder after `new()`:

  ```rust
  /// Attach an output sender so the plugin can stream status lines to the orchestrator.
  pub fn with_output_tx(
      mut self,
      tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
  ) -> Self {
      self.output_tx = Some(tx);
      self
  }
  ```

- [ ] **Step 2: Verify compilation**

  ```bash
  cargo check -p uptrakit-plugin-infrastructure-core --all-features
  ```

  Expected: no errors. Any external plugin that uses `..` spread on `ControllerProtectionContext`
  still compiles because `#[non_exhaustive]` already requires `..`.

- [ ] **Step 3: Commit**

  ```text
  feat(infrastructure-core): add output_tx field to ControllerProtectionContext
  ```

---

## Task 2: Add `AdminEvent::UpdateProtectionStarted`

**Files:**

- Modify: `crates/shared/web-api-types/src/events.rs`

- [ ] **Step 1: Write failing test**

  In the `tests` module (after line 276), add:

  ```rust
  #[test]
  fn update_protection_started_event_name() {
      let id = Uuid::nil();
      let event = AdminEvent::UpdateProtectionStarted {
          update_history_id: id,
          host_id: id,
          software_item_id: id,
      };
      assert_eq!(event.event_name(), "update_protection_started");
  }
  ```

- [ ] **Step 2: Run test to verify it fails**

  ```bash
  cargo test -p uptrakit-web-api-types update_protection_started_event_name
  ```

  Expected: compile error — `UpdateProtectionStarted` variant does not exist.

- [ ] **Step 3: Add the variant**

  After `UpdateTriggered` variant (around line 51, before `UpdateStarted`), add:

  ```rust
  /// Controller pre-update protection started for a software update.
  ///
  /// Emitted by the orchestrator when protection (snapshot/backup) begins.
  /// The frontend transitions the update record to In Progress state on receipt.
  UpdateProtectionStarted {
      update_history_id: Uuid,
      host_id: Uuid,
      software_item_id: Uuid,
  },
  ```

  Add arm to `event_name()` match (after `"update_triggered"` arm):

  ```rust
  Self::UpdateProtectionStarted { .. } => "update_protection_started",
  ```

  Add to `all_variants()` vec (after `UpdateTriggered` entry):

  ```rust
  AdminEvent::UpdateProtectionStarted {
      update_history_id: id,
      host_id: id,
      software_item_id: id,
  },
  ```

  Update count assertion (line 276):

  ```rust
  assert_eq!(all_variants().len(), 20);
  ```

- [ ] **Step 4: Run tests**

  ```bash
  cargo test -p uptrakit-web-api-types
  ```

  Expected: all pass.

- [ ] **Step 5: Commit**

  ```text
  feat(web-api-types): add AdminEvent::UpdateProtectionStarted
  ```

---

## Task 3: Query-layer helpers in `update_dispatch.rs`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`

This task adds three things independently: `Clone` on `ValidatedUpdateTarget`, two new public
helpers (`set_inprogress_for_orchestrator`, `insert_protection_output_line`), and promotes
`fail_before_agent_dispatch` to `pub`.

- [ ] **Step 1: Derive `Clone` on `ValidatedUpdateTarget`**

  At line 226, change:

  ```rust
  #[derive(Debug)]
  pub struct ValidatedUpdateTarget {
  ```

  to:

  ```rust
  #[derive(Clone, Debug)]
  pub struct ValidatedUpdateTarget {
  ```

  All fields are SeaORM `Model` types (which derive `Clone`) and `Vec<PluginAssignment>`.
  Verify `PluginAssignment` is `Clone` — it is (defined in `agent-core`, derives `Clone`).

- [ ] **Step 2: Promote `fail_before_agent_dispatch` to `pub`**

  At line 705, change:

  ```rust
  async fn fail_before_agent_dispatch(
  ```

  to:

  ```rust
  pub async fn fail_before_agent_dispatch(
  ```

  This is called cross-crate from `web-api/src/update_orchestrator.rs`.

- [ ] **Step 3: Write failing test for `set_inprogress_for_orchestrator`**

  In the existing `#[cfg(test)]` block of `update_dispatch.rs`, add:

  ```rust
  #[tokio::test]
  async fn set_inprogress_for_orchestrator_transitions_pending_sets_started_at() {
      use sea_orm::EntityTrait;
      use uptrakit_shared_db::entity::update_history;
      // Use the same DB setup as existing tests in this file.
      let db = {
          use sea_orm::Database;
          let db = Database::connect("sqlite::memory:").await.unwrap();
          uptrakit_shared_db::migration::run_migrations(&db).await.unwrap();
          db
      };
      // Insert a minimal Pending record (copy pattern from trigger tests in this file).
      let now = time::OffsetDateTime::now_utc();
      let id = uuid::Uuid::now_v7();
      update_history::ActiveModel {
          id: sea_orm::Set(id),
          tenant_id: sea_orm::Set(uuid::Uuid::now_v7()),
          host_id: sea_orm::Set(uuid::Uuid::now_v7()),
          software_item_id: sea_orm::Set(uuid::Uuid::now_v7()),
          host_software_item_id: sea_orm::Set(None),
          from_version: sea_orm::Set(None),
          to_version: sea_orm::Set(Some("1.0.0".to_string())),
          status: sea_orm::Set(update_history::UpdateStatus::Pending),
          output: sea_orm::Set(String::new()),
          output_bytes: sea_orm::Set(0),
          actor_type: sea_orm::Set("user".to_string()),
          actor_id: sea_orm::Set(String::new()),
          execution_owner_service_id: sea_orm::Set(None),
          execution_owner_instance_id: sea_orm::Set(None),
          started_at: sea_orm::Set(None),
          completed_at: sea_orm::Set(None),
          created_at: sea_orm::Set(now),
          update_category: sea_orm::Set("security".to_string()),
          batch_id: sea_orm::Set(None),
          interactive: sea_orm::Set(false),
          output_truncated: sea_orm::Set(false),
          pre_update_protection_status: sea_orm::Set(None),
          pre_update_protection_summary: sea_orm::Set(None),
          recovery_hint: sea_orm::Set(None),
      }
      .insert(&db)
      .await
      .unwrap();

      let rows = set_inprogress_for_orchestrator(&db, id).await.unwrap();
      assert_eq!(rows, 1, "CAS must affect exactly one row");

      let row = update_history::Entity::find_by_id(id)
          .one(&db)
          .await
          .unwrap()
          .unwrap();
      assert_eq!(row.status, update_history::UpdateStatus::InProgress);
      assert_eq!(row.pre_update_protection_status.as_deref(), Some("in_progress"));
      assert!(row.execution_owner_service_id.is_none(), "orchestrator sentinel: owner must be NULL");
      assert!(row.started_at.is_some(), "started_at must be set");
  }

  #[tokio::test]
  async fn set_inprogress_for_orchestrator_returns_zero_when_not_pending() {
      use sea_orm::EntityTrait;
      use uptrakit_shared_db::entity::update_history;
      let db = {
          use sea_orm::Database;
          let db = Database::connect("sqlite::memory:").await.unwrap();
          uptrakit_shared_db::migration::run_migrations(&db).await.unwrap();
          db
      };
      let now = time::OffsetDateTime::now_utc();
      let id = uuid::Uuid::now_v7();
      // Insert as InProgress (already claimed by an agent).
      update_history::ActiveModel {
          id: sea_orm::Set(id),
          tenant_id: sea_orm::Set(uuid::Uuid::now_v7()),
          host_id: sea_orm::Set(uuid::Uuid::now_v7()),
          software_item_id: sea_orm::Set(uuid::Uuid::now_v7()),
          host_software_item_id: sea_orm::Set(None),
          from_version: sea_orm::Set(None),
          to_version: sea_orm::Set(Some("1.0.0".to_string())),
          status: sea_orm::Set(update_history::UpdateStatus::InProgress),
          output: sea_orm::Set(String::new()),
          output_bytes: sea_orm::Set(0),
          actor_type: sea_orm::Set("user".to_string()),
          actor_id: sea_orm::Set(String::new()),
          execution_owner_service_id: sea_orm::Set(Some(uuid::Uuid::now_v7())),
          execution_owner_instance_id: sea_orm::Set(None),
          started_at: sea_orm::Set(Some(now)),
          completed_at: sea_orm::Set(None),
          created_at: sea_orm::Set(now),
          update_category: sea_orm::Set("security".to_string()),
          batch_id: sea_orm::Set(None),
          interactive: sea_orm::Set(false),
          output_truncated: sea_orm::Set(false),
          pre_update_protection_status: sea_orm::Set(None),
          pre_update_protection_summary: sea_orm::Set(None),
          recovery_hint: sea_orm::Set(None),
      }
      .insert(&db)
      .await
      .unwrap();

      let rows = set_inprogress_for_orchestrator(&db, id).await.unwrap();
      assert_eq!(rows, 0, "CAS must not affect an already-InProgress row");
  }
  ```

- [ ] **Step 4: Run tests to verify they fail**

  ```bash
  cargo test -p uptrakit-web-api-queries set_inprogress_for_orchestrator
  ```

  Expected: compile error — `set_inprogress_for_orchestrator` not found.

- [ ] **Step 5: Implement `set_inprogress_for_orchestrator`**

  Add after `build_controller_post_update_context` (around line 681 in `update_dispatch.rs`):

  ```rust
  /// Atomically transition a `Pending` record to `InProgress` for orchestrator ownership.
  ///
  /// Sets `status = InProgress`, `pre_update_protection_status = "in_progress"`,
  /// `execution_owner_service_id = NULL`, and `started_at = now()`.
  ///
  /// CAS guard: only updates if `status = Pending`. Returns the number of rows
  /// affected (1 = success, 0 = raced or record gone).
  pub async fn set_inprogress_for_orchestrator(
      db: &DatabaseConnection,
      update_history_id: Uuid,
  ) -> Result<u64> {
      let now = OffsetDateTime::now_utc();
      let result = UpdateHistory::update_many()
          .filter(update_history::Column::Id.eq(update_history_id))
          .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
          .col_expr(
              update_history::Column::Status,
              Expr::value(update_history::UpdateStatus::InProgress),
          )
          .col_expr(
              update_history::Column::PreUpdateProtectionStatus,
              Expr::value(Some("in_progress".to_string())),
          )
          .col_expr(
              update_history::Column::ExecutionOwnerServiceId,
              Expr::value(Option::<Uuid>::None),
          )
          .col_expr(
              update_history::Column::StartedAt,
              Expr::value(Some(now)),
          )
          .exec(db)
          .await
          .context_to()?;
      Ok(result.rows_affected)
  }
  ```

  Also add `insert_protection_output_line` after `set_inprogress_for_orchestrator`:

  ```rust
  /// Insert one protection output line into `update_output_line`.
  ///
  /// No ownership check — called from the orchestrator's `forward_protection_output`
  /// task which already knows the record belongs to the orchestrator.
  pub async fn insert_protection_output_line(
      db: &DatabaseConnection,
      update_history_id: Uuid,
      line_id: Uuid,
      text: String,
      stream: uptrakit_shared_types::OutputStreamType,
      timestamp: time::OffsetDateTime,
  ) -> Result<()> {
      use uptrakit_shared_db::entity::update_output_line;
      use sea_orm::Set;
      UpdateOutputLine::insert(update_output_line::ActiveModel {
          id: Set(line_id),
          update_history_id: Set(update_history_id),
          stream: Set(stream),
          output: Set(text),
          created_at: Set(timestamp),
      })
      .exec(db)
      .await
      .context_to()?;
      Ok(())
  }
  ```

  `UpdateOutputLine` is already in scope via the existing `prelude::*` glob import. Add only
  `update_output_line` (the module) to the existing entity import block at the top of the file:

  ```rust
  use uptrakit_shared_db::entity::{
      // ... existing entries ...,
      update_output_line,
  };
  ```

- [ ] **Step 6: Run tests**

  ```bash
  cargo test -p uptrakit-web-api-queries set_inprogress_for_orchestrator
  ```

  Expected: both pass.

- [ ] **Step 7: Verify full crate compiles**

  ```bash
  cargo check -p uptrakit-web-api-queries --all-features
  ```

  Expected: no errors.

- [ ] **Step 8: Commit**

  ```text
  feat(web-api-queries): add orchestrator query helpers and promote fail_before_agent_dispatch
  ```

---

## Task 4: Add `output_tx` param to `prepare_pre_update_protection` and update all call sites

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs:746`
- Modify: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:186`
- Modify: `crates/plugins/infrastructure/proxmox/src/update_protection.rs` (send lines via `ctx.output_tx`)

- [ ] **Step 1: Update `prepare_pre_update_protection` signature**

  At line 746 of `update_dispatch.rs`, change the function signature from:

  ```rust
  pub async fn prepare_pre_update_protection(
      db: &DatabaseConnection,
      protection: Option<Arc<dyn ControllerUpdateProtection>>,
      target: &ValidatedUpdateTarget,
      update_history_id: Uuid,
  ) -> Result<PreUpdateProtectionOutcome> {
  ```

  to:

  ```rust
  pub async fn prepare_pre_update_protection(
      db: &DatabaseConnection,
      protection: Option<Arc<dyn ControllerUpdateProtection>>,
      target: &ValidatedUpdateTarget,
      update_history_id: Uuid,
      output_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
  ) -> Result<PreUpdateProtectionOutcome> {
  ```

  Inside the function body, pass `output_tx` to `build_controller_protection_context`. Change the `build_controller_protection_context` call:

  ```rust
  let ctx = build_controller_protection_context(&controller, target, update_history_id);
  ```

  to:

  ```rust
  let ctx = build_controller_protection_context(&controller, target, update_history_id);
  let ctx = if let Some(tx) = output_tx {
      ctx.with_output_tx(tx)
  } else {
      ctx
  };
  ```

- [ ] **Step 2: Fix the call site in `update_triggers.rs`**

  At line 174 of `update_triggers.rs`, change:

  ```rust
  let pre_update_outcome = prepare_pre_update_protection(
      db,
      dispatch.protection.clone(),
      &target,
      update_history_id,
  )
  .await?;
  ```

  to:

  ```rust
  let pre_update_outcome = prepare_pre_update_protection(
      db,
      dispatch.protection.clone(),
      &target,
      update_history_id,
      None,
  )
  .await?;
  ```

  **Note:** This call site is temporary — it will be removed entirely in Task 5 when `trigger_update_for_host` is refactored.

- [ ] **Step 3: Fix the call site in `update_batches/dispatch.rs`**

  At line 186-188, change:

  ```rust
  let pre_update_outcome =
      prepare_pre_update_protection(db, dispatch.protection.clone(), &target, next_record.id)
          .await?;
  ```

  to:

  ```rust
  let pre_update_outcome =
      prepare_pre_update_protection(db, dispatch.protection.clone(), &target, next_record.id, None)
          .await?;
  ```

- [ ] **Step 4: Fix the call site in `update_batches/mod.rs`**

  At line 235 of `update_batches/mod.rs`, change:

  ```rust
  let pre_update_outcome = prepare_pre_update_protection(
      db,
      dispatch.protection.clone(),
      target,
      update_history_id,
  )
  .await?;
  ```

  to:

  ```rust
  let pre_update_outcome = prepare_pre_update_protection(
      db,
      dispatch.protection.clone(),
      target,
      update_history_id,
      None,
  )
  .await?;
  ```

  This is the `create_batch` call site — batch protection stays inline, so `None` is correct.

- [ ] **Step 5: Verify all crates compile**

  ```bash
  cargo check -p uptrakit-web-api-queries --all-features
  ```

  Expected: no errors.

- [ ] **Step 7: Add `output_tx` line sending to Proxmox plugin**

  In `crates/plugins/infrastructure/proxmox/src/update_protection.rs`, in `prepare_snapshot_protection`:

  - After `"creating Proxmox snapshot for pre-update protection"` tracing (around line 318), add:

    ```rust
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(
            format!(
                "Creating Proxmox snapshot for {} (VMID {})…\n",
                mapping.proxmox_node, mapping.proxmox_vmid
            )
            .into_bytes(),
        );
    }
    ```

  - After `"Proxmox snapshot created successfully"` tracing (around line 411), add:

    ```rust
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(
            format!(
                "Proxmox snapshot '{}' created successfully.\n",
                snapshot_name
            )
            .into_bytes(),
        );
    }
    ```

  - In the `wait_for_task_completion` error branch (around line 377), add before `return Ok(snapshot_decision_failure())`:

    ```rust
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(
            format!(
                "Proxmox snapshot task failed: {error}\n"
            )
            .into_bytes(),
        );
    }
    ```

  Apply the same pattern in `prepare_backup_protection`:

  - After `"starting Proxmox backup for pre-update protection"` tracing (around line 506):

    ```rust
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(
            format!(
                "Starting Proxmox backup for {} (VMID {}) to storage '{}'…\n",
                mapping.proxmox_node, mapping.proxmox_vmid, target_storage_id
            )
            .into_bytes(),
        );
    }
    ```

  - After `"Proxmox backup completed successfully"` tracing (around line 595):

    ```rust
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(b"Proxmox backup completed successfully.\n".to_vec());
    }
    ```

  - In the `wait_for_task_completion` error branch (around line 562):

    ```rust
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(
            format!("Proxmox backup task failed: {error}\n").into_bytes(),
        );
    }
    ```

- [ ] **Step 8: Verify Proxmox plugin compiles**

  ```bash
  cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
  ```

  Expected: no errors.

- [ ] **Step 9: Commit**

  ```text
  feat(update-dispatch): add output_tx param to prepare_pre_update_protection; stream lines in Proxmox plugin
  ```

---

## Task 5: Refactor `trigger_update_for_host` — remove inline protection+dispatch

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_triggers.rs`

- [ ] **Step 1: Write failing tests**

  In the `#[cfg(test)]` block of `update_triggers.rs`, add:

  ```rust
  #[tokio::test]
  async fn trigger_update_pending_returns_work_bundle() {
      // Pending case must return pending_protection_work (not None).
      let db = setup_db().await;
      let f = insert_base_fixture(&db).await;

      let result = trigger_update_for_host(
          &db,
          TriggerUpdateParams {
              tenant_id: f.tenant_id,
              item_id: f.item_id,
              host_id: f.host_id,
              to_version: "1.1.0".to_string(),
              actor_type: ActorType::User.as_str(),
              actor_id: "user-1",
              release_info: None,
              interactive: false,
          },
      )
      .await
      .unwrap();

      assert!(
          matches!(result.initial_status, update_history::UpdateStatus::Pending),
          "expected Pending"
      );
      assert!(
          result.pending_protection_work.is_some(),
          "Pending case must return a work bundle"
      );
      let work = result.pending_protection_work.unwrap();
      assert_eq!(work.to_version, "1.1.0");
      assert!(!work.interactive);
  }

  #[tokio::test]
  async fn trigger_update_queued_returns_no_work_bundle() {
      // Queued case (host busy) must return None for pending_protection_work.
      let db = setup_db().await;
      let f = insert_base_fixture(&db).await;
      let now = time::OffsetDateTime::now_utc();

      // Seed a Pending record so the host appears busy.
      update_history::ActiveModel {
          id: sea_orm::Set(Uuid::now_v7()),
          tenant_id: sea_orm::Set(f.tenant_id),
          host_id: sea_orm::Set(f.host_id),
          software_item_id: sea_orm::Set(f.item_id),
          host_software_item_id: sea_orm::Set(None),
          from_version: sea_orm::Set(None),
          to_version: sea_orm::Set(Some("1.0.0".to_string())),
          status: sea_orm::Set(update_history::UpdateStatus::Pending),
          output: sea_orm::Set(String::new()),
          output_bytes: sea_orm::Set(0),
          actor_type: sea_orm::Set("user".to_string()),
          actor_id: sea_orm::Set(String::new()),
          execution_owner_service_id: sea_orm::Set(None),
          execution_owner_instance_id: sea_orm::Set(None),
          started_at: sea_orm::Set(Some(now)),
          completed_at: sea_orm::Set(None),
          created_at: sea_orm::Set(now),
          update_category: sea_orm::Set("feature".to_string()),
          batch_id: sea_orm::Set(None),
          interactive: sea_orm::Set(false),
          output_truncated: sea_orm::Set(false),
          pre_update_protection_status: sea_orm::Set(None),
          pre_update_protection_summary: sea_orm::Set(None),
          recovery_hint: sea_orm::Set(None),
      }
      .insert(&db)
      .await
      .unwrap();

      let result = trigger_update_for_host(
          &db,
          TriggerUpdateParams {
              tenant_id: f.tenant_id,
              item_id: f.item_id,
              host_id: f.host_id,
              to_version: "1.2.0".to_string(),
              actor_type: ActorType::User.as_str(),
              actor_id: "user-1",
              release_info: None,
              interactive: false,
          },
      )
      .await
      .unwrap();

      assert!(
          matches!(result.initial_status, update_history::UpdateStatus::Queued),
          "expected Queued"
      );
      assert!(
          result.pending_protection_work.is_none(),
          "Queued case must return no work bundle"
      );
  }
  ```

- [ ] **Step 2: Run tests to verify they fail**

  ```bash
  cargo test -p uptrakit-web-api-queries trigger_update_pending_returns_work_bundle trigger_update_queued_returns_no_work_bundle
  ```

  Expected: compile error — `trigger_update_for_host` still takes `DispatchContext`.

- [ ] **Step 3: Add `PendingProtectionWork` struct and update `TriggerUpdateResult`**

  Replace the existing `TriggerUpdateResult` struct (lines 36-42) with:

  ```rust
  /// All data the orchestrator needs to run protection and dispatch.
  ///
  /// Returned by [`trigger_update_for_host`] for the `Pending` case.
  /// The caller must pass this to `update_orchestrator::spawn_protection_and_dispatch`.
  pub struct PendingProtectionWork {
      pub target: ValidatedUpdateTarget,
      pub update_history_id: Uuid,
      pub to_version: String,
      pub release_info: Option<uptrakit_internal_wire::ReleaseInfo>,
      /// Fully resolved interactive flag (incl. `prefer_interactive` from plugin config).
      pub interactive: bool,
  }

  /// Result returned by a successful [`trigger_update_for_host`] call.
  pub struct TriggerUpdateResult {
      /// The newly-created `update_history` record ID.
      pub update_history_id: Uuid,
      /// The initial status of the record. `Queued` when the host already had
      /// an active update at dispatch time; `Pending` otherwise.
      pub initial_status: update_history::UpdateStatus,
      /// Present when `initial_status == Pending`. The caller must spawn
      /// `update_orchestrator::spawn_protection_and_dispatch` with this bundle.
      /// `None` when `initial_status == Queued` (host busy — no dispatch needed now).
      pub pending_protection_work: Option<Box<PendingProtectionWork>>,
  }
  ```

- [ ] **Step 4: Refactor `trigger_update_for_host`**

  Remove `DispatchContext` from the signature. Replace the existing function signature and body at lines 92-205:

  ```rust
  #[tracing::instrument(skip_all)]
  pub async fn trigger_update_for_host(
      db: &sea_orm::DatabaseConnection,
      params: TriggerUpdateParams<'_>,
  ) -> super::update_dispatch::Result<TriggerUpdateResult> {
      let target =
          validate_update_preconditions(db, params.tenant_id, params.host_id, params.item_id).await?;

      let execute_update_plugin = build_plugin_assignment(
          &target.execute_update_data.0,
          target.execute_update_data.1.as_ref(),
      )?;
      let resolved_interactive = params.interactive
          || config_prefers_interactive(
              &execute_update_plugin.plugin_type,
              &execute_update_plugin.config,
          );

      let build_record = |initial_status| CreateUpdateRecordParams {
          tenant_id: params.tenant_id,
          host_id: params.host_id,
          item_id: params.item_id,
          host_software_item_id: Some(target.hsi_link.id),
          to_version: &params.to_version,
          from_version: target.hsi_link.installed_version.clone(),
          actor_type: params.actor_type,
          actor_id: params.actor_id,
          update_category: &target.hsi_link.update_category,
          batch_id: None,
          initial_status,
          interactive: resolved_interactive,
      };

      let host_busy = has_active_update_for_host(db, params.host_id).await?;

      if host_busy {
          let update_history_id =
              create_update_history_record(db, &build_record(update_history::UpdateStatus::Queued))
                  .await?;
          tracing::info!(
              update_id = %update_history_id,
              host_id = %params.host_id,
              "host has an active update — new update queued"
          );
          return Ok(TriggerUpdateResult {
              update_history_id,
              initial_status: update_history::UpdateStatus::Queued,
              pending_protection_work: None,
          });
      }

      let pending_insert =
          create_update_history_record(db, &build_record(update_history::UpdateStatus::Pending))
              .await;

      let (update_history_id, initial_status) = match pending_insert {
          Ok(id) => (id, update_history::UpdateStatus::Pending),
          Err(e) if is_unique_constraint_violation(&e) => {
              tracing::debug!(
                  host_id = %params.host_id,
                  "concurrent Pending INSERT detected (unique constraint); re-inserting as Queued"
              );
              let id = create_update_history_record(
                  db,
                  &build_record(update_history::UpdateStatus::Queued),
              )
              .await?;
              (id, update_history::UpdateStatus::Queued)
          }
          Err(e) => return Err(e),
      };

      let pending_protection_work = if matches!(initial_status, update_history::UpdateStatus::Pending) {
          Some(Box::new(PendingProtectionWork {
              target,
              update_history_id,
              to_version: params.to_version,
              release_info: params.release_info,
              interactive: resolved_interactive,
          }))
      } else {
          None
      };

      Ok(TriggerUpdateResult {
          update_history_id,
          initial_status,
          pending_protection_work,
      })
  }
  ```

  Update the imports at the top of the file — remove `DispatchContext`, `DispatchUpdateParams`,
  `PreUpdateProtectionOutcome`, `prepare_pre_update_protection`, `dispatch_update_to_agent` from
  the `use` block (they are no longer called here). **Keep `ValidatedUpdateTarget`** — it is the
  type of `PendingProtectionWork.target` and must remain in scope:

  ```rust
  use super::update_dispatch::{
      CreateUpdateRecordParams, TriggerUpdateError, ValidatedUpdateTarget,
      build_plugin_assignment, config_prefers_interactive,
      create_update_history_record, has_active_update_for_host,
      validate_update_preconditions,
  };
  ```

  The test file uses `insert_base_fixture` — tests that called `trigger_update_for_host` with
  `DispatchContext` need updating to the new signature. Remove the `DispatchContext` arg from
  all existing test calls:

  - `trigger_update_queued_when_host_busy` (line 691): remove
    `DispatchContext { notifier: &NoopNotifier, protection: None }` arg
  - `trigger_update_pending_when_host_free` (line 733): same
  - `trigger_update_protection_failure_marks_failed_and_returns_err` (line 766): this test used
    protection to fail inline — **remove this test** since protection no longer runs inline in
    `trigger_update_for_host`. The behavior it tested (protection failure → record Failed) is now
    the orchestrator's responsibility, not `trigger_update_for_host`'s.

- [ ] **Step 5: Run tests**

  ```bash
  cargo test -p uptrakit-web-api-queries
  ```

  Expected: all tests pass, including the two new ones and the updated old ones.

- [ ] **Step 6: Commit**

  ```text
  feat(web-api-queries): refactor trigger_update_for_host to return PendingProtectionWork
  ```

---

## Task 6: Orchestrator-InProgress CAS + reconnect recovery in `update_batches/dispatch.rs`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`

- [ ] **Step 1: Write failing tests**

  In the `tests` module of `update_batches/dispatch.rs`, add:

  ```rust
  async fn insert_orchestrator_inprogress_record(
      db: &DatabaseConnection,
      f: &Fixture,
  ) -> update_history::Model {
      let now = OffsetDateTime::now_utc();
      update_history::ActiveModel {
          id: Set(Uuid::now_v7()),
          tenant_id: Set(f.tenant_id),
          host_id: Set(f.host_id),
          software_item_id: Set(f.item_id),
          host_software_item_id: Set(None),
          from_version: Set(Some("1.0.0".to_string())),
          to_version: Set(Some("1.1.0".to_string())),
          status: Set(update_history::UpdateStatus::InProgress),
          output: Set(String::new()),
          output_bytes: Set(0),
          actor_type: Set("user".to_string()),
          actor_id: Set(String::new()),
          // orchestrator sentinel: owner is NULL
          execution_owner_service_id: Set(None),
          execution_owner_instance_id: Set(None),
          started_at: Set(Some(now)),
          completed_at: Set(None),
          created_at: Set(now),
          update_category: Set("security".to_string()),
          batch_id: Set(None),
          interactive: Set(false),
          output_truncated: Set(false),
          pre_update_protection_status: Set(Some("protected".to_string())),
          pre_update_protection_summary: Set(None),
          recovery_hint: Set(None),
      }
      .insert(db)
      .await
      .unwrap()
  }

  #[tokio::test]
  async fn claim_start_orchestrator_inprogress_is_claimed_by_agent() {
      let db = setup_db().await;
      let f = insert_base_fixture(&db).await;
      let record = insert_orchestrator_inprogress_record(&db, &f).await;
      let service_id = f.service_id;
      let instance_id = Uuid::now_v7();

      let outcome = claim_or_replay_update_start_db(
          &db,
          record.id,
          service_id,
          Some(instance_id),
          true,
      )
      .await
      .unwrap();

      assert!(
          matches!(outcome, ClaimExecutionOutcome::Claimed(_)),
          "orchestrator-InProgress record must be Claimed by the confirming agent"
      );
      let row = UpdateHistory::find_by_id(record.id)
          .one(&db)
          .await
          .unwrap()
          .unwrap();
      assert_eq!(row.execution_owner_service_id, Some(service_id));
      assert_eq!(row.execution_owner_instance_id, Some(instance_id));
      assert!(row.interactive, "interactive must be updated to agent's value");
  }

  #[tokio::test]
  async fn claim_start_orchestrator_inprogress_race_returns_rejected() {
      // Simulate two agents racing to claim the same orchestrator-owned record.
      let db = setup_db().await;
      let f = insert_base_fixture(&db).await;
      let record = insert_orchestrator_inprogress_record(&db, &f).await;

      // First agent claims it directly (simulating a concurrent claim).
      UpdateHistory::update_many()
          .filter(update_history::Column::Id.eq(record.id))
          .col_expr(
              update_history::Column::ExecutionOwnerServiceId,
              Expr::value(Some(Uuid::now_v7())),
          )
          .exec(&db)
          .await
          .unwrap();

      // Second agent's claim must lose the CAS.
      let outcome = claim_or_replay_update_start_db(
          &db,
          record.id,
          f.service_id,
          Some(Uuid::now_v7()),
          false,
      )
      .await
      .unwrap();

      assert!(matches!(outcome, ClaimExecutionOutcome::Rejected));
  }

  #[tokio::test]
  async fn claim_start_orchestrator_inprogress_preserves_output_lines() {
      // Protection output lines must NOT be deleted when the agent claims.
      let db = setup_db().await;
      let f = insert_base_fixture(&db).await;
      let record = insert_orchestrator_inprogress_record(&db, &f).await;
      seed_update_output_line(&db, record.id, "snapshot started\n").await;

      claim_or_replay_update_start_db(
          &db,
          record.id,
          f.service_id,
          Some(Uuid::now_v7()),
          false,
      )
      .await
      .unwrap();

      let line_count = UpdateOutputLine::find()
          .filter(update_output_line::Column::UpdateHistoryId.eq(record.id))
          .count(&db)
          .await
          .unwrap();
      assert_eq!(line_count, 1, "protection output lines must be preserved on agent claim");
  }

  #[tokio::test]
  async fn mark_orchestrator_inprogress_as_failed_marks_only_null_owner_rows() {
      let db = setup_db().await;
      let f = insert_base_fixture(&db).await;
      let orchestrator_row = insert_orchestrator_inprogress_record(&db, &f).await;
      // Also insert an agent-owned row — must not be touched.
      let agent_row = insert_owned_in_progress_record(&db, &f, f.service_id, Some(Uuid::now_v7())).await;

      mark_orchestrator_inprogress_as_failed_on_reconnect(&db, &[f.host_id])
          .await
          .unwrap();

      let orch = UpdateHistory::find_by_id(orchestrator_row.id)
          .one(&db).await.unwrap().unwrap();
      assert_eq!(orch.status, update_history::UpdateStatus::Failed);
      assert_eq!(orch.pre_update_protection_status.as_deref(), Some("failed"));
      assert!(orch.completed_at.is_some());

      let agent = UpdateHistory::find_by_id(agent_row.id)
          .one(&db).await.unwrap().unwrap();
      assert_eq!(agent.status, update_history::UpdateStatus::InProgress, "agent-owned row must be untouched");
  }

  #[tokio::test]
  async fn mark_orchestrator_inprogress_as_failed_ignores_empty_host_list() {
      let db = setup_db().await;
      let f = insert_base_fixture(&db).await;
      let row = insert_orchestrator_inprogress_record(&db, &f).await;

      mark_orchestrator_inprogress_as_failed_on_reconnect(&db, &[])
          .await
          .unwrap();

      let status = UpdateHistory::find_by_id(row.id)
          .one(&db).await.unwrap().unwrap().status;
      assert_eq!(status, update_history::UpdateStatus::InProgress, "empty host list must be a no-op");
  }
  ```

- [ ] **Step 2: Run tests to verify they fail**

  ```bash
  cargo test -p uptrakit-web-api-queries claim_start_orchestrator mark_orchestrator_orchestrator
  ```

  Expected: compile errors — `mark_orchestrator_inprogress_as_failed_on_reconnect` not found, new `claim_or_replay` case missing.

- [ ] **Step 3: Add orchestrator-InProgress CAS case to `claim_or_replay_update_start_db`**

  In `claim_or_replay_update_start_db`, after the existing Pending CAS block (after line 603,
  before the `if record.status == InProgress && record.execution_owner_service_id == Some(service_id)`
  check), add:

  ```rust
  // Orchestrator-owned InProgress: agent confirms an update whose record was
  // already transitioned by the orchestrator. Claim ownership atomically.
  if record.status == update_history::UpdateStatus::InProgress
      && record.execution_owner_service_id.is_none()
  {
      let txn = db.begin().await.context_to()?;
      let claimed = UpdateHistory::update_many()
          .filter(update_history::Column::Id.eq(record.id))
          .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
          .filter(update_history::Column::ExecutionOwnerServiceId.is_null()) // CAS guard
          .col_expr(
              update_history::Column::ExecutionOwnerServiceId,
              Expr::value(Some(service_id)),
          )
          .col_expr(
              update_history::Column::ExecutionOwnerInstanceId,
              Expr::value(runtime_instance_id),
          )
          .col_expr(
              update_history::Column::Interactive,
              Expr::value(interactive),
          )
          .exec(&txn)
          .await
          .context_to()?;

      if claimed.rows_affected == 0 {
          txn.rollback().await.context_to()?;
          return Ok(ClaimExecutionOutcome::Rejected);
      }

      txn.commit().await.context_to()?;
      // NOTE: No UpdateOutputLine::delete_many() — protection output lines are kept.
      // NOTE: started_at is NOT reset — it was set by set_inprogress_for_orchestrator.
      return Ok(ClaimExecutionOutcome::Claimed(claim_execution_info(&record)));
  }
  ```

- [ ] **Step 4: Add `mark_orchestrator_inprogress_as_failed_on_reconnect`**

  After `mark_all_in_progress_as_failed_for_rollout`, add:

  ```rust
  /// Fail all orchestrator-owned InProgress records for the given hosts on agent reconnect.
  ///
  /// Orchestrator-owned means `execution_owner_service_id IS NULL` + `status = InProgress`.
  /// These records were mid-protection or mid-dispatch when the controller restarted.
  /// The user must re-trigger; Proxmox protection will re-run.
  ///
  /// Called after `mark_owned_in_progress_as_failed_on_reconnect` so agent-owned rows
  /// are handled first.
  pub async fn mark_orchestrator_inprogress_as_failed_on_reconnect(
      db: &DatabaseConnection,
      host_ids: &[Uuid],
  ) -> std::result::Result<(), rootcause::Report<TriggerUpdateError>> {
      if host_ids.is_empty() {
          return Ok(());
      }
      let now = OffsetDateTime::now_utc();
      let reason = "Protection interrupted: controller restarted";
      UpdateHistory::update_many()
          .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
          .filter(update_history::Column::ExecutionOwnerServiceId.is_null())
          .filter(update_history::Column::HostId.is_in(host_ids.to_vec()))
          .col_expr(
              update_history::Column::Status,
              Expr::value(update_history::UpdateStatus::Failed),
          )
          .col_expr(update_history::Column::CompletedAt, Expr::value(Some(now)))
          .col_expr(
              update_history::Column::Output,
              Expr::value(reason.to_string()),
          )
          .col_expr(
              update_history::Column::OutputBytes,
              Expr::value(reason.len() as i64),
          )
          .col_expr(update_history::Column::OutputTruncated, Expr::value(false))
          .col_expr(
              update_history::Column::PreUpdateProtectionStatus,
              Expr::value(Some("failed".to_string())),
          )
          .exec(db)
          .await
          .context_to()?;
      Ok(())
  }
  ```

- [ ] **Step 5: Re-export from `update_batches/mod.rs`**

  In `crates/ui/web-api-queries/src/queries/update_batches/mod.rs`, add
  `mark_orchestrator_inprogress_as_failed_on_reconnect` to the existing `pub use dispatch::{...}`
  block (line 17):

  ```rust
  pub use dispatch::{
      BatchCompletionInfo, ClaimExecutionInfo, ClaimExecutionOutcome, FinalizeBatchItemIfOwnedArgs,
      FinalizeUpdateResultIfOwnedArgs, append_update_output_if_owned,
      claim_or_replay_update_start_db, dispatch_next_in_batch, dispatch_next_queued_for_host,
      fail_pending_unowned_update, finalize_batch_item_if_owned, finalize_update_result_if_owned,
      mark_all_in_progress_as_failed_for_rollout, mark_orchestrator_inprogress_as_failed_on_reconnect,
      mark_owned_in_progress_as_failed_on_reconnect,
      touch_stdin_attention_if_owned,
  };
  ```

  Without this, `crate::queries::update_batches::mark_orchestrator_inprogress_as_failed_on_reconnect` in `web-api` will fail to resolve.

- [ ] **Step 6: Run tests**

  ```bash
  cargo test -p uptrakit-web-api-queries claim_start_orchestrator mark_orchestrator
  ```

  Expected: all pass.

- [ ] **Step 7: Commit**

  ```text
  feat(web-api-queries): add orchestrator CAS case and reconnect recovery
  ```

---

## Task 7: Create `update_orchestrator.rs` and register in `lib.rs`

**Files:**

- Create: `crates/ui/web-api/src/update_orchestrator.rs`
- Modify: `crates/ui/web-api/src/lib.rs`

- [ ] **Step 1: Register the module in `lib.rs`**

  In `crates/ui/web-api/src/lib.rs`, add the module declaration (alongside other `pub(crate) mod` declarations):

  ```rust
  pub(crate) mod update_orchestrator;
  ```

- [ ] **Step 2: Create `update_orchestrator.rs`**

  Create `crates/ui/web-api/src/update_orchestrator.rs` with:

  ```rust
  //! Background orchestration of pre-update protection and agent dispatch.
  //!
  //! [`spawn_protection_and_dispatch`] is the single public entry point.
  //! It spawns a tokio task that:
  //!
  //! 1. Checks agent connectivity.
  //! 2. Transitions the `update_history` record to `InProgress` (CAS).
  //! 3. Creates the broadcast channel.
  //! 4. Pushes MQTT states.
  //! 5. Emits [`AdminEvent::UpdateProtectionStarted`].
  //! 6. Creates an mpsc channel for protection output.
  //! 7. Spawns a forwarder that persists and broadcasts each output line.
  //! 8. Runs pre-update protection.
  //! 9. On success, dispatches to the agent.

  use std::sync::Arc;

  use time::OffsetDateTime;
  use tokio::sync::mpsc;
  use uptrakit_internal_wire::OutputStreamType;
  use uptrakit_shared_types::UpdateStatus;
  use uptrakit_web_api_types::AdminEvent;
  use uuid::Uuid;

  use crate::AppState;
  use crate::queries::update_batches::mark_orchestrator_inprogress_as_failed_on_reconnect;
  use crate::queries::update_dispatch::{
      DispatchUpdateParams, fail_before_agent_dispatch, insert_protection_output_line,
      prepare_pre_update_protection, set_inprogress_for_orchestrator,
      dispatch_update_to_agent,
  };
  use crate::queries::update_triggers::PendingProtectionWork;

  /// Spawn a background task to run pre-update protection then dispatch.
  ///
  /// Returns immediately. The task handles its own error logging.
  pub fn spawn_protection_and_dispatch(state: Arc<AppState>, work: PendingProtectionWork) {
      tokio::spawn(run_protection_and_dispatch(state, work));
  }

  async fn run_protection_and_dispatch(state: Arc<AppState>, work: PendingProtectionWork) {
      let db = state.db();
      let update_history_id = work.update_history_id;
      let host_id = work.target.host.id;
      let software_item_id = work.target.item.id;
      let tenant_id = work.target.item.tenant_id;

      // Step 1: Check agent connectivity. If offline, record stays Pending;
      // reconnect recovery will spawn the orchestrator when the agent comes back.
      if !state
          .service_connections
          .is_connected(&work.target.agent.id)
          .await
      {
          tracing::debug!(
              update_id = %update_history_id,
              agent_id = %work.target.agent.id,
              "agent offline at orchestrator start — leaving record Pending for reconnect"
          );
          return;
      }

      // Step 2: CAS Pending → InProgress (orchestrator sentinel: owner = NULL).
      let rows = match set_inprogress_for_orchestrator(db, update_history_id).await {
          Ok(rows) => rows,
          Err(e) => {
              tracing::warn!(
                  update_id = %update_history_id,
                  error = %e,
                  "failed to transition update to InProgress in orchestrator"
              );
              return;
          }
      };
      if rows == 0 {
          tracing::debug!(
              update_id = %update_history_id,
              "CAS missed — record already gone or raced; orchestrator exiting"
          );
          return;
      }

      // Step 3: Create broadcast channel (after CAS to avoid leaking channel on race).
      state
          .broadcast
          .update_output_broadcaster
          .create_channel(update_history_id)
          .await;

      // Step 4: Push MQTT states so connected agents see the host as in-progress.
      state
          .notification
          .notification_service
          .push_software_states_for_tenant(db, tenant_id)
          .await;

      // Step 5: Emit AdminEvent so frontend transitions to In Progress state.
      state
          .broadcast
          .event_broadcaster
          .send(
              tenant_id,
              AdminEvent::UpdateProtectionStarted {
                  update_history_id,
                  host_id,
                  software_item_id,
              },
          )
          .await;

      // Step 6: Output channel for streaming protection lines.
      let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();

      // Step 7: Spawn forwarder.
      let forward_db = db.clone();
      let forward_broadcaster = state.broadcast.update_output_broadcaster.clone();
      tokio::spawn(forward_protection_output(
          forward_db,
          forward_broadcaster,
          update_history_id,
          rx,
      ));

      // Step 8: Run protection.
      let protection = state.controller_update_protection();
      let outcome = match prepare_pre_update_protection(
          db,
          protection,
          &work.target,
          update_history_id,
          Some(tx),
      )
      .await
      {
          Ok(outcome) => outcome,
          Err(e) => {
              tracing::warn!(
                  update_id = %update_history_id,
                  error = %e,
                  "prepare_pre_update_protection returned error in orchestrator"
              );
              state
                  .notification
                  .notification_service
                  .push_software_states_for_tenant(db, tenant_id)
                  .await;
              return;
          }
      };

      // Step 9: Match outcome.
      match outcome {
          crate::queries::update_dispatch::PreUpdateProtectionOutcome::Failed => {
              // Record already set to Failed by fail_before_agent_dispatch inside
              // prepare_pre_update_protection.
              state
                  .notification
                  .notification_service
                  .push_software_states_for_tenant(db, tenant_id)
                  .await;
          }
          crate::queries::update_dispatch::PreUpdateProtectionOutcome::Proceed => {
              let notifier = &*state.notification.notification_service;
              let dispatch_result = dispatch_update_to_agent(
                  notifier,
                  &work.target,
                  DispatchUpdateParams {
                      update_history_id,
                      to_version: work.to_version,
                      release_info: work.release_info,
                      interactive: work.interactive,
                  },
              )
              .await;

              match dispatch_result {
                  Err(e) => {
                      tracing::warn!(
                          update_id = %update_history_id,
                          error = %e,
                          "dispatch_update_to_agent failed in orchestrator; marking update failed"
                      );
                      if let Err(fail_err) =
                          fail_before_agent_dispatch(db, update_history_id, None).await
                      {
                          tracing::warn!(
                              update_id = %update_history_id,
                              error = %fail_err,
                              "fail_before_agent_dispatch also failed"
                          );
                      }
                  }
                  Ok(false) => {
                      // Agent disconnected between protection and dispatch.
                      // Record stays InProgress with owner=NULL; reconnect recovery will
                      // mark it Failed on next connect (crash-recovery path).
                      tracing::debug!(
                          update_id = %update_history_id,
                          "dispatch_update_to_agent returned false (agent offline); leaving for reconnect recovery"
                      );
                  }
                  Ok(true) => {}
              }

              state
                  .notification
                  .notification_service
                  .push_software_states_for_tenant(db, tenant_id)
                  .await;
          }
      }
  }

  /// Forward protection output lines from the mpsc channel to DB + broadcaster.
  ///
  /// Runs as a detached tokio task. Exits when the sender is dropped (protection done).
  async fn forward_protection_output(
      db: sea_orm::DatabaseConnection,
      broadcaster: crate::update_output_broadcaster::UpdateOutputBroadcaster,
      update_history_id: Uuid,
      mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
  ) {
      while let Some(raw) = rx.recv().await {
          let text = String::from_utf8_lossy(&raw).into_owned();
          let line_id = Uuid::now_v7();
          let timestamp = OffsetDateTime::now_utc();

          if let Err(e) = insert_protection_output_line(
              &db,
              update_history_id,
              line_id,
              text.clone(),
              OutputStreamType::Stdout,
              timestamp,
          )
          .await
          {
              tracing::warn!(
                  update_id = %update_history_id,
                  error = %e,
                  "failed to persist protection output line"
              );
          }

          broadcaster
              .send_line(update_history_id, line_id, text, OutputStreamType::Stdout, timestamp)
              .await;
      }
  }
  ```

  Note: `state.service_connections.is_connected` — confirmed async (`pub async fn` at
  `service_connections.rs:255`). The `.await` is required (already in the code above).

  Note: `state.notification.notification_service` is `Arc<dyn NotificationOps>`.
  `push_software_states_for_tenant` is an async method on `NotificationOps`. Verify the trait
  has this method or use `state.controller_update_protection()` pattern.

  Note: `state.broadcast.event_broadcaster.send(tenant_id, event)` — check existing usages in
  `updates.rs` for the exact call pattern. Mirror those exactly.

- [ ] **Step 3: Verify compilation**

  ```bash
  cargo check -p uptrakit-web-api --all-features 2>&1 | head -40
  ```

  Fix any import errors by checking existing usages in `web-api` for `event_broadcaster.send`,
  `push_software_states_for_tenant`, and `service_connections.is_connected`. All in `updates.rs`:

  ```bash
  grep -n "event_broadcaster\|push_software_states\|is_connected" \
    crates/ui/web-api/src/routes/service_ws/handler/updates.rs | head -20
  ```

  Mirror the exact call patterns found there.

- [ ] **Step 4: Commit**

  ```text
  feat(web-api): add update_orchestrator module
  ```

---

## Task 8: Wire call sites — REST trigger, WS trigger, `actions::trigger_update`

**Files:**

- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs` (around line 1195)
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs` (around line 140)
- Modify: `crates/ui/web-api/src/actions/software_items.rs`

- [ ] **Step 1: Drop `protection` from `actions::trigger_update`**

  In `actions/software_items.rs`, find `trigger_update` function. It currently takes a
  `protection: Option<Arc<dyn ControllerUpdateProtection>>` parameter and passes it in
  `DispatchContext`. Remove the `protection` parameter and the `DispatchContext` construction.
  Change the call to `trigger_update_for_host` to the new signature:

  ```rust
  // Before:
  crate::queries::trigger_update_for_host(
      db,
      DispatchContext { notifier, protection },
      params,
  )
  .await

  // After:
  crate::queries::trigger_update_for_host(db, params).await
  ```

  Also remove the `protection` parameter from the function signature entirely and any callers that pass it.

- [ ] **Step 2: Update REST trigger in `routes/software_items/mod.rs`**

  Find the `trigger_update` call site (around line 1252). After calling
  `item_actions::trigger_update`, spawn the orchestrator if work is present:

  ```rust
  let result = item_actions::trigger_update(&tenant_db, &ctx, params).await?;
  if let Some(work) = result.pending_protection_work {
      crate::update_orchestrator::spawn_protection_and_dispatch(Arc::clone(&state), *work);
  }
  ```

  The `state` variable is available via `State(state): State<Arc<AppState>>` in the handler.

- [ ] **Step 3: Update WS trigger in `handler/update_tracking.rs`**

  Find the `trigger_update_for_host` or `trigger_update` call (around line 140). After the call, spawn the orchestrator:

  ```rust
  let result = /* existing trigger call */;
  if let Some(work) = result.pending_protection_work {
      crate::update_orchestrator::spawn_protection_and_dispatch(Arc::clone(state), *work);
  }
  ```

- [ ] **Step 4: Verify compilation**

  ```bash
  cargo check -p uptrakit-web-api --all-features 2>&1 | head -40
  ```

  Fix remaining compilation errors. The most common: callers of `trigger_update` that passed a
  `protection` argument — update those to omit the argument.

- [ ] **Step 5: Commit**

  ```text
  feat(web-api): wire orchestrator spawn at REST and WS trigger call sites
  ```

---

## Task 9: `handle_update_started`, reconnect recovery, and `dispatch_next_queued_update_with_notifier`

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`

This task has three independent sub-changes in the same file.

### Sub-change A: `handle_update_started` Claimed arm

- [ ] **Step 1: Change `create_channel` to `get_or_create_channel` in the Claimed arm**

  In `handle_update_started` (around line 1003-1015), change:

  ```rust
  crate::queries::update_batches::ClaimExecutionOutcome::Claimed(info) => {
      let info = UpdateStartedInfo { ... };
      state
          .broadcast
          .update_output_broadcaster
          .create_channel(payload.update_history_id)
          .await;
      broadcast_update_started_events(state, service_id, payload, &info).await;
  }
  ```

  to:

  ```rust
  crate::queries::update_batches::ClaimExecutionOutcome::Claimed(info) => {
      let info = UpdateStartedInfo {
          batch_id: info.batch_id,
          host_id: info.host_id,
          software_item_id: info.software_item_id,
          tenant_id: info.tenant_id,
      };
      // Use get_or_create_channel (not create_channel) to preserve any existing
      // subscriber connections from the orchestrator's protection-output phase.
      state
          .broadcast
          .update_output_broadcaster
          .get_or_create_channel(payload.update_history_id)
          .await;
      broadcast_update_started_events(state, service_id, payload, &info).await;
  }
  ```

### Sub-change B: Orchestrator InProgress reconnect recovery

- [ ] **Step 2: Add `mark_orchestrator_inprogress_as_failed_on_reconnect` call in reconnect recovery**

  The reconnect recovery function is `recover_owned_updates_on_connect_with_dispatch_mode`
  (line 495 in `updates.rs`). It takes `service_id` but does not pre-compute a host ID list.

  `load_linked_host_ids` is already imported at line 15 via
  `use super::shared_types::{ProcessorResponse, load_linked_host_ids}`.
  Load host IDs after the existing `mark_owned_in_progress_as_failed_on_reconnect` call
  and then call the new function:

  ```rust
  // Fail orchestrator-owned InProgress records (protection was mid-run when controller restarted).
  let linked_host_ids_for_orchestrator: Vec<uuid::Uuid> =
      load_linked_host_ids(state.db(), service_id)
          .await
          .unwrap_or_default();
  if let Err(e) = crate::queries::update_batches::mark_orchestrator_inprogress_as_failed_on_reconnect(
      state.db(),
      &linked_host_ids_for_orchestrator,
  )
  .await
  {
      tracing::warn!(
          %service_id,
          error = %e,
          "failed to mark orchestrator-owned InProgress records as Failed on reconnect"
      );
  }
  ```

### Sub-change C: `prepare_pending_replay_messages` — spawn orchestrator for unprotected Pending

- [ ] **Step 3: Promote `load_target_for_dispatch` to `pub` in `update_dispatch.rs`**

  `load_target_for_dispatch` is currently `pub(crate)` in `web-api-queries`. It will be called
  from `web-api` (a different crate) in steps below. Change its visibility:

  In `crates/ui/web-api-queries/src/queries/update_dispatch.rs`, find
  `pub(crate) async fn load_target_for_dispatch` and change to:

  ```rust
  pub async fn load_target_for_dispatch(
  ```

- [ ] **Step 4: Update `prepare_pending_replay_messages`**

  In `prepare_pending_replay_messages` (line 547), inside the loop over `records.pending_updates`,
  add a check before `build_execute_payload`:

  ```rust
  for update_record in &records.pending_updates {
      // Records with pre_update_protection_status = NULL have not had protection run.
      // Spawn the orchestrator instead of replaying directly.
      if update_record.pre_update_protection_status.is_none() {
          // Reconstruct a work bundle from the DB record.
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
                      "could not load target for unprotected Pending update on reconnect; skipping"
                  );
                  continue;
              }
          };
          let work = crate::queries::update_triggers::PendingProtectionWork {
              target,
              update_history_id: update_record.id,
              to_version: update_record.to_version.clone().unwrap_or_default(),
              release_info: None, // Release info not preserved in update_history; agent handles version resolution.
              interactive: update_record.interactive,
          };
          crate::update_orchestrator::spawn_protection_and_dispatch(Arc::clone(state), work);
          continue;
      }

      // Existing logic for records that already have protection status set.
      if let Some(batch_id) = update_record.batch_id {
          // ... existing batch dedup logic
      }
      // ... rest of existing loop body
  }
  ```

  `load_target_for_dispatch` was promoted to `pub` in Step 3 above. Its signature:

  ```rust
  pub async fn load_target_for_dispatch(
      db: &DatabaseConnection,
      tenant_id: Uuid,
      host_id: Uuid,
      software_item_id: Uuid,
  ) -> Result<ValidatedUpdateTarget>
  ```

### Sub-change D: Refactor `dispatch_next_queued_update_with_notifier`

- [ ] **Step 5: Drop `notifier` and `protection` params, call orchestrator**

  Find `dispatch_next_queued_update_with_notifier` (line 1824). Replace the function with:

  ```rust
  async fn dispatch_next_queued_update_with_notifier(
      state: &Arc<AppState>,
      service_id: uuid::Uuid,
      host_id: uuid::Uuid,
  ) {
      let tenant_id = match service::Entity::find_by_id(service_id)
          .one(state.db())
          .await
      {
          Ok(Some(svc)) => svc.tenant_id,
          _ => return,
      };

      // Find the next Queued update for this host and promote it via the orchestrator.
      let next = match uptrakit_shared_db::entity::update_history::Entity::find()
          .filter(
              uptrakit_shared_db::entity::update_history::Column::HostId.eq(host_id),
          )
          .filter(
              uptrakit_shared_db::entity::update_history::Column::Status
                  .eq(uptrakit_shared_db::entity::update_history::UpdateStatus::Queued),
          )
          .order_by_asc(uptrakit_shared_db::entity::update_history::Column::Id)
          .one(state.db())
          .await
      {
          Ok(Some(record)) => record,
          Ok(None) => return,
          Err(e) => {
              tracing::warn!(
                  %host_id,
                  error = %e,
                  "failed to find next queued update for host"
              );
              return;
          }
      };

      // CAS: Queued → Pending.
      let cas_result = uptrakit_shared_db::entity::update_history::Entity::update_many()
          .filter(
              uptrakit_shared_db::entity::update_history::Column::Id.eq(next.id),
          )
          .filter(
              uptrakit_shared_db::entity::update_history::Column::Status
                  .eq(uptrakit_shared_db::entity::update_history::UpdateStatus::Queued),
          )
          .col_expr(
              uptrakit_shared_db::entity::update_history::Column::Status,
              sea_orm::Expr::value(
                  uptrakit_shared_db::entity::update_history::UpdateStatus::Pending,
              ),
          )
          .exec(state.db())
          .await;

      match cas_result {
          Ok(r) if r.rows_affected == 0 => return, // raced
          Err(e) => {
              tracing::warn!(%host_id, error = %e, "CAS failed promoting queued update");
              return;
          }
          _ => {}
      }

      // Load target and spawn orchestrator.
      let target = match crate::queries::update_dispatch::load_target_for_dispatch(
          state.db(),
          tenant_id,
          next.host_id,
          next.software_item_id,
      )
      .await
      {
          Ok(t) => t,
          Err(e) => {
              tracing::warn!(
                  update_id = %next.id,
                  error = %e,
                  "failed to load target for promoted queued update"
              );
              return;
          }
      };

      let work = crate::queries::update_triggers::PendingProtectionWork {
          target,
          update_history_id: next.id,
          to_version: next.to_version.unwrap_or_default(),
          release_info: None,
          interactive: next.interactive,
      };
      crate::update_orchestrator::spawn_protection_and_dispatch(Arc::clone(state), work);
  }
  ```

  Update the two callers (`dispatch_next_queued_update` and
  `dispatch_next_queued_update_for_replay`) to drop the `notifier` and `protection` args:

  ```rust
  // Before:
  dispatch_next_queued_update_with_notifier(
      state,
      service_id,
      host_id,
      &*state.notification.notification_service,
      state.controller_update_protection(),
  )
  .await;

  // After:
  dispatch_next_queued_update_with_notifier(state, service_id, host_id).await;
  ```

  If `ReplayPreparationNotifier` is now unused in this path, Rust will warn. Remove its use from
  this call site. If `ReplayPreparationNotifier` has no other uses, delete the struct entirely
  (`#[allow(dead_code)]` is not acceptable).

- [ ] **Step 6: Verify compilation**

  ```bash
  cargo check -p uptrakit-web-api --all-features 2>&1 | head -50
  ```

  Fix any remaining errors. Common issue: imports for `update_history` entities in the new function body.

- [ ] **Step 7: Run all tests**

  ```bash
  cargo test --all-features 2>&1 | tail -30
  ```

  Expected: all tests pass.

- [ ] **Step 8: Commit**

  ```text
  feat(web-api): wire reconnect recovery, pending replay, and queued promotion via orchestrator
  ```

---

## Task 10: Full quality gate

- [ ] **Step 1: Format**

  ```bash
  cargo fmt --all
  ```

- [ ] **Step 2: Check no-default-features with SQLite**

  ```bash
  cargo check --no-default-features --features db-sqlite 2>&1 | tail -20
  ```

  Expected: no errors.

- [ ] **Step 3: Check all-features**

  ```bash
  cargo check --all-features 2>&1 | tail -20
  ```

- [ ] **Step 4: Clippy (SQLite)**

  ```bash
  cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | tail -30
  ```

- [ ] **Step 5: Clippy (all-features)**

  ```bash
  cargo clippy --all-targets --all-features 2>&1 | tail -30
  ```

- [ ] **Step 6: Tests**

  ```bash
  cargo test --all-features 2>&1 | tail -30
  ```

  Expected: all pass.

- [ ] **Step 7: Dependency audit**

  ```bash
  cargo deny check
  ```

- [ ] **Step 8: Markdown lint**

  ```bash
  markdownlint --config .markdownlint.json '**/*.md'
  ```

- [ ] **Step 9: Commit**

  ```text
  chore: quality gate pass for background pre-update protection
  ```

---

## Self-Review

Checking spec coverage:

| Spec requirement | Task |
| --- | --- |
| HTTP trigger returns immediately | Task 5 (no inline protection), Task 8 (spawn after return) |
| Protection in background task | Task 7 (`tokio::spawn` in orchestrator) |
| Record transitions Pending→InProgress when protection starts | Task 7 (step 2, `set_inprogress_for_orchestrator`) |
| Every protection output line persisted + broadcast | Task 7 (`forward_protection_output`), Task 3 (`insert_protection_output_line`) |
| Agent dispatch only after protection succeeds | Task 7 (step 9 match) |
| Queued promotion through orchestrator | Task 9 sub-change D |
| Agent offline → stay Pending | Task 7 (step 1, connectivity check) |
| Crash recovery → orchestrator InProgress marked Failed | Task 6 (`mark_orchestrator_inprogress_as_failed_on_reconnect`), Task 9 sub-change B |
| `AdminEvent::UpdateProtectionStarted` | Task 2, Task 7 (emitted at step 5) |
| `claim_or_replay` new InProgress+no_owner CAS | Task 6 |
| `handle_update_started` Claimed → `get_or_create_channel` | Task 9 sub-change A |
| `ControllerProtectionContext` `output_tx` field | Task 1 |
| Proxmox plugin streams lines via `output_tx` | Task 4 |
| `fail_before_agent_dispatch` → `pub` | Task 3 |
| `ValidatedUpdateTarget` → `Clone` | Task 3 |
| `TriggerUpdateResult` + `PendingProtectionWork` types | Task 5 |
| `prepare_pending_replay_messages` Pending+unprotected → orchestrator | Task 9 sub-change C |
| `DispatchContext` stays for batch path | Not changed — `dispatch_next_in_batch` untouched |

All requirements covered.
