# Update Trigger UX — Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add item-level duplicate-trigger prevention, `UpdateStatus` grouping helpers, `active_update_status` on the host summary wire type, and a
`status` field on the `UpdateTriggered` SSE event.

**Architecture:** All changes are backward-compatible additions. The new `(host_id, software_item_id)` partial unique index is a separate constraint
from the existing host-level index — both remain. The `UpdateStatus` helpers centralise previously inlined status arrays. The `UpdateTriggered` SSE
field uses a named serde default so old consumers (no `status` key) receive `"pending"` instead of an empty string.

**Tech Stack:** Rust, SeaORM, sea-orm-migration, serde, tokio

---

## File Map

| File                                                                                  | Action                                                                                                       |
| ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `crates/shared/types/src/update_status.rs`                                            | Add `unfinished()` and `host_blocking()` const fn helpers                                                    |
| `crates/ui/web-api-queries/src/queries/update_dispatch.rs`                            | Add `has_active_update_for_host_software_item`; update `has_active_update_for_host` to use `host_blocking()` |
| `crates/ui/web-api-queries/src/queries/update_triggers.rs`                            | Call new pre-check before host-level check; update re-queue fallback                                         |
| `crates/ui/web-api-queries/src/queries/software_items/mod.rs`                         | Switch status filter to `unfinished()`; carry `(Uuid, String)` in active_updates map                         |
| `crates/ui/web-api-queries/src/queries/software_states.rs`                            | Fix AwaitingRestart omission (lines 139–141, 407–409); switch to `unfinished()`                              |
| `crates/shared/web-api-types/src/software_items.rs`                                   | Add `active_update_status: Option<String>` to `SoftwareItemHostSummary`                                      |
| `crates/ui/web-api-queries/src/queries/software_items/crud.rs`                        | Add `active_update_status: None` to struct literal at line 1092                                              |
| `crates/ui/cli/src/commands/software_items.rs`                                        | Add `active_update_status: None` to struct literals at lines 855, 919                                        |
| `crates/shared/wire/src/admin_events.rs`                                              | Add `status: String` to `UpdateTriggered`; update Inner struct + `all_variants()`                            |
| `crates/shared/wire/asyncapi.yaml`                                                    | Update `update_triggered` example to include `status`                                                        |
| `crates/shared/openapi-client/src/events_stream.rs`                                   | Add `status` to `AdminSseEvent::UpdateTriggered` + Payload struct + test                                     |
| `crates/shared/db/src/migration/m20260515_000002_update_history_item_active_index.rs` | New migration                                                                                                |
| `crates/shared/db/src/migration/mod.rs`                                               | Register new migration                                                                                       |
| `crates/ui/web-api/src/actions/update_batches.rs`                                     | Add `status` to two `UpdateTriggered` emits (lines 88, 331)                                                  |
| `crates/ui/web-api/src/routes/software_items/mod.rs`                                  | Add `UpdateTriggered` emit with `status` after dispatch (line ~1791)                                         |
| `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs`                  | Add `status` to existing `UpdateTriggered` emit (line 165)                                                   |
| `crates/ui/web-api/src/routes/events.rs`                                              | Add `status` to `UpdateTriggered` in test (lines 164–183)                                                    |
| `docs/development/coding-standards.md`                                                | Add UpdateStatus grouping helpers note                                                                       |
| `CONTEXT.md`                                                                          | Add `unfinished()` / `host_blocking()` to UpdateStatus glossary entry                                        |

---

### Task 1: Add `UpdateStatus` grouping helpers

**Files:**

- Modify: `crates/shared/types/src/update_status.rs:54` (after `as_str()` impl)

- [ ] **Step 1.1: Add helpers to the impl block**

  In `crates/shared/types/src/update_status.rs`, inside the existing `impl UpdateStatus` block (after the `as_str` method at line 44), add:

  ```rust
  /// All non-terminal statuses — states where a new trigger for the same
  /// (host, software_item) must be rejected.
  pub const fn unfinished() -> [Self; 4] {
      [Self::Queued, Self::Pending, Self::InProgress, Self::AwaitingRestart]
  }

  /// Statuses that block the host from running another update concurrently.
  /// Excludes `Queued` — a queued update does not occupy the host's execution
  /// slot.
  pub const fn host_blocking() -> [Self; 3] {
      [Self::Pending, Self::InProgress, Self::AwaitingRestart]
  }
  ```

- [ ] **Step 1.2: Add tests for helpers**

  In the `#[cfg(test)] mod tests` block of `update_status.rs`, add:

  ```rust
  #[test]
  fn unfinished_contains_four_statuses() {
      let u = UpdateStatus::unfinished();
      assert_eq!(u.len(), 4);
      assert!(u.contains(&UpdateStatus::Queued));
      assert!(u.contains(&UpdateStatus::Pending));
      assert!(u.contains(&UpdateStatus::InProgress));
      assert!(u.contains(&UpdateStatus::AwaitingRestart));
      assert!(!u.contains(&UpdateStatus::Completed));
      assert!(!u.contains(&UpdateStatus::Failed));
  }

  #[test]
  fn host_blocking_contains_three_statuses_excludes_queued() {
      let h = UpdateStatus::host_blocking();
      assert_eq!(h.len(), 3);
      assert!(!h.contains(&UpdateStatus::Queued));
      assert!(h.contains(&UpdateStatus::Pending));
      assert!(h.contains(&UpdateStatus::InProgress));
      assert!(h.contains(&UpdateStatus::AwaitingRestart));
  }
  ```

- [ ] **Step 1.3: Run tests**

  ```bash
  cargo test -p uptrakit-shared-types -- update_status 2>&1 | tail -20
  ```

  Expected: all tests pass (including existing serde/display/from_str tests).

- [ ] **Step 1.4: Update `has_active_update_for_host` in `update_dispatch.rs` to use `host_blocking()`**

  In `crates/ui/web-api-queries/src/queries/update_dispatch.rs`, find `has_active_update_for_host` (around line 937). Replace the three-element status
  filter with the helper:

  Before:

  ```rust
  .filter(update_history::Column::Status.is_in([
      update_history::UpdateStatus::Pending,
      update_history::UpdateStatus::InProgress,
      update_history::UpdateStatus::AwaitingRestart,
  ]))
  ```

  After:

  ```rust
  .filter(update_history::Column::Status.is_in(UpdateStatus::host_blocking()))
  ```

  `update_history::UpdateStatus` is a re-export of `uptrakit_shared_types::UpdateStatus` — the import `use uptrakit_shared_types::UpdateStatus;` may
  already be present; if not, add it near the top of the file alongside other `uptrakit_shared_types` imports.

- [ ] **Step 1.5: Fix `software_states.rs` — two callsites missing `AwaitingRestart`**

  In `crates/ui/web-api-queries/src/queries/software_states.rs`, there are two `Condition::any()` blocks that build the active-status filter (around
  lines 128–141 and 400–409) using only `Queued | Pending | InProgress`. Replace both with `.is_in(UpdateStatus::unfinished())`.

  First callsite (around line 132):

  Before:

  ```rust
  let active_updates: HashSet<(Uuid, Uuid)> = UpdateHistory::find()
      ...
      .filter(
          Condition::any()
              .add(update_history::Column::Status.eq(update_history::UpdateStatus::Queued))
              .add(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
              .add(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress)),
      )
  ```

  After:

  ```rust
  let active_updates: HashSet<(Uuid, Uuid)> = UpdateHistory::find()
      ...
      .filter(update_history::Column::Status.is_in(UpdateStatus::unfinished()))
  ```

  Apply the same replacement to the second callsite (around line 400). Both callsites use `Queued` — that's why `unfinished()`, not `host_blocking()`,
  is correct here.

  Add `use uptrakit_shared_types::UpdateStatus;` at the top of the file if not already present.

- [ ] **Step 1.6: Check compilation**

  ```bash
  cargo check -p uptrakit-web-api-queries --no-default-features --features db-sqlite 2>&1 | grep -E "^error" | head -20
  ```

  Expected: no errors.

- [ ] **Step 1.7: Commit**

  ```bash
  git add \
    crates/shared/types/src/update_status.rs \
    crates/ui/web-api-queries/src/queries/update_dispatch.rs \
    crates/ui/web-api-queries/src/queries/software_states.rs
  git commit -m "feat(types): add UpdateStatus::unfinished() and host_blocking() grouping helpers

  Centralises the repeated status-array patterns at callsites. Fixes two
  software_states.rs callsites that were missing AwaitingRestart.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 2: DB migration — `(host_id, software_item_id)` partial unique index

**Files:**

- Create: `crates/shared/db/src/migration/m20260515_000002_update_history_item_active_index.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 2.1: Create the migration file**

  Create `crates/shared/db/src/migration/m20260515_000002_update_history_item_active_index.rs`:

  ```rust
  use sea_orm_migration::prelude::*;

  /// Add a partial unique index on `update_history(host_id, software_item_id)`
  /// scoped to active rows (status IN ('queued', 'pending', 'in_progress',
  /// 'awaiting_restart')).
  ///
  /// This complements the existing `uix_update_history_host_active` (host-level
  /// serialisation) with a narrower invariant: no two rows for the same
  /// (host, software item) pair may exist in any non-terminal status.
  ///
  /// Allows batch updates to create Queued rows for different software items on
  /// the same host while preventing duplicate triggers for the same item.
  #[derive(DeriveMigrationName)]
  pub(super) struct Migration;

  #[async_trait::async_trait]
  impl MigrationTrait for Migration {
      async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
          manager
              .create_index(
                  Index::create()
                      .name("uix_update_history_host_software_item_active")
                      .table(UpdateHistory::Table)
                      .col(UpdateHistory::HostId)
                      .col(UpdateHistory::SoftwareItemId)
                      .unique()
                      .and_where(
                          Expr::col(UpdateHistory::Status).is_in([
                              "queued",
                              "pending",
                              "in_progress",
                              "awaiting_restart",
                          ]),
                      )
                      .to_owned(),
              )
              .await
      }

      async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
          manager
              .drop_index(
                  Index::drop()
                      .name("uix_update_history_host_software_item_active")
                      .table(UpdateHistory::Table)
                      .to_owned(),
              )
              .await
      }
  }

  #[derive(DeriveIden)]
  enum UpdateHistory {
      Table,
      HostId,
      SoftwareItemId,
      Status,
  }
  ```

- [ ] **Step 2.2: Register the migration**

  In `crates/shared/db/src/migration/mod.rs`:
  1. Add the module declaration alongside the other migration modules:

     ```rust
     mod m20260515_000002_update_history_item_active_index;
     ```

  2. In the `fn migrations() -> Vec<Box<dyn MigrationTrait>>` vec, append after the current last entry (`m20260516_000001_2fa`):

     ```rust
     Box::new(m20260515_000002_update_history_item_active_index::Migration),
     ```

- [ ] **Step 2.3: Verify migration compiles**

  ```bash
  cargo check -p uptrakit-shared-db --no-default-features --features db-sqlite 2>&1 | grep -E "^error" | head -20
  ```

  Expected: no errors.

- [ ] **Step 2.4: Run migrations against a test DB to confirm index is created**

  ```bash
  cargo test -p uptrakit-shared-db --no-default-features --features db-sqlite -- migration 2>&1 | tail -20
  ```

  Expected: tests pass.

- [ ] **Step 2.5: Commit**

  ```bash
  git add \
    crates/shared/db/src/migration/m20260515_000002_update_history_item_active_index.rs \
    crates/shared/db/src/migration/mod.rs
  git commit -m "feat(db): add (host_id, software_item_id) partial unique index for active updates

  Prevents duplicate triggers for the same software item on the same host
  while in any non-terminal status. Complements the existing host-level
  unique index; both serve different invariants and both remain active.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 3: `has_active_update_for_host_software_item` pre-check + re-queue fallback

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs` (add function)
- Modify: `crates/ui/web-api-queries/src/queries/update_triggers.rs` (call new check; update fallback)

- [ ] **Step 3.1: Write a failing test for `has_active_update_for_host_software_item`**

  In the `#[cfg(test)] mod tests` block of `update_dispatch.rs`, add (this block already uses `#[cfg(all(test, feature = "db-sqlite"))]` — check the
  existing test setup):

  ```rust
  #[tokio::test]
  async fn has_active_update_for_host_software_item_returns_true_for_active_statuses() {
      use sea_orm::{ActiveModelTrait, Set};
      use uptrakit_shared_db::entity::update_history;
      use uptrakit_shared_types::UpdateStatus;

      let db = make_sqlite_db().await;
      let (tenant_id, host_id, item_id) = insert_update_history_parents(&db).await;

      for status in UpdateStatus::unfinished() {
          // Clear previous rows to avoid index violations between iterations
          update_history::Entity::delete_many()
              .exec(&db)
              .await
              .expect("delete");

          let id = uuid::Uuid::now_v7();
          let now = time::OffsetDateTime::now_utc();
          update_history::ActiveModel {
              id: Set(id),
              tenant_id: Set(tenant_id),
              host_id: Set(host_id),
              software_item_id: Set(item_id),
              host_software_item_id: Set(Some(uuid::Uuid::now_v7())),
              status: Set(status),
              batch_id: Set(None),
              actor_type: Set(crate::queries::update_types::ActorType::User.as_str().to_string()),
              actor_id: Set("test".to_string()),
              from_version: Set(None),
              to_version: Set(None),
              update_category: Set("unknown".to_string()),
              interactive: Set(false),
              created_at: Set(now),
              updated_at: Set(now),
          }
          .insert(&db)
          .await
          .expect("insert update_history");

          let result =
              super::has_active_update_for_host_software_item(&db, host_id, item_id)
                  .await
                  .expect("query");
          assert!(result, "expected true for status {status:?}");
      }
  }

  #[tokio::test]
  async fn has_active_update_for_host_software_item_returns_false_for_terminal_statuses() {
      use sea_orm::{ActiveModelTrait, Set};
      use uptrakit_shared_db::entity::update_history;
      use uptrakit_shared_types::UpdateStatus;

      let db = make_sqlite_db().await;
      let (tenant_id, host_id, item_id) = insert_update_history_parents(&db).await;

      for status in [UpdateStatus::Completed, UpdateStatus::Failed] {
          update_history::Entity::delete_many()
              .exec(&db)
              .await
              .expect("delete");

          let now = time::OffsetDateTime::now_utc();
          update_history::ActiveModel {
              id: Set(uuid::Uuid::now_v7()),
              tenant_id: Set(tenant_id),
              host_id: Set(host_id),
              software_item_id: Set(item_id),
              host_software_item_id: Set(Some(uuid::Uuid::now_v7())),
              status: Set(status),
              batch_id: Set(None),
              actor_type: Set(crate::queries::update_types::ActorType::User.as_str().to_string()),
              actor_id: Set("test".to_string()),
              from_version: Set(None),
              to_version: Set(None),
              update_category: Set("unknown".to_string()),
              interactive: Set(false),
              created_at: Set(now),
              updated_at: Set(now),
          }
          .insert(&db)
          .await
          .expect("insert");

          let result =
              super::has_active_update_for_host_software_item(&db, host_id, item_id)
                  .await
                  .expect("query");
          assert!(!result, "expected false for status {status:?}");
      }
  }
  ```

- [ ] **Step 3.2: Run tests to confirm they fail**

  ```bash
  cargo test -p uptrakit-web-api-queries --no-default-features --features db-sqlite -- has_active_update_for_host_software_item 2>&1 | tail -20
  ```

  Expected: compile error — `has_active_update_for_host_software_item` not defined yet.

- [ ] **Step 3.3: Add `has_active_update_for_host_software_item` to `update_dispatch.rs`**

  In `crates/ui/web-api-queries/src/queries/update_dispatch.rs`, add this function immediately after `has_active_update_for_host` (around line 950):

  ```rust
  /// Returns `true` if a Queued, Pending, InProgress, or AwaitingRestart row
  /// already exists for the given (host_id, software_item_id) pair.
  pub async fn has_active_update_for_host_software_item(
      db: &DatabaseConnection,
      host_id: Uuid,
      software_item_id: Uuid,
  ) -> Result<bool> {
      let count = UpdateHistory::find()
          .filter(update_history::Column::HostId.eq(host_id))
          .filter(update_history::Column::SoftwareItemId.eq(software_item_id))
          .filter(update_history::Column::Status.is_in(UpdateStatus::unfinished()))
          .count(db)
          .await
          .context_to()?;
      Ok(count > 0)
  }
  ```

  `UpdateStatus` is `uptrakit_shared_types::UpdateStatus` — add `use uptrakit_shared_types::UpdateStatus;` if not already in scope (it may already be
  imported via `update_history::UpdateStatus`).

- [ ] **Step 3.4: Run tests again to confirm they pass**

  ```bash
  cargo test -p uptrakit-web-api-queries --no-default-features --features db-sqlite -- has_active_update_for_host_software_item 2>&1 | tail -20
  ```

  Expected: both tests pass.

- [ ] **Step 3.5: Update `trigger_update_for_host` in `update_triggers.rs` — add item-level pre-check**

  In `crates/ui/web-api-queries/src/queries/update_triggers.rs`, find `trigger_update_for_host`. Around line 149, just before the
  `has_active_update_for_host` call, insert:

  ```rust
  // Item-level deduplication: 409 if any non-terminal row exists for this
  // (host, software_item) pair (precedes the host-level serialisation check).
  if has_active_update_for_host_software_item(db, params.host_id, params.item_id).await? {
      return Err(report!(TriggerUpdateError::UpdateAlreadyActive));
  }
  ```

  The existing line `let host_busy = has_active_update_for_host(db, params.host_id).await?;` stays immediately after.

  Add the import at the top of the file:

  ```rust
  use crate::queries::update_dispatch::has_active_update_for_host_software_item;
  ```

- [ ] **Step 3.6: Update re-queue fallback to handle the new unique constraint**

  In `update_triggers.rs`, find the `is_unique_constraint_violation` fallback (around line 175). It currently re-inserts as `Queued` and unwraps the
  result with `await?`. Change it so that if the Queued re-insert ALSO hits a unique violation, it returns `UpdateAlreadyActive`:

  Before:

  ```rust
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
  ```

  After:

  ```rust
  Err(e) if is_unique_constraint_violation(&e) => {
      tracing::debug!(
          host_id = %params.host_id,
          "concurrent Pending INSERT detected (unique constraint); re-inserting as Queued"
      );
      let queued_result = create_update_history_record(
          db,
          &build_record(update_history::UpdateStatus::Queued),
      )
      .await;
      match queued_result {
          Ok(id) => (id, update_history::UpdateStatus::Queued),
          Err(e) if is_unique_constraint_violation(&e) => {
              // (host_id, software_item_id) constraint violated for Queued row —
              // duplicate trigger for the same item.
              return Err(report!(TriggerUpdateError::UpdateAlreadyActive));
          }
          Err(e) => return Err(e),
      }
  }
  ```

- [ ] **Step 3.7: Verify `has_active_update_for_host_software_item` is wired in `update_triggers.rs`**

  Confirm the pre-check call site exists:

  ```bash
  grep -n "has_active_update_for_host_software_item" \
    crates/ui/web-api-queries/src/queries/update_triggers.rs
  ```

  Expected: at least one match at the line added in Step 3.5.

  > **Integration test note:** A full end-to-end test that calls `trigger_update_for_host` twice requires wiring a complete execute-update plugin
  > config (see the nearest existing test in `update_triggers.rs` that calls `trigger_update_for_host` directly). Implement that test in the same
  > `#[cfg(all(test, feature = "db-sqlite"))]` block, following the pattern of `insert_update_history_parents` in `update_dispatch.rs` for the
  > prerequisite setup. The integration test is strongly recommended before shipping; the unit tests in Steps 3.1–3.4 already cover the core query
  > logic.
  > **Do not commit a `todo!()` macro** — the workspace `todo = "deny"` lint rejects it in all build targets including tests.

- [ ] **Step 3.8: Verify compilation**

  ```bash
  cargo check -p uptrakit-web-api-queries --no-default-features --features db-sqlite 2>&1 | grep -E "^error" | head -20
  ```

  Expected: no errors.

- [ ] **Step 3.9: Commit**

  ```bash
  git add \
    crates/ui/web-api-queries/src/queries/update_dispatch.rs \
    crates/ui/web-api-queries/src/queries/update_triggers.rs
  git commit -m "feat(queries): add item-level duplicate-trigger pre-check

  has_active_update_for_host_software_item checks all four non-terminal
  statuses and is called before the host-level check in trigger_update_for_host.
  Re-queue fallback updated to return UpdateAlreadyActive when the Queued
  re-insert also hits the new (host_id, software_item_id) constraint.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 4: Enrich `SoftwareItemHostSummary` with `active_update_status`

**Files:**

- Modify: `crates/shared/web-api-types/src/software_items.rs:390` (add field to struct)
- Modify: `crates/ui/web-api-queries/src/queries/software_items/mod.rs` (active_updates map + query filter)
- Modify: `crates/ui/web-api-queries/src/queries/software_items/crud.rs:1092` (add field to struct literal)
- Modify: `crates/ui/cli/src/commands/software_items.rs:855,919` (add field to struct literals)

- [ ] **Step 4.1: Add field to `SoftwareItemHostSummary`**

  In `crates/shared/web-api-types/src/software_items.rs`, after the `active_update_history_id` field (line 390):

  ```rust
  /// Status of the active update, if any. One of: "queued", "pending",
  /// "in_progress", "awaiting_restart". None when no active update exists.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub active_update_status: Option<String>,
  ```

  The struct is `#[non_exhaustive]` — all struct literal sites will fail to compile until updated.

- [ ] **Step 4.2: Run `cargo check` to find all struct literal sites**

  ```bash
  cargo check -p uptrakit-web-api-queries --no-default-features --features db-sqlite 2>&1 | grep "SoftwareItemHostSummary\|missing field" | head -20
  ```

  Expected errors at the three known literal sites (crud.rs:1092, cli:855, cli:919) plus `software_items/mod.rs:481`.

- [ ] **Step 4.3: Update struct literal in `crud.rs`**

  In `crates/ui/web-api-queries/src/queries/software_items/crud.rs` at line 1119 (the `SoftwareItemHostSummary` struct literal that ends with
  `active_update_history_id: None`), add after that field:

  ```rust
  active_update_status: None,
  ```

- [ ] **Step 4.4: Update struct literals in CLI**

  In `crates/ui/cli/src/commands/software_items.rs`, both struct literals (around lines 855 and 919) have `active_update_history_id: None` — add after
  each:

  ```rust
  active_update_status: None,
  ```

- [ ] **Step 4.5: Update the `active_updates` map in `software_items/mod.rs`**

  In `crates/ui/web-api-queries/src/queries/software_items/mod.rs`, the `HostAssignmentData` struct stores:

  ```rust
  pub(super) active_updates: HashMap<Uuid, Uuid>,
  ```

  Change it to carry the status alongside the ID:

  ```rust
  pub(super) active_updates: HashMap<Uuid, (Uuid, String)>,
  ```

  Find the `active_updates` query (around line 346). Make three changes:
  1. Switch the status filter from the manual list to `unfinished()`:

     Before:

     ```rust
     .filter(update_history::Column::Status.is_in([
         UpdateStatus::Queued,
         UpdateStatus::Pending,
         UpdateStatus::InProgress,
     ]))
     ```

     After:

     ```rust
     .filter(update_history::Column::Status.is_in(UpdateStatus::unfinished()))
     ```

     Add `use uptrakit_shared_types::UpdateStatus;` at the top of the file if not present.

  2. Update the `collect` call (line 357) to store `(id, status_str)`:

     Before:

     ```rust
     Ok(rows) => rows.into_iter().map(|u| (u.host_id, u.id)).collect(),
     ```

     After:

     ```rust
     Ok(rows) => rows
         .into_iter()
         .map(|u| (u.host_id, (u.id, u.status.to_string())))
         .collect(),
     ```

  3. Update the `SoftwareItemHostSummary` construction (around line 481) to unpack the new tuple. Compute a single `active` lookup before the struct
     literal to avoid calling `HashMap::get` twice for the same key:

     Before:

     ```rust
     active_update_history_id: data.active_updates.get(&host.id).copied(),
     ```

     After (declare `active` immediately before the struct literal):

     ```rust
     let active = data.active_updates.get(&host.id);
     ```

     Then in the struct literal:

     ```rust
     active_update_history_id: active.map(|(id, _)| *id),
     active_update_status: active.map(|(_, s)| s.clone()),
     ```

- [ ] **Step 4.6: Write a test verifying `AwaitingRestart` populates `active_update_status`**

  In the test section of `software_items/mod.rs` (or an existing test file), add:

  ```rust
  #[cfg(all(test, feature = "db-sqlite"))]
  mod active_update_status_tests {
      #![expect(
          clippy::expect_used,
          reason = "test code: panics on failure are acceptable"
      )]

      // Test that a host with an AwaitingRestart row populates both
      // active_update_history_id and active_update_status on the summary.
      // Use the existing test DB setup pattern from this module.
      // Expected assertions:
      //   assert_eq!(host_summary.active_update_history_id, Some(update_id));
      //   assert_eq!(host_summary.active_update_status.as_deref(), Some("awaiting_restart"));
      //
      // For a Queued row:
      //   assert_eq!(host_summary.active_update_status.as_deref(), Some("queued"));
      //
      // For a Completed row (no active row):
      //   assert!(host_summary.active_update_history_id.is_none());
      //   assert!(host_summary.active_update_status.is_none());
      //
      // Implement following the pattern of existing software_items/mod.rs tests
      // that set up host_software_item rows and call the query functions.
  }
  ```

  > **Note:** The full implementation depends on the existing test harness in `software_items/mod.rs`. Follow the existing test structure — insert the
  > required parent rows (tenant, host, software_item, host_software_item) then insert an `update_history` row with `status = AwaitingRestart`, then
  > call the query function and assert the returned `SoftwareItemHostSummary` fields.

- [ ] **Step 4.7: Verify compilation (all features)**

  ```bash
  cargo check --all-features 2>&1 | grep "^error" | head -20
  ```

  Expected: no errors.

- [ ] **Step 4.8: Commit**

  ```bash
  git add \
    crates/shared/web-api-types/src/software_items.rs \
    crates/ui/web-api-queries/src/queries/software_items/mod.rs \
    crates/ui/web-api-queries/src/queries/software_items/crud.rs \
    crates/ui/cli/src/commands/software_items.rs
  git commit -m "feat(api): add active_update_status to SoftwareItemHostSummary

  Populates the field from the active update row alongside the existing
  active_update_history_id. Switches the active_updates query to use
  UpdateStatus::unfinished() — adds AwaitingRestart to the filter.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 5: Add `status` field to `UpdateTriggered` SSE event

**Files:**

- Modify: `crates/shared/wire/src/admin_events.rs`
- Modify: `crates/shared/wire/asyncapi.yaml`
- Modify: `crates/shared/openapi-client/src/events_stream.rs`
- Modify: `crates/ui/web-api/src/routes/events.rs` (test)
- Modify: `crates/ui/web-api/src/actions/update_batches.rs` (lines 88, 331)
- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs` (add emit)
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs` (line 165)

- [ ] **Step 5.1: Add `status` to `UpdateTriggered` enum variant in `admin_events.rs`**

  In `crates/shared/wire/src/admin_events.rs`, update the `UpdateTriggered` variant (around line 55):

  Before:

  ```rust
  UpdateTriggered {
      update_history_id: Uuid,
      host_id: Uuid,
      software_item_id: Uuid,
  },
  ```

  After:

  ```rust
  UpdateTriggered {
      update_history_id: Uuid,
      host_id: Uuid,
      software_item_id: Uuid,
      /// Trigger status: "pending" (agent connected) or "queued" (agent offline).
      status: String,
  },
  ```

- [ ] **Step 5.2: Update the handwritten `Deserialize` impl — `Inner` struct**

  In the handwritten `Deserialize` impl (around line 271), find the `"update_triggered"` arm. Add `status` to the local `Inner` struct with a named
  default function:

  Before:

  ```rust
  "update_triggered" => {
      #[derive(Deserialize)]
      struct Inner {
          update_history_id: Uuid,
          host_id: Uuid,
          software_item_id: Uuid,
      }
      let Inner {
          update_history_id,
          host_id,
          software_item_id,
      } = serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
      Ok(Self::UpdateTriggered {
          update_history_id,
          host_id,
          software_item_id,
      })
  }
  ```

  After:

  ```rust
  "update_triggered" => {
      fn default_pending_status() -> String {
          "pending".into()
      }
      #[derive(Deserialize)]
      struct Inner {
          update_history_id: Uuid,
          host_id: Uuid,
          software_item_id: Uuid,
          #[serde(default = "default_pending_status")]
          status: String,
      }
      let Inner {
          update_history_id,
          host_id,
          software_item_id,
          status,
      } = serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
      Ok(Self::UpdateTriggered {
          update_history_id,
          host_id,
          software_item_id,
          status,
      })
  }
  ```

  > **Why `default_pending_status` not `#[serde(default)]`:** bare `#[serde(default)]` would produce an empty string `""` when the `status` key is
  > absent from old-server payloads. An empty string cannot be mapped to a badge label. Defaulting to `"pending"` is safe — single-host triggers start
  > as `Pending`, and `"pending"` shows the correct non-clickable badge on both old and new servers.

- [ ] **Step 5.3: Update `all_variants()` test helper**

  In the `all_variants()` function (around line 478), add `status` to the `UpdateTriggered` constructor:

  Before:

  ```rust
  AdminEvent::UpdateTriggered {
      update_history_id: id,
      host_id: id,
      software_item_id: id,
  },
  ```

  After:

  ```rust
  AdminEvent::UpdateTriggered {
      update_history_id: id,
      host_id: id,
      software_item_id: id,
      status: "pending".to_string(),
  },
  ```

- [ ] **Step 5.4: Update the `events.rs` test**

  In `crates/ui/web-api/src/routes/events.rs`, find the test `sse_data_update_triggered_exposes_inner_fields` (around line 163). Update the
  `AdminEvent::UpdateTriggered` constructor:

  Before:

  ```rust
  let event = AdminEvent::UpdateTriggered {
      update_history_id: id,
      host_id: id,
      software_item_id: id,
  };
  ```

  After:

  ```rust
  let event = AdminEvent::UpdateTriggered {
      update_history_id: id,
      host_id: id,
      software_item_id: id,
      status: "pending".to_string(),
  };
  ```

  Also add an assertion that `status` is exposed:

  ```rust
  assert!(data.get("status").is_some(), "status missing: {data}");
  ```

- [ ] **Step 5.5: Check compile to find all remaining `UpdateTriggered` constructor sites**

  ```bash
  cargo check --all-features 2>&1 | grep "UpdateTriggered\|missing field" | head -20
  ```

  Expected: errors at `update_batches.rs:88`, `update_batches.rs:331`, `service_ws/handler/update_tracking.rs:165`, and the `software_items/mod.rs`
  trigger handler.

- [ ] **Step 5.6: Update batch emit sites in `update_batches.rs`**

  In `crates/ui/web-api/src/actions/update_batches.rs`, there are two `AdminEvent::UpdateTriggered` constructors (lines 88 and 331). Each iterates
  `resp.updates` and `item` is a `BatchUpdateItem` with field `trigger_status: TriggerUpdateStatus`. Add `status` to both:

  Line 88 — before:

  ```rust
  AdminEvent::UpdateTriggered {
      update_history_id: item.update_history_id,
      host_id: item.host_id,
      software_item_id: item.software_item_id,
  },
  ```

  After:

  ```rust
  AdminEvent::UpdateTriggered {
      update_history_id: item.update_history_id,
      host_id: item.host_id,
      software_item_id: item.software_item_id,
      status: item.trigger_status.to_string(),
  },
  ```

  Apply the same change at line 331. (`TriggerUpdateStatus` has no `as_str()` method — use `.to_string()` via its `Display` impl.)

- [ ] **Step 5.7: Add `UpdateTriggered` emit to the HTTP `trigger_update` handler**

  In `crates/ui/web-api/src/routes/software_items/mod.rs`, find the `trigger_update` function (line 1726). After the `status` match and before the
  `Ok(...)` return (around line 1791), add the event broadcast:

  Before:

  ```rust
  let resp = TriggerUpdateResponse {
      update_history_id: result.update_history_id,
      status,
  };
  Ok((StatusCode::OK, Json(resp)).into_response())
  ```

  After:

  ```rust
  state
      .notification
      .event_broadcaster
      .send(
          tenant_db.tenant_id(),
          AdminEvent::UpdateTriggered {
              update_history_id: result.update_history_id,
              host_id,
              software_item_id: item_id,
              status: status.to_string(),
          },
      )
      .await;

  let resp = TriggerUpdateResponse {
      update_history_id: result.update_history_id,
      status,
  };
  Ok((StatusCode::OK, Json(resp)).into_response())
  ```

  Verify `AdminEvent` is already in scope (check the imports at the top of `mod.rs`).

- [ ] **Step 5.8: Update service-WS emit in `update_tracking.rs`**

  In `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs`, find the `AdminEvent::UpdateTriggered` emit (line 165). The local variable
  holding the dispatch status (e.g. `dispatch_status`) should be used. Read the surrounding code to determine the correct variable name:

  Before:

  ```rust
  AdminEvent::UpdateTriggered {
      update_history_id: result.update_history_id,
      host_id: payload.host_id,
      software_item_id: payload.software_item_id,
  },
  ```

  After:

  ```rust
  AdminEvent::UpdateTriggered {
      update_history_id: result.update_history_id,
      host_id: payload.host_id,
      software_item_id: payload.software_item_id,
      status: dispatch_status.to_string(),
  },
  ```

  > **Note:** The `dispatch_status` variable name is from the `serde_json::json!` block just above (line 146 shows
  > `"dispatch_status": dispatch_status`). Confirm the variable name before substituting — it may be a `TriggerUpdateStatus` or similar type.

- [ ] **Step 5.9: Update `openapi-client/src/events_stream.rs`**

  In `crates/shared/openapi-client/src/events_stream.rs`:
  1. Add `status: String` to `AdminSseEvent::UpdateTriggered` variant:

     Before:

     ```rust
     UpdateTriggered {
         update_history_id: Uuid,
         host_id: Uuid,
         software_item_id: Uuid,
     },
     ```

     After:

     ```rust
     UpdateTriggered {
         update_history_id: Uuid,
         host_id: Uuid,
         software_item_id: Uuid,
         /// Trigger status: "pending" or "queued".
         status: String,
     },
     ```

  2. Add `status` to the `Payload` struct in the `"update_triggered"` arm (around line 196):

     ```rust
     "update_triggered" => {
         fn default_pending_status() -> String {
             "pending".into()
         }
         #[derive(serde::Deserialize)]
         struct Payload {
             update_history_id: Uuid,
             host_id: Uuid,
             software_item_id: Uuid,
             #[serde(default = "default_pending_status")]
             status: String,
         }
         let p: Payload = serde_json::from_str(&event.data)?;
         Ok(AdminSseEvent::UpdateTriggered {
             update_history_id: p.update_history_id,
             host_id: p.host_id,
             software_item_id: p.software_item_id,
             status: p.status,
         })
     }
     ```

  3. Update the `parse_update_triggered` test (line 368) to include `status` in the JSON fixture:

     Before:

     ```rust
     let event = make_event(
         "update_triggered",
         r#"{"update_history_id":"550e8400-e29b-41d4-a716-446655440001","host_id":"550e8400-e29b-41d4-a716-446655440002","software_item_id":"550e8400-e29b-41d4-a716-446655440003"}"#,
     );
     ```

     After:

     ```rust
     let event = make_event(
         "update_triggered",
         r#"{"update_history_id":"550e8400-e29b-41d4-a716-446655440001","host_id":"550e8400-e29b-41d4-a716-446655440002","software_item_id":"550e8400-e29b-41d4-a716-446655440003","status":"pending"}"#,
     );
     ```

     Add an assertion on the `status` field:

     ```rust
     assert!(matches!(
         result,
         AdminSseEvent::UpdateTriggered { status, .. } if status == "pending"
     ));
     ```

  4. Also add a backward-compat test verifying `status` defaults to `"pending"` when absent:

     ```rust
     #[test]
     fn parse_update_triggered_missing_status_defaults_to_pending() {
         let event = make_event(
             "update_triggered",
             r#"{"update_history_id":"550e8400-e29b-41d4-a716-446655440001","host_id":"550e8400-e29b-41d4-a716-446655440002","software_item_id":"550e8400-e29b-41d4-a716-446655440003"}"#,
         );
         let result = parse_typed_event(event).unwrap();
         assert!(matches!(
             result,
             AdminSseEvent::UpdateTriggered { status, .. } if status == "pending"
         ));
     }
     ```

- [ ] **Step 5.10: Update `asyncapi.yaml` example**

  In `crates/shared/wire/asyncapi.yaml` at line 4279, update the example string for `update_triggered` to include `status`:

  Before:

  ```yaml
  - '{"update_triggered":{"update_history_id":"...","host_id":"...","software_item_id":"..."}}'
  ```

  After:

  ```yaml
  - '{"update_triggered":{"update_history_id":"...","host_id":"...","software_item_id":"...","status":"pending"}}'
  ```

- [ ] **Step 5.11: Compile and test**

  ```bash
  cargo check --all-features 2>&1 | grep "^error" | head -20
  ```

  ```bash
  cargo test -p uptrakit-wire --all-features 2>&1 | tail -30
  cargo test -p uptrakit-openapi-client --all-features 2>&1 | tail -20
  ```

  Expected: all clean.

- [ ] **Step 5.12: Commit**

  ```bash
  git add \
    crates/shared/wire/src/admin_events.rs \
    crates/shared/wire/asyncapi.yaml \
    crates/shared/openapi-client/src/events_stream.rs \
    crates/ui/web-api/src/routes/events.rs \
    crates/ui/web-api/src/actions/update_batches.rs \
    crates/ui/web-api/src/routes/software_items/mod.rs \
    crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs
  git commit -m "feat(wire): add status field to UpdateTriggered SSE event

  All four emit sites updated. Handwritten Deserialize Inner struct uses
  #[serde(default = \"default_pending_status\")] for backward compat —
  absent field defaults to 'pending' rather than empty string.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 6: Full quality gate + documentation

**Files:**

- Modify: `docs/development/coding-standards.md`
- Modify: `CONTEXT.md`

- [ ] **Step 6.1: Run full backend quality gate**

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error\|^warning.*unused" | head -30
  cargo clippy --all-targets --all-features 2>&1 | grep "^error\|^warning.*unused" | head -30
  cargo test --all-features 2>&1 | tail -30
  cargo deny check
  ```

  Fix any clippy warnings before continuing.

- [ ] **Step 6.2: Update `coding-standards.md`**

  In `docs/development/coding-standards.md`, find the "Database Query Patterns" section. Add a note:

  ```markdown
  ### UpdateStatus grouping helpers

  Use `UpdateStatus::unfinished()` and `UpdateStatus::host_blocking()` for status filters — do not inline the status arrays at call sites.

  - `unfinished()` — all four non-terminal statuses (Queued, Pending, InProgress, AwaitingRestart). Use for: "does an active row exist for this (host,
    item)?", state reporting queries.
  - `host_blocking()` — excludes Queued. Use for: "is this host currently occupied by an in-flight update?", host-level serialisation checks.
  ```

- [ ] **Step 6.3: Update `CONTEXT.md`**

  In `CONTEXT.md`, find the `UpdateStatus` glossary entry and append or update:

  ```text
  UpdateStatus — grouping helpers: `unfinished()` = [Queued, Pending, InProgress,
  AwaitingRestart] (all non-terminal; use for duplicate-trigger checks);
  `host_blocking()` = [Pending, InProgress, AwaitingRestart] (occupies host
  execution slot; excludes Queued).
  ```

- [ ] **Step 6.4: Lint markdown**

  ```bash
  npx markdownlint --config .markdownlint.json docs/development/coding-standards.md CONTEXT.md
  ```

  Fix any violations.

- [ ] **Step 6.5: Commit**

  ```bash
  git add docs/development/coding-standards.md CONTEXT.md
  git commit -m "docs: document UpdateStatus grouping helpers in standards and glossary

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```
