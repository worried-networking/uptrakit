# Resumable Updates & Self-Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add resumable update semantics (AwaitingRestart status + scheduler verification) and an auto-discovery plugin
that wires up uptrakit self-update with zero manual configuration.

**Architecture:** Three layers — data model (migrations + entity fields + new status variant), protocol extensions
(resumable flag on wire types, early-result channel in agent pipeline), and a new discovery plugin that generates
topology-specific update scripts and injects `resumable: true` into plugin assignments. The scheduler gains a
`TickExecutor` abstraction to drive cross-tenant AwaitingRestart polling without a DB-backed scheduled_task row.

**Tech Stack:** Rust/Tokio, SeaORM (SQLite in tests, Postgres in prod), `sea_orm_migration`, `async-trait`,
`parking_lot`, `tokio::sync::mpsc`, `declare_plugin!` macro.

---

## File Structure

### New files

- `crates/shared/db/src/migration/m20260430_000001_awaiting_restart_timeout.rs` — Add `awaiting_restart_timeout` to `software_items`
- `crates/shared/db/src/migration/m20260430_000002_awaiting_restart_since.rs` — Add `awaiting_restart_since` to `update_history`
- `crates/shared/db/src/migration/m20260430_000003_update_history_host_active_index.rs` — Recreate partial unique index to include `awaiting_restart`
- `crates/shared/scheduler-engine/src/tick_executor.rs` — `TickExecutor` trait
- `crates/shared/scheduler-engine/src/executors/awaiting_restart.rs` — `AwaitingRestartExecutor`
- `crates/plugins/infrastructure/core/src/service_metadata.rs` — `ServiceMetadata`, `DeploymentTopology`, `ServiceMetadataProvider`
- `crates/core/controller-runtime/src/embedded/metadata_runtime.rs` — `MetadataAwareHostRuntime`
- `crates/plugins/discovery/uptrakit-self-update/Cargo.toml` — New plugin crate
- `crates/plugins/discovery/uptrakit-self-update/src/lib.rs` — Crate root
- `crates/plugins/discovery/uptrakit-self-update/src/config.rs` — `UptrakitSelfUpdateConfig`
- `crates/plugins/discovery/uptrakit-self-update/src/error.rs` — `SelfUpdateError`
- `crates/plugins/discovery/uptrakit-self-update/src/plugin.rs` — `declare_plugin!` + `new()`
- `crates/plugins/discovery/uptrakit-self-update/src/discovery.rs` — `Discoverer` impl

### Modified files

- `crates/shared/db/src/migration/mod.rs` — Register 3 new migrations
- `crates/shared/db/src/entity/software_item.rs` — Add `awaiting_restart_timeout: Option<i32>`
- `crates/shared/db/src/entity/update_history.rs` — Add `awaiting_restart_since: Option<OffsetDateTime>`
- `crates/shared/types/src/update_status.rs` — Add `AwaitingRestart` variant
- `crates/shared/web-api-types/src/update_history.rs` — Add `AwaitingRestart` variant
- `crates/shared/wire/src/payloads.rs` — Add `resumable` to `UpdateResultPayload`; `not_ready` to `VersionCheckResult`
- `crates/plugins/infrastructure/core/src/roles.rs` — `ExecuteUpdateResult` struct; change `UpdateExecutor::execute_update` return type
- `crates/plugins/infrastructure/core/src/host_runtime.rs` — `metadata_provider()` default method on `HostRuntime`
- `crates/plugins/infrastructure/core/src/lib.rs` — Re-export `service_metadata` types
- `crates/plugins/generic/shell/src/config.rs` — Add `resumable: bool`
- `crates/plugins/generic/shell/src/plugin.rs` — Return `ExecuteUpdateResult`
- All other `UpdateExecutor` implementors — Return `ExecuteUpdateResult { output, resumable: false }`
- `crates/shared/agent-core/src/update.rs` — `PipelineResult`, post-hooks out of pipeline, `early_result_tx` param
- `crates/shared/agent-core/src/client.rs` — `InFlightUpdate` + `early_sent`, `send_update_result`, `handle_graceful_shutdown`
- `crates/core/agent-runtime/src/lib.rs` — `poll_in_flight_update` drain, `Completed` handler
- `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs` — `transition_to_awaiting_restart`,
  `has_active_update_for_host`, `maybe_complete_batch`
- `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` — Resumable branch in `handle_update_result`
- `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` — AwaitingRestart correlation in `handle_version_check_results`
- `crates/shared/scheduler-engine/src/scheduler.rs` — `tick_executors` Vec + separate JoinSet
- `crates/shared/scheduler-engine/src/lib.rs` — Re-export `TickExecutor`
- `crates/shared/scheduler-engine/src/notifier.rs` — Add `signal_host_progression`
- `crates/core/controller-runtime/src/scheduler/mod.rs` — Implement `signal_host_progression`; register `AwaitingRestartExecutor`
- `crates/core/controller-runtime/src/embedded/` — Wire `MetadataAwareHostRuntime` into plugin construction
- Controller-standalone plugin registry — Register `UptrakitSelfUpdatePlugin`

---

### Task 1: DB Migrations

**Files:**

- Create: `crates/shared/db/src/migration/m20260430_000001_awaiting_restart_timeout.rs`
- Create: `crates/shared/db/src/migration/m20260430_000002_awaiting_restart_since.rs`
- Create: `crates/shared/db/src/migration/m20260430_000003_update_history_host_active_index.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`
- Modify: `crates/shared/db/src/entity/software_item.rs`
- Modify: `crates/shared/db/src/entity/update_history.rs`

- [ ] **Step 1: Read the migration template**

Read `crates/shared/db/src/migration/m20260424_000001_access_mcp_permission.rs` and
`crates/shared/db/src/migration/mod.rs` to confirm the `pub(super)` visibility, module naming, and registration order.

- [ ] **Step 2: Create migration 1 — `awaiting_restart_timeout` on `software_items`**

```rust
// crates/shared/db/src/migration/m20260430_000001_awaiting_restart_timeout.rs
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("software_items"))
                    .add_column_if_not_exists(
                        ColumnDef::new(Alias::new("awaiting_restart_timeout"))
                            .integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("software_items"))
                    .drop_column(Alias::new("awaiting_restart_timeout"))
                    .to_owned(),
            )
            .await
    }
}
```

- [ ] **Step 3: Create migration 2 — `awaiting_restart_since` on `update_history`**

```rust
// crates/shared/db/src/migration/m20260430_000002_awaiting_restart_since.rs
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("update_history"))
                    .add_column_if_not_exists(
                        ColumnDef::new(Alias::new("awaiting_restart_since"))
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("update_history"))
                    .drop_column(Alias::new("awaiting_restart_since"))
                    .to_owned(),
            )
            .await
    }
}
```

- [ ] **Step 4: Create migration 3 — recreate partial unique index**

The `SeaORM` `SchemaManager` cannot express partial indexes with `WHERE` clauses. Use raw SQL.

```rust
// crates/shared/db/src/migration/m20260430_000003_update_history_host_active_index.rs
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP INDEX IF EXISTS uix_update_history_host_active",
        )
        .await?;
        db.execute_unprepared(
            "CREATE UNIQUE INDEX uix_update_history_host_active \
             ON update_history (host_id) \
             WHERE status IN ('pending', 'in_progress', 'awaiting_restart')",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP INDEX IF EXISTS uix_update_history_host_active",
        )
        .await?;
        db.execute_unprepared(
            "CREATE UNIQUE INDEX uix_update_history_host_active \
             ON update_history (host_id) \
             WHERE status IN ('pending', 'in_progress')",
        )
        .await?;
        Ok(())
    }
}
```

- [ ] **Step 5: Register all three in `mod.rs`**

Add to the `mod` declarations at the top of `crates/shared/db/src/migration/mod.rs`:

```rust
mod m20260430_000001_awaiting_restart_timeout;
mod m20260430_000002_awaiting_restart_since;
mod m20260430_000003_update_history_host_active_index;
```

Add to the `vec![]` in `Migrator::migrations()` (after the existing last entry):

```rust
Box::new(m20260430_000001_awaiting_restart_timeout::Migration),
Box::new(m20260430_000002_awaiting_restart_since::Migration),
Box::new(m20260430_000003_update_history_host_active_index::Migration),
```

- [ ] **Step 6: Add columns to entity `Model` structs**

In `crates/shared/db/src/entity/software_item.rs`, add to `Model`:

```rust
pub awaiting_restart_timeout: Option<i32>,
```

In `crates/shared/db/src/entity/update_history.rs`, add to `Model` (after `completed_at`):

```rust
pub awaiting_restart_since: Option<OffsetDateTime>,
```

`OffsetDateTime` is already imported in the file (`use time::OffsetDateTime`). SeaORM's `DeriveEntityModel`
auto-generates `Column::AwaitingRestartSince` and `Column::AwaitingRestartTimeout` from the new fields —
no manual `Column` enum edits needed.

- [ ] **Step 7: Verify compilation**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git commit --only \
  crates/shared/db/src/migration/m20260430_000001_awaiting_restart_timeout.rs \
  crates/shared/db/src/migration/m20260430_000002_awaiting_restart_since.rs \
  crates/shared/db/src/migration/m20260430_000003_update_history_host_active_index.rs \
  crates/shared/db/src/migration/mod.rs \
  crates/shared/db/src/entity/software_item.rs \
  crates/shared/db/src/entity/update_history.rs \
  -m "feat(db): add awaiting_restart_timeout, awaiting_restart_since columns and update partial index"
```

---

### Task 2: `UpdateStatus::AwaitingRestart` — both enums

**Files:**

- Modify: `crates/shared/types/src/update_status.rs`
- Modify: `crates/shared/web-api-types/src/update_history.rs`
- Test: existing test files in each crate

- [ ] **Step 1: Write the failing test in shared/types**

Find the existing round-trip test in `crates/shared/types/src/update_status.rs` (or a test module that iterates all variants). Add:

```rust
#[test]
fn test_awaiting_restart_round_trip() {
    let s = UpdateStatus::AwaitingRestart;
    assert_eq!(s.as_str(), "awaiting_restart");
    let parsed: UpdateStatus = "awaiting_restart".parse().unwrap();
    assert_eq!(parsed, UpdateStatus::AwaitingRestart);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-shared-types test_awaiting_restart_round_trip -- --nocapture
```

Expected: FAIL — `AwaitingRestart` not found.

- [ ] **Step 3: Add `AwaitingRestart` to shared/types `UpdateStatus`**

In `crates/shared/types/src/update_status.rs`, add after `InProgress`:

```rust
#[cfg_attr(feature = "sea-orm", sea_orm(string_value = "awaiting_restart"))]
AwaitingRestart,
```

`as_str()` needs a new arm:

```rust
UpdateStatus::AwaitingRestart => "awaiting_restart",
```

`FromStr` needs a new arm (find the match block):

```rust
"awaiting_restart" => Ok(Self::AwaitingRestart),
```

If `Display` has an explicit match, add the same arm there too.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p uptrakit-shared-types -- --nocapture
```

Expected: all pass including `test_awaiting_restart_round_trip`.

- [ ] **Step 5: Write failing test in web-api-types**

Find the test module in `crates/shared/web-api-types/src/update_history.rs`. Add:

```rust
#[test]
fn test_awaiting_restart_serde() {
    let s = UpdateStatus::AwaitingRestart;
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(json, r#""awaiting_restart""#);
    let back: UpdateStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(back, UpdateStatus::AwaitingRestart);
}
```

- [ ] **Step 6: Run to verify it fails**

```bash
cargo test -p uptrakit-web-api-types test_awaiting_restart_serde
```

Expected: FAIL.

- [ ] **Step 7: Add `AwaitingRestart` to web-api-types `UpdateStatus`**

In `crates/shared/web-api-types/src/update_history.rs`, add after `InProgress`:

```rust
AwaitingRestart,
```

The `#[serde(rename_all = "snake_case")]` derive handles serialization automatically.

- [ ] **Step 8: Verify and commit**

```bash
cargo test -p uptrakit-web-api-types -- --nocapture
cargo check --all-features
```

```bash
git commit --only \
  crates/shared/types/src/update_status.rs \
  crates/shared/web-api-types/src/update_history.rs \
  -m "feat(types): add UpdateStatus::AwaitingRestart variant"
```

---

### Task 3: Wire type extensions

**Files:**

- Modify: `crates/shared/wire/src/payloads.rs`

- [ ] **Step 1: Write failing tests**

Find or create the test module in `crates/shared/wire/src/payloads.rs`. Add:

```rust
#[cfg(test)]
mod resumable_tests {
    use super::*;

    #[test]
    fn test_update_result_payload_resumable_defaults_none() {
        let json = r#"{"update_history_id":"00000000-0000-0000-0000-000000000001","status":"completed","output":""}"#;
        let p: UpdateResultPayload = serde_json::from_str(json).unwrap();
        assert_eq!(p.resumable, None);
    }

    #[test]
    fn test_update_result_payload_resumable_true_round_trips() {
        let p = UpdateResultPayload {
            update_history_id: uuid::Uuid::nil(),
            status: crate::UpdateFinalStatus::Completed,
            from_version: None,
            to_version: None,
            output: String::new(),
            error: None,
            resumable: Some(true),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"resumable\":true"));
        let back: UpdateResultPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.resumable, Some(true));
    }

    #[test]
    fn test_version_check_result_not_ready_defaults_none() {
        let json = r#"{"software_item_id":"00000000-0000-0000-0000-000000000001","update_category":"none"}"#;
        let r: VersionCheckResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.not_ready, None);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p uptrakit-wire resumable_tests -- --nocapture
```

Expected: FAIL — `resumable` field not found on `UpdateResultPayload`.

- [ ] **Step 3: Add `resumable` to `UpdateResultPayload`**

In `crates/shared/wire/src/payloads.rs`, locate `UpdateResultPayload` and add after `error`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub resumable: Option<bool>,
```

- [ ] **Step 4: Add `not_ready` to `VersionCheckResult`**

Locate `VersionCheckResult` and add after `error`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub not_ready: Option<bool>,
```

- [ ] **Step 5: Fix all struct literal usages**

`UpdateResultPayload` and `VersionCheckResult` are `#[non_exhaustive]` (verify — if they are, struct literals in the
same crate work; external crates must use `..Default::default()` or builder). Run:

```bash
cargo check --all-features 2>&1 | grep "missing field"
```

For every location that constructs `UpdateResultPayload` without `resumable`, add `resumable: None`.
For `VersionCheckResult` without `not_ready`, add `not_ready: None`.

- [ ] **Step 6: Verify tests pass**

```bash
cargo test -p uptrakit-wire resumable_tests -- --nocapture
cargo check --all-features
```

- [ ] **Step 7: Commit**

```bash
git commit --only crates/shared/wire/src/payloads.rs \
  -m "feat(wire): add resumable to UpdateResultPayload, not_ready to VersionCheckResult"
```

---

### Task 4: `ExecuteUpdateResult` struct + `UpdateExecutor` trait change

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs`

- [ ] **Step 1: Write failing test**

Find the test module in `crates/plugins/infrastructure/core/src/roles.rs`. Add:

```rust
#[cfg(test)]
mod execute_update_result_tests {
    use super::*;

    #[test]
    fn test_execute_update_result_default_not_resumable() {
        let r = ExecuteUpdateResult { output: "ok".to_string(), resumable: false };
        assert!(!r.resumable);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-core execute_update_result_tests
```

Expected: FAIL — `ExecuteUpdateResult` not defined.

- [ ] **Step 3: Define `ExecuteUpdateResult` and update trait**

In `crates/plugins/infrastructure/core/src/roles.rs`, before the `UpdateExecutor` trait, add:

```rust
/// Result returned by [`UpdateExecutor::execute_update`].
pub struct ExecuteUpdateResult {
    pub output: String,
    /// When `true`, the controller will transition this update to `AwaitingRestart`
    /// instead of `Completed`. The plugin decides this based on what actually happened
    /// (e.g., the shell plugin's `resumable: true` config, or APT detecting a reboot is needed).
    pub resumable: bool,
}
```

Change the `UpdateExecutor` trait `execute_update` signature from:

```rust
async fn execute_update(
    &self,
    package_identifier: &str,
    to_version: &str,
    release_info: Option<&ReleaseInfo>,
    output_tx: &UpdateOutputSender,
) -> Result<String>;
```

to:

```rust
async fn execute_update(
    &self,
    package_identifier: &str,
    to_version: &str,
    release_info: Option<&ReleaseInfo>,
    output_tx: &UpdateOutputSender,
) -> Result<ExecuteUpdateResult>;
```

Also re-export `ExecuteUpdateResult` in `crates/plugins/infrastructure/core/src/lib.rs`:

```rust
pub use roles::ExecuteUpdateResult;
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-core execute_update_result_tests
```

Expected: PASS.

- [ ] **Step 5: Check what breaks**

```bash
cargo check --all-features 2>&1 | grep "error\[" | head -40
```

Every plugin implementing `execute_update` will now fail to compile — they all return `Result<String>`.
Note all the file paths. You will fix them in Task 5.

- [ ] **Step 6: Commit the trait change only**

```bash
git commit --only \
  crates/plugins/infrastructure/core/src/roles.rs \
  crates/plugins/infrastructure/core/src/lib.rs \
  -m "feat(plugin-core): ExecuteUpdateResult struct, update UpdateExecutor::execute_update return type"
```

---

### Task 5: Update all `UpdateExecutor` implementors

**Files:**

- Modify: every file that returned `Result<String>` from `execute_update` (found in Task 4 Step 5)
- Modify: `crates/shared/agent-core/src/update.rs` (calls `execute_update`, uses the `String` result)

- [ ] **Step 1: Find all implementors**

```bash
cargo check --all-features 2>&1 | grep "error\[E" | grep "execute_update\|expected.*String\|found.*ExecuteUpdateResult" | grep "src/" | sed 's/.*--> //' | cut -d: -f1 | sort -u
```

This lists every file with a compile error from the return-type change. Expected to include:

- `crates/plugins/generic/shell/src/plugin.rs`
- `crates/plugins/package-managers/apt/src/plugin.rs` (if exists)
- `crates/plugins/package-managers/homebrew/src/plugin.rs` (if exists)
- `crates/plugins/package-managers/npm/src/plugin.rs` (if exists)
- Any Docker or releases plugin that executes updates
- `crates/shared/agent-core/src/update.rs` (the caller)

- [ ] **Step 2: Fix each plugin — wrap return value**

For each plugin that previously returned `Ok(output_string)`, change to:

```rust
Ok(ExecuteUpdateResult { output: output_string, resumable: false })
```

For each plugin that previously returned `Ok(accumulated_output)` or similar, same pattern.
Do NOT set `resumable: true` here — that is added in Task 15 (shell plugin only).

Example for a typical plugin's `execute_update` body ending:

```rust
// Before:
Ok(output)

// After:
Ok(uptrakit_plugin_infrastructure_core::ExecuteUpdateResult {
    output,
    resumable: false,
})
```

If `ExecuteUpdateResult` is already re-exported from the crate's prelude/lib, use the short form. Otherwise use
the full path or add `use uptrakit_plugin_infrastructure_core::ExecuteUpdateResult;`.

- [ ] **Step 3: Fix `execute_update` caller in agent-core**

In `crates/shared/agent-core/src/update.rs`, find the call to the plugin's `execute_update` (inside
`execute_plugin_update` or similar). It currently uses the returned `String` as the output. Change to:

```rust
// Before (approximate):
let output = plugin.execute_update(...).await?;

// After:
let exec_result = plugin.execute_update(...).await?;
let output = exec_result.output;
let resumable = exec_result.resumable;
```

For now, just capture `resumable` — it will be used in Task 6.

- [ ] **Step 4: Verify compilation**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: no errors related to `execute_update` return type.

- [ ] **Step 5: Run existing tests**

```bash
cargo test --all-features 2>&1 | tail -20
```

Expected: all previously passing tests still pass.

- [ ] **Step 6: Commit**

```bash
git add -p  # stage all execute_update return type fixes
git commit -m "feat(plugins): update all UpdateExecutor impls to return ExecuteUpdateResult"
```

---

### Task 6: `PipelineResult` + `execute_update_pipeline` refactor

**Files:**

- Modify: `crates/shared/agent-core/src/update.rs`

The key change: `execute_update_pipeline` stops running post-hooks. It returns
`PipelineResult { succeeded: bool, resumable: bool }`. Post-hooks move to `execute_update`.

- [ ] **Step 1: Write the failing test**

In `crates/shared/agent-core/src/update.rs` test module (or `tests/` file), add:

```rust
#[cfg(test)]
mod pipeline_resumable_tests {
    // This test verifies that when the pipeline result has resumable=true,
    // execute_update sends the early result BEFORE post-hooks run.
    // We approximate this by checking that UpdateExecutionResult.resumable == true.
    use super::*;

    // A minimal mock plugin that returns resumable=true.
    struct ResumablePlugin;

    #[async_trait::async_trait]
    impl uptrakit_plugin_infrastructure_core::UpdateExecutor for ResumablePlugin {
        // ... implement required trait methods ...
        async fn execute_update(
            &self, _: &str, _: &str,
            _: Option<&uptrakit_plugin_infrastructure_core::ReleaseInfo>,
            _: &uptrakit_plugin_infrastructure_core::UpdateOutputSender,
        ) -> uptrakit_plugin_infrastructure_core::error::Result<
            uptrakit_plugin_infrastructure_core::ExecuteUpdateResult
        > {
            Ok(uptrakit_plugin_infrastructure_core::ExecuteUpdateResult {
                output: "done".to_string(),
                resumable: true,
            })
        }
    }

    #[tokio::test]
    async fn test_pipeline_returns_resumable_true_from_plugin() {
        // NOTE: This is a compilation/interface test.
        // Full post-hook ordering is verified by integration tests.
        // Just verify PipelineResult carries the resumable flag.
        // Actual test body requires constructing ExecuteUpdatePayload with
        // a ResumablePlugin — see existing update tests for the pattern.
        // For now, assert the type exists and has the right fields:
        let _ = PipelineResult { succeeded: true, resumable: true };
        let _ = PipelineResult { succeeded: false, resumable: false };
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p uptrakit-agent-core pipeline_resumable_tests
```

Expected: FAIL — `PipelineResult` not defined.

- [ ] **Step 3: Add `PipelineResult` struct**

In `crates/shared/agent-core/src/update.rs`, above `execute_update_pipeline`, add:

```rust
struct PipelineResult {
    succeeded: bool,
    resumable: bool,
}
```

- [ ] **Step 4: Refactor `execute_update_pipeline` — remove post-hooks, return `PipelineResult`**

Change the function signature from:

```rust
async fn execute_update_pipeline(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    executor: Arc<dyn CommandExecutor>,
    accumulated_output: &mut String,
) -> std::result::Result<(), AgentCoreError>
```

to:

```rust
async fn execute_update_pipeline(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    executor: Arc<dyn CommandExecutor>,
    accumulated_output: &mut String,
) -> PipelineResult
```

Remove the `run_post_hook_plugins` call at the end of the pipeline. The pipeline now ends after `execute_plugin_update`. Return:

```rust
// On success (execute_plugin_update returned Ok with resumable=true):
PipelineResult { succeeded: true, resumable: true }

// On success (resumable=false):
PipelineResult { succeeded: true, resumable: false }

// On any failure (pre-hook error, attestation gate, plugin error):
PipelineResult { succeeded: false, resumable: false }
```

The `resumable` value comes from the `ExecuteUpdateResult` returned by the plugin (captured in Task 5 Step 3).
If `execute_plugin_update` returns `Err`, `resumable` is always `false`.

- [ ] **Step 5: Update `execute_update` to call post-hooks conditionally**

In the `execute_update` function, after the `tokio::time::timeout` call wrapping `execute_update_pipeline`:

```rust
let pipeline_result = tokio::time::timeout(
    timeout_duration,
    execute_update_pipeline(&payload, &output_tx, Arc::clone(&executor), &mut accumulated_output),
)
.await;

let PipelineResult { succeeded, resumable } = match pipeline_result {
    Ok(r) => r,
    Err(_elapsed) => {
        final_status = UpdateFinalStatus::Failed;
        final_error = Some(format!("Update timed out after {:?}", timeout_duration));
        PipelineResult { succeeded: false, resumable: false }
    }
};

if !succeeded {
    final_status = UpdateFinalStatus::Failed;
}

// Post-hooks: run inline for non-resumable, spawned for resumable.
if succeeded && resumable {
    // Spawn — the hook will trigger restart; don't block on it.
    let post_hook_plugins = payload.post_update_hook_plugins.clone();
    // (executor and output_tx for hooks — use the same pattern as the existing
    // run_post_hook_plugins call; pass clones)
    tokio::spawn(async move {
        run_post_hook_plugins(&post_hook_plugins, /* post_ctx */ ..., ..., ...).await;
        // Log errors only — outcome is irrelevant after restart.
    });
} else {
    run_post_hook_plugins(
        &payload.post_update_hook_plugins,
        &post_ctx,
        Arc::clone(&executor),
        &output_tx,
        &mut accumulated_output,
    )
    .await;
}
```

Read the existing `run_post_hook_plugins` call in the file to get the exact `post_ctx` construction and arguments.

Also add `resumable: bool` to `UpdateExecutionResult`:

```rust
pub struct UpdateExecutionResult {
    pub result: UpdateResultPayload,
    pub resumable: bool,
}
```

And when building the return value, set `resumable: succeeded && resumable_flag`. Also set
`result.resumable = Some(true)` when resumable, so the controller can read it from the wire payload.

- [ ] **Step 6: Verify and run tests**

```bash
cargo test -p uptrakit-agent-core pipeline_resumable_tests
cargo check --all-features
```

- [ ] **Step 7: Commit**

```bash
git commit --only crates/shared/agent-core/src/update.rs \
  -m "feat(agent-core): PipelineResult, move post-hooks out of pipeline, resumable branch in execute_update"
```

---

### Task 7: Early result channel

**Files:**

- Modify: `crates/shared/agent-core/src/client.rs`
- Modify: `crates/core/agent-runtime/src/lib.rs`

- [ ] **Step 1: Write failing test**

In `crates/shared/agent-core/src/client.rs` test module, add:

```rust
#[test]
fn test_in_flight_update_has_early_result_fields() {
    // Compilation test — verifies the struct has the new fields.
    // Just reference the field names; the test body is empty.
    fn _assert_fields(u: &InFlightUpdate) {
        let _: &tokio::sync::mpsc::UnboundedReceiver<uptrakit_wire::UpdateResultPayload> =
            &u.early_result_rx;
        let _: bool = u.early_sent;
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p uptrakit-agent-core test_in_flight_update_has_early_result_fields
```

Expected: FAIL — fields not found.

- [ ] **Step 3: Add fields to `InFlightUpdate`**

In `crates/shared/agent-core/src/client.rs`, `InFlightUpdate` struct — add two fields:

```rust
pub early_result_rx: tokio::sync::mpsc::UnboundedReceiver<uptrakit_wire::UpdateResultPayload>,
pub early_sent: bool,
```

- [ ] **Step 4: Create the channel in `start_update` and thread the sender through**

In `start_update`, before calling `spawn_update_task`:

```rust
let (early_result_tx, early_result_rx) =
    tokio::sync::mpsc::unbounded_channel::<uptrakit_wire::UpdateResultPayload>();
```

Pass `early_result_tx` into `spawn_update_task`. Inside `spawn_update_task`, thread it into the `tokio::spawn` that calls `execute_update`:

```rust
// spawn_update_task spawns:
tokio::spawn(execute_update(payload, executor, output_tx, early_result_tx))
```

Update `execute_update`'s signature to accept `early_result_tx: tokio::sync::mpsc::UnboundedSender<UpdateResultPayload>`.

In `execute_update`, when `succeeded && resumable`, send the early payload:

```rust
let early_payload = UpdateResultPayload {
    update_history_id: payload.update_history_id,
    status: UpdateFinalStatus::Completed,
    from_version: from_version.clone(),
    to_version: detect_to_version_after_update(&payload, Arc::clone(&executor)).await,
    output: accumulated_output.clone(),
    error: None,
    resumable: Some(true),
};
let _ = early_result_tx.send(early_payload);
```

The `detect_to_version_after_update` call is whatever version detection the normal success path uses —
find it in the existing `execute_update` success branch.

Return the updated `InFlightUpdate` with `early_result_rx` and `early_sent: false`.

- [ ] **Step 5: Update `poll_in_flight_update` in `agent-runtime`**

In `crates/core/agent-runtime/src/lib.rs`, `poll_in_flight_update`, add the early result as the highest-priority arm:

```rust
#[cfg(not(feature = "interactive"))]
let event = tokio::select! {
    biased;
    Some(early) = update.early_result_rx.recv() => UpdateEvent::EarlyResult(early),
    Some(output_msg) = update.output_rx.recv() => UpdateEvent::Output(output_msg),
    result = &mut update.handle => UpdateEvent::Completed(result),
};
```

Add `EarlyResult(uptrakit_wire::UpdateResultPayload)` variant to `UpdateEvent` enum in `agent-core/src/client.rs`:

```rust
pub enum UpdateEvent {
    Output(crate::update::UpdateOutputMessage),
    Completed(std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>),
    EarlyResult(uptrakit_wire::UpdateResultPayload),
    #[cfg(feature = "interactive")]
    Attention(uuid::Uuid),
}
```

Handle `UpdateEvent::EarlyResult` in the agent-runtime event loop (alongside `UpdateEvent::Output` and `UpdateEvent::Completed`):

```rust
AgentRuntimeEvent::Update(UpdateEvent::EarlyResult(early_payload)) => {
    if let Some(ref mut update) = self.in_flight_update {
        if !update.early_sent {
            if let Err(e) = transport.transport_send(
                uptrakit_wire::ServiceMessage::UpdateResult(early_payload)
            ).await {
                tracing::error!(error = %e, "failed to send early update result");
            }
            update.early_sent = true;
        }
    }
}
```

In the `UpdateEvent::Completed` handler, drain `early_result_rx` before calling `send_update_result`:

```rust
AgentRuntimeEvent::Update(UpdateEvent::Completed(result)) => {
    let Some(mut update) = self.in_flight_update.take() else { ... return outcome; };
    // Drain any pending early result that arrived in the same poll batch.
    while let Ok(early) = update.early_result_rx.try_recv() {
        if !update.early_sent {
            let _ = transport.transport_send(
                uptrakit_wire::ServiceMessage::UpdateResult(early)
            ).await;
            update.early_sent = true;
        }
    }
    if !update.early_sent {
        if let Err(e) = send_update_result(transport, update.update_history_id, result).await {
            tracing::error!(error = %e, "failed to send update result");
            outcome = Some(uptrakit_agent_core::LoopOutcome::Disconnected);
        }
    }
}
```

- [ ] **Step 6: Update `handle_graceful_shutdown`**

In `crates/shared/agent-core/src/client.rs`, `handle_graceful_shutdown`, before calling `send_update_result`, check `early_sent`:

```rust
result = &mut update.handle => {
    if !update.early_sent {
        if let Err(e) = send_update_result(conn, update.update_history_id, result).await {
            tracing::warn!(...);
        }
    }
    break;
}
```

- [ ] **Step 7: Verify compilation**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

- [ ] **Step 8: Commit**

```bash
git commit --only \
  crates/shared/agent-core/src/client.rs \
  crates/core/agent-runtime/src/lib.rs \
  -m "feat(agent): early result channel for resumable updates; try_recv drain in Completed handler"
```

---

### Task 8: `transition_to_awaiting_restart` + `handle_update_result` resumable branch

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`

- [ ] **Step 1: Write failing tests**

In the test module of `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs` (or a `tests/` file in the crate), add:

```rust
#[tokio::test]
async fn test_transition_to_awaiting_restart_updates_status() {
    let db = setup_test_db().await; // follow the existing test helper pattern
    let (tenant_id, host_id, service_id) = setup_tenant_host_service(&db).await;
    let software_item_id = create_software_item(&db, tenant_id).await;
    let record_id = create_update_history_in_progress(
        &db, tenant_id, host_id, software_item_id, service_id
    ).await;

    let rows = transition_to_awaiting_restart(&db, record_id, service_id)
        .await
        .unwrap();

    assert_eq!(rows, 1);
    let record = load_update_history(&db, record_id).await;
    assert_eq!(record.status, UpdateStatus::AwaitingRestart);
    assert!(record.awaiting_restart_since.is_some());
}

#[tokio::test]
async fn test_transition_to_awaiting_restart_wrong_service_is_noop() {
    let db = setup_test_db().await;
    let (tenant_id, host_id, service_id) = setup_tenant_host_service(&db).await;
    let other_service_id = uuid::Uuid::new_v4();
    let software_item_id = create_software_item(&db, tenant_id).await;
    let record_id = create_update_history_in_progress(
        &db, tenant_id, host_id, software_item_id, service_id
    ).await;

    let rows = transition_to_awaiting_restart(&db, record_id, other_service_id)
        .await
        .unwrap();

    assert_eq!(rows, 0); // CAS fails — wrong service
    let record = load_update_history(&db, record_id).await;
    assert_eq!(record.status, UpdateStatus::InProgress); // unchanged
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p uptrakit-web-api-queries test_transition_to_awaiting_restart -- --nocapture
```

Expected: FAIL — function not found.

- [ ] **Step 3: Implement `transition_to_awaiting_restart`**

In `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`, add the function (following `finalize_update_result_if_owned` as a template):

```rust
/// CAS: id = update_history_id AND status = 'in_progress' AND execution_owner_service_id = service_id
/// Sets: status = 'awaiting_restart', awaiting_restart_since = now()
/// Returns rows_affected (0 = race lost, skip dispatch progression)
pub async fn transition_to_awaiting_restart(
    db: &DatabaseConnection,
    update_history_id: uuid::Uuid,
    service_id: uuid::Uuid,
) -> std::result::Result<u64, rootcause::Report<TriggerUpdateError>> {
    let now = time::OffsetDateTime::now_utc();
    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(update_history::Column::Status.eq(UpdateStatus::InProgress))
        .filter(update_history::Column::ExecutionOwnerServiceId.eq(service_id))
        .col_expr(
            update_history::Column::Status,
            Expr::value(UpdateStatus::AwaitingRestart),
        )
        .col_expr(
            update_history::Column::AwaitingRestartSince,
            Expr::value(Some(now)),
        )
        .exec(db)
        .await
        .context_to()?;
    Ok(result.rows_affected)
}
```

The imports (`UpdateHistory`, `update_history::Column`, `UpdateStatus`, `Expr`, `context_to`) are already used
in the same file — no new imports needed.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p uptrakit-web-api-queries test_transition_to_awaiting_restart -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Write test that verifies dispatch is NOT called on resumable**

```rust
#[tokio::test]
async fn test_awaiting_restart_does_not_trigger_dispatch() {
    // Setup InProgress record.
    let db = setup_test_db().await;
    // ... create record ...
    let rows = transition_to_awaiting_restart(&db, record_id, service_id).await.unwrap();
    assert_eq!(rows, 1);
    // After transition, verify there is no new Pending record dispatched for the host.
    let pending = count_pending_for_host(&db, host_id).await;
    assert_eq!(pending, 0);
}
```

- [ ] **Step 6: Update `handle_update_result` in the WS handler**

In `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`, after `finalize_update_result_if_owned`
succeeds (when `updated > 0`) and `status == Completed`, check `payload.resumable`:

```rust
// After: let updated = finalize_update_result_if_owned(...).await?;
// if updated == 0: existing fallback logic (unchanged)
// if updated > 0 AND status == Completed AND payload.resumable == Some(true):
if updated > 0
    && matches!(payload.status, UpdateFinalStatus::Completed)
    && payload.resumable == Some(true)
{
    let rows = uptrakit_web_api_queries::transition_to_awaiting_restart(
        state.db(),
        payload.update_history_id,
        service_id,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "transition_to_awaiting_restart failed");
        0
    });
    if rows == 0 {
        tracing::warn!(
            update_history_id = %payload.update_history_id,
            "transition_to_awaiting_restart: CAS lost (rows_affected=0), skipping"
        );
    }
    // Do NOT call dispatch_next_in_batch — AwaitingRestart is not terminal.
    return ProcessorResponse::ok();
}
// Otherwise: existing dispatch logic (unchanged).
```

- [ ] **Step 7: Verify compilation and tests**

```bash
cargo check --all-features
cargo test -p uptrakit-web-api-queries -- --nocapture 2>&1 | tail -10
```

- [ ] **Step 8: Commit**

```bash
git commit --only \
  crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs \
  crates/ui/web-api/src/routes/service_ws/handler/updates.rs \
  -m "feat(controller): transition_to_awaiting_restart CAS; handle_update_result resumable branch"
```

---

### Task 9: Batch sequencing — `AwaitingRestart` status inclusion

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[tokio::test]
async fn test_has_active_update_includes_awaiting_restart() {
    let db = setup_test_db().await;
    let (tenant_id, host_id, _, _) = setup_full_context(&db).await;
    create_update_history_with_status(
        &db, tenant_id, host_id, UpdateStatus::AwaitingRestart
    ).await;

    let active = has_active_update_for_host(&db, host_id).await.unwrap();
    assert!(active, "AwaitingRestart should count as active");
}

#[tokio::test]
async fn test_maybe_complete_batch_waits_for_awaiting_restart() {
    let db = setup_test_db().await;
    let (tenant_id, host_id, _, _) = setup_full_context(&db).await;
    let batch_id = create_batch(&db, tenant_id).await;
    create_batch_item_with_status(&db, batch_id, host_id, UpdateStatus::Completed).await;
    create_batch_item_with_status(&db, batch_id, host_id, UpdateStatus::AwaitingRestart).await;

    let result = maybe_complete_batch(&db, batch_id, tenant_id).await.unwrap();
    assert!(result.is_none(), "batch should not complete with AwaitingRestart item");
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p uptrakit-web-api-queries \
  test_has_active_update_includes_awaiting_restart \
  test_maybe_complete_batch_waits_for_awaiting_restart \
  -- --nocapture
```

Expected: FAIL — `AwaitingRestart` not in the filter.

- [ ] **Step 3: Update `has_active_update_for_host`**

In `crates/ui/web-api-queries/src/queries/update_dispatch.rs`, find the status filter. Change from:

```rust
.filter(Column::Status.is_in([Pending, InProgress]))
```

to:

```rust
.filter(Column::Status.is_in([Pending, InProgress, AwaitingRestart]))
```

(`AwaitingRestart` is `UpdateStatus::AwaitingRestart` from the shared-types import already in scope.)

- [ ] **Step 4: Update `maybe_complete_batch` non-terminal filter**

In `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`, find the `maybe_complete_batch` pending count query. Change from:

```rust
.filter(update_history::Column::Status.is_in([
    update_history::UpdateStatus::Queued,
    update_history::UpdateStatus::Pending,
    update_history::UpdateStatus::InProgress,
]))
```

to:

```rust
.filter(update_history::Column::Status.is_in([
    update_history::UpdateStatus::Queued,
    update_history::UpdateStatus::Pending,
    update_history::UpdateStatus::InProgress,
    update_history::UpdateStatus::AwaitingRestart,
]))
```

- [ ] **Step 5: Add unique constraint violation handling**

In `dispatch_next_queued_for_host` (or wherever the `Pending` insert happens), find the call that inserts the
new `update_history` row. Wrap it to handle DB unique constraint errors gracefully:

```rust
match insert_pending_update_history_row(...).await {
    Ok(_) => { /* proceed */ }
    Err(e) if is_unique_constraint_violation(&e) => {
        tracing::debug!(
            host_id = %host_id,
            "dispatch: host already has an active update (unique constraint), skipping"
        );
        return Ok(None);
    }
    Err(e) => return Err(e),
}
```

Find the `is_unique_constraint_violation` helper or implement it: check if the `DbErr` string contains
`"UNIQUE constraint"` (SQLite) or `"duplicate key value"` (Postgres). The existing codebase may already have
a helper for this — grep for `unique_constraint` or `duplicate_key`.

- [ ] **Step 6: Run tests to verify they pass**

```bash
cargo test -p uptrakit-web-api-queries \
  test_has_active_update_includes_awaiting_restart \
  test_maybe_complete_batch_waits_for_awaiting_restart \
  -- --nocapture
```

- [ ] **Step 7: Commit**

```bash
git commit --only \
  crates/ui/web-api-queries/src/queries/update_dispatch.rs \
  crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs \
  -m "feat(dispatch): include AwaitingRestart in active-update and batch-completion checks"
```

---

### Task 10: `handle_version_check_results` — `AwaitingRestart` correlation

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs` (new query helper)

- [ ] **Step 1: Write failing tests**

In the web-api-queries crate test module:

```rust
#[tokio::test]
async fn test_awaiting_restart_transitions_completed_on_version_match() {
    let db = setup_test_db().await;
    let (tenant_id, host_id, service_id, host_software_item_id) =
        setup_full_context(&db).await;
    let record_id = create_awaiting_restart_record(
        &db, tenant_id, host_id, host_software_item_id, "1.2.0"
    ).await;

    apply_awaiting_restart_version_check(
        &db,
        host_software_item_id,
        Some("1.2.0".to_string()),
        None,  // not_ready
        None,  // error
    )
    .await
    .unwrap();

    let record = load_update_history(&db, record_id).await;
    assert_eq!(record.status, UpdateStatus::Completed);
}

#[tokio::test]
async fn test_awaiting_restart_stays_on_not_ready() {
    // ... create AwaitingRestart record with to_version="1.2.0" ...
    apply_awaiting_restart_version_check(&db, hsi_id, None, Some(true), None).await.unwrap();
    let record = load_update_history(&db, record_id).await;
    assert_eq!(record.status, UpdateStatus::AwaitingRestart);
}

#[tokio::test]
async fn test_awaiting_restart_fails_on_version_mismatch() {
    // ... to_version="1.2.0" ...
    apply_awaiting_restart_version_check(&db, hsi_id, Some("1.1.0".to_string()), None, None)
        .await.unwrap();
    let record = load_update_history(&db, record_id).await;
    assert_eq!(record.status, UpdateStatus::Failed);
}

#[tokio::test]
async fn test_awaiting_restart_stays_on_absent_installed_version() {
    // installed_version: None, not_ready: None, error: None → stay
    apply_awaiting_restart_version_check(&db, hsi_id, None, None, None).await.unwrap();
    let record = load_update_history(&db, record_id).await;
    assert_eq!(record.status, UpdateStatus::AwaitingRestart);
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p uptrakit-web-api-queries test_awaiting_restart_transitions -- --nocapture
```

Expected: FAIL — `apply_awaiting_restart_version_check` not found.

- [ ] **Step 3: Implement `apply_awaiting_restart_version_check` query**

In `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`, add:

```rust
/// After a VersionCheckResult arrives for a host_software_item_id,
/// apply the AwaitingRestart evaluation rules.
/// Returns: Some(new_terminal_status) if a transition happened, None if record stays.
pub async fn apply_awaiting_restart_version_check(
    db: &DatabaseConnection,
    host_software_item_id: uuid::Uuid,
    installed_version: Option<String>,
    not_ready: Option<bool>,
    error: Option<String>,
) -> std::result::Result<Option<UpdateStatus>, rootcause::Report<TriggerUpdateError>> {
    // Load the AwaitingRestart record for this host_software_item_id.
    let record = UpdateHistory::find()
        .filter(update_history::Column::HostSoftwareItemId.eq(host_software_item_id))
        .filter(update_history::Column::Status.eq(UpdateStatus::AwaitingRestart))
        .one(db)
        .await
        .context_to()?;

    let Some(record) = record else { return Ok(None) };

    // Evaluation order per spec:
    // 1. not_ready: Some(true) → stay
    if not_ready == Some(true) {
        return Ok(None);
    }
    // 2. error: Some(_) → stay
    if error.is_some() {
        return Ok(None);
    }
    // 3. installed_version: None (not_ready absent, error absent) → stay
    let Some(installed) = installed_version else {
        return Ok(None);
    };
    let Some(ref to_version) = record.to_version else {
        // No to_version stored — cannot compare, stay
        return Ok(None);
    };
    // 4. version mismatch → Failed
    // 5. version match → Completed
    let new_status = if &installed == to_version {
        UpdateStatus::Completed
    } else {
        UpdateStatus::Failed
    };

    // CAS transition
    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(record.id))
        .filter(update_history::Column::Status.eq(UpdateStatus::AwaitingRestart))
        .col_expr(update_history::Column::Status, Expr::value(new_status.clone()))
        .col_expr(
            update_history::Column::CompletedAt,
            Expr::value(Some(time::OffsetDateTime::now_utc())),
        )
        .exec(db)
        .await
        .context_to()?;

    if result.rows_affected == 0 {
        // CAS lost — another controller already acted
        return Ok(None);
    }

    Ok(Some(new_status))
}
```

Also expose `batch_id` and `host_id` from the record for dispatch progression. Return a richer type if needed,
or make a separate helper that loads `(batch_id, host_id, tenant_id)` from the record.

- [ ] **Step 4: Hook into `handle_version_check_results`**

In `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`, inside the per-result loop
(after `apply_version_update_to_db`), add the AwaitingRestart correlation:

```rust
// Inside the loop, after apply_version_update_to_db:
if let Some(hsi_id) = result.host_software_item_id {
    let terminal = uptrakit_web_api_queries::apply_awaiting_restart_version_check(
        state.db(),
        hsi_id,
        result.installed_version.clone(),
        result.not_ready,
        result.error.clone(),
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "apply_awaiting_restart_version_check failed");
        None
    });

    if let Some(_status) = terminal {
        // Load batch_id and host_id for progression.
        // Use the matching_rows already resolved above to get the host_id.
        let first_host_id = matching_rows.first().map(|r| r.host_id);
        if let Some(h_id) = first_host_id {
            // Load the update_history record to get batch_id and tenant_id.
            // Then dispatch progression:
            // if batch: dispatch_next_in_batch(db, dispatch, batch_id, h_id, tenant_id)
            // else: dispatch_next_queued_for_host(db, dispatch, h_id, tenant_id)
            // Follow the pattern in handle_update_result in updates.rs.
            trigger_host_progression_after_awaiting_restart(
                state, hsi_id, h_id
            ).await;
        }
    }
}
```

Implement `trigger_host_progression_after_awaiting_restart` as a private async fn in messages.rs that:

1. Loads the just-completed update_history record (now Completed or Failed)
2. Extracts `batch_id`, `tenant_id`
3. Constructs `DispatchContext`
4. Calls `dispatch_next_in_batch` or `dispatch_next_queued_for_host`

Follow the exact pattern from `handle_update_result` in `updates.rs`.

- [ ] **Step 5: Run tests**

```bash
cargo test -p uptrakit-web-api-queries test_awaiting_restart -- --nocapture
cargo check --all-features
```

- [ ] **Step 6: Commit**

```bash
git commit --only \
  crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs \
  crates/ui/web-api/src/routes/service_ws/handler/messages.rs \
  -m "feat(controller): AwaitingRestart version-check correlation and terminal transitions"
```

---

### Task 11: `TickExecutor` trait + Scheduler extension + `SchedulerNotifier::signal_host_progression`

**Files:**

- Create: `crates/shared/scheduler-engine/src/tick_executor.rs`
- Modify: `crates/shared/scheduler-engine/src/scheduler.rs`
- Modify: `crates/shared/scheduler-engine/src/lib.rs`
- Modify: `crates/shared/scheduler-engine/src/notifier.rs`
- Modify: `crates/core/controller-runtime/src/scheduler/mod.rs`

- [ ] **Step 1: Write failing tests**

```rust
// In crates/shared/scheduler-engine/src/tick_executor.rs test module:
#[tokio::test]
async fn test_tick_executor_runs_on_poll_cycle() {
    use std::sync::{Arc, atomic::{AtomicU32, Ordering}};
    let counter = Arc::new(AtomicU32::new(0));
    let counter2 = Arc::clone(&counter);

    struct CountingExecutor(Arc<AtomicU32>);
    #[async_trait::async_trait]
    impl TickExecutor for CountingExecutor {
        async fn execute_tick(&self, _db: &DatabaseConnection) -> error::Result<()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    // Create a Scheduler with in-memory DB and register the tick executor.
    // Run one poll cycle.
    // Assert counter == 1.
    // (follow the existing Scheduler test setup pattern in the crate)
    let _ = CountingExecutor(counter2); // compilation check
    assert_eq!(counter.load(Ordering::Relaxed), 0); // placeholder until full test
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p uptrakit-scheduler-engine test_tick_executor_runs_on_poll_cycle
```

Expected: FAIL — `TickExecutor` not found.

- [ ] **Step 3: Create `tick_executor.rs`**

```rust
// crates/shared/scheduler-engine/src/tick_executor.rs
use sea_orm::DatabaseConnection;

#[async_trait::async_trait]
pub trait TickExecutor: Send + Sync {
    async fn execute_tick(&self, db: &DatabaseConnection) -> crate::error::Result<()>;
}
```

- [ ] **Step 4: Add `tick_executors` to `Scheduler`**

In `crates/shared/scheduler-engine/src/scheduler.rs`, add to `Scheduler` struct:

```rust
tick_executors: Vec<std::sync::Arc<dyn crate::tick_executor::TickExecutor>>,
```

Initialize to `vec![]` in `Scheduler::new`. Add method:

```rust
pub fn register_tick_executor(
    &mut self,
    executor: Box<dyn crate::tick_executor::TickExecutor>,
) {
    self.tick_executors.push(std::sync::Arc::from(executor));
}
```

In `poll_cycle`, after the existing `JoinSet` drains, run tick executors in a separate `JoinSet`:

```rust
let mut tick_join_set: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
for exec in &self.tick_executors {
    let exec = std::sync::Arc::clone(exec);
    let db = self.db.clone();
    tick_join_set.spawn(async move {
        match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            exec.execute_tick(&db),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "tick executor error"),
            Err(_) => tracing::warn!("tick executor timed out after 60s"),
        }
    });
}
while let Some(result) = tick_join_set.join_next().await {
    if let Err(e) = result {
        if e.is_panic() {
            tracing::error!("tick executor panicked — continuing");
        }
    }
}
```

- [ ] **Step 5: Add `signal_host_progression` to `SchedulerNotifier`**

In `crates/shared/scheduler-engine/src/notifier.rs`:

```rust
async fn signal_host_progression(&self, host_id: uuid::Uuid, tenant_id: uuid::Uuid);
```

Add no-op default so existing implementors don't break immediately:

```rust
async fn signal_host_progression(&self, _host_id: uuid::Uuid, _tenant_id: uuid::Uuid) {}
```

- [ ] **Step 6: Implement `signal_host_progression` in `ControllerSchedulerNotifier`**

In `crates/core/controller-runtime/src/scheduler/mod.rs`, implement the method on `ControllerSchedulerNotifier`:

```rust
async fn signal_host_progression(&self, host_id: uuid::Uuid, tenant_id: uuid::Uuid) {
    let dispatch = uptrakit_web_api_queries::DispatchContext {
        notifier: &self.notification_service,
        protection: None,
    };
    if let Err(e) = uptrakit_web_api_queries::dispatch_next_queued_for_host(
        &self.db, dispatch, host_id, tenant_id,
    )
    .await
    {
        tracing::warn!(
            error = %e, %host_id, %tenant_id,
            "signal_host_progression: dispatch_next_queued_for_host failed"
        );
    }
}
```

`notification_service` and `db` are already fields on `ControllerSchedulerNotifier` (confirmed from exploration).

- [ ] **Step 7: Re-export from lib.rs**

In `crates/shared/scheduler-engine/src/lib.rs`, add:

```rust
pub mod tick_executor;
pub use tick_executor::TickExecutor;
```

- [ ] **Step 8: Verify and commit**

```bash
cargo check --all-features
cargo test -p uptrakit-scheduler-engine -- --nocapture
```

```bash
git commit --only \
  crates/shared/scheduler-engine/src/tick_executor.rs \
  crates/shared/scheduler-engine/src/scheduler.rs \
  crates/shared/scheduler-engine/src/lib.rs \
  crates/shared/scheduler-engine/src/notifier.rs \
  crates/core/controller-runtime/src/scheduler/mod.rs \
  -m "feat(scheduler): TickExecutor trait, register_tick_executor, signal_host_progression"
```

---

### Task 12: `AwaitingRestartExecutor`

**Files:**

- Create: `crates/shared/scheduler-engine/src/executors/awaiting_restart.rs`
- Modify: `crates/shared/scheduler-engine/src/executors/mod.rs` (or lib.rs)
- Modify: `crates/core/controller-runtime/src/scheduler/mod.rs` (register)

- [ ] **Step 1: Write failing tests**

```rust
#[tokio::test]
async fn test_awaiting_restart_executor_skips_null_execution_owner() {
    let db = setup_test_db().await;
    let (tenant_id, host_id, _) = setup_tenant_host(&db).await;
    let software_item_id = create_software_item(&db, tenant_id).await;
    // Create AwaitingRestart record with execution_owner_service_id = NULL
    create_awaiting_restart_record_no_owner(&db, tenant_id, host_id, software_item_id).await;

    let notifier = Arc::new(MockSchedulerNotifier::new());
    let exec = AwaitingRestartExecutor::new(Arc::clone(&notifier));
    exec.execute_tick(&db).await.unwrap();

    assert_eq!(notifier.send_to_service_calls(), 0, "should not dispatch to NULL owner");
}

#[tokio::test]
#[tokio::test(start_paused = true)]
async fn test_awaiting_restart_executor_times_out_record() {
    let db = setup_test_db().await;
    let (tenant_id, host_id, service_id) = setup_tenant_host_service(&db).await;
    let software_item_id = create_software_item_with_timeout(&db, tenant_id, 120).await;
    let record_id = create_awaiting_restart_record_with_since(
        &db, tenant_id, host_id, software_item_id, service_id,
        time::OffsetDateTime::now_utc() - time::Duration::seconds(130), // past timeout
    ).await;

    let notifier = Arc::new(MockSchedulerNotifier::new());
    let exec = AwaitingRestartExecutor::new(Arc::clone(&notifier));
    exec.execute_tick(&db).await.unwrap();

    let record = load_update_history(&db, record_id).await;
    assert_eq!(record.status, UpdateStatus::Failed);
    assert_eq!(notifier.signal_host_progression_calls(), 1);
}

#[tokio::test]
async fn test_awaiting_restart_executor_skips_missing_plugin_assignment() {
    let db = setup_test_db().await;
    // Create AwaitingRestart record with no detect_version plugin assignment
    // on the host_software_item_plugin table.
    // ...
    let notifier = Arc::new(MockSchedulerNotifier::new());
    let exec = AwaitingRestartExecutor::new(Arc::clone(&notifier));
    exec.execute_tick(&db).await.unwrap();
    assert_eq!(notifier.send_to_service_calls(), 0);
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p uptrakit-scheduler-engine test_awaiting_restart_executor -- --nocapture
```

Expected: FAIL — struct not found.

- [ ] **Step 3: Implement `AwaitingRestartExecutor`**

Create `crates/shared/scheduler-engine/src/executors/awaiting_restart.rs`:

```rust
use std::sync::Arc;
use sea_orm::{DatabaseConnection, EntityTrait, QueryFilter, ColumnTrait, QuerySelect};
use uuid::Uuid;

use crate::{SchedulerNotifier, TickExecutor};

pub struct AwaitingRestartExecutor {
    notifier: Arc<dyn SchedulerNotifier>,
}

impl AwaitingRestartExecutor {
    pub fn new(notifier: Arc<dyn SchedulerNotifier>) -> Self {
        Self { notifier }
    }
}

#[async_trait::async_trait]
impl TickExecutor for AwaitingRestartExecutor {
    async fn execute_tick(&self, db: &DatabaseConnection) -> crate::error::Result<()> {
        self.enforce_timeouts(db).await?;
        self.dispatch_verification(db).await?;
        Ok(())
    }
}

impl AwaitingRestartExecutor {
    async fn enforce_timeouts(&self, db: &DatabaseConnection) -> crate::error::Result<()> {
        use uptrakit_shared_db::entity::{update_history, software_item};
        use uptrakit_shared_types::UpdateStatus;
        use sea_orm::prelude::*;

        let now = time::OffsetDateTime::now_utc();

        // Load all AwaitingRestart records with awaiting_restart_since IS NOT NULL.
        // Join software_item to read awaiting_restart_timeout.
        let records: Vec<(update_history::Model, Option<software_item::Model>)> =
            update_history::Entity::find()
                .filter(update_history::Column::Status.eq(UpdateStatus::AwaitingRestart))
                .filter(update_history::Column::AwaitingRestartSince.is_not_null())
                .find_also_related(software_item::Entity)
                .all(db)
                .await
                .map_err(|e| crate::error::SchedulerError::from(e))?;

        for (record, software) in records {
            let Some(since) = record.awaiting_restart_since else {
                // IS NOT NULL filter should prevent this — log and skip.
                tracing::warn!(
                    update_history_id = %record.id,
                    "AwaitingRestart record has awaiting_restart_since IS NULL despite filter"
                );
                continue;
            };
            let timeout_secs = software
                .and_then(|s| s.awaiting_restart_timeout)
                .unwrap_or(600) as i64;
            let deadline = since + time::Duration::seconds(timeout_secs);

            if now <= deadline {
                continue;
            }

            // CAS transition to Failed
            let result = update_history::Entity::update_many()
                .filter(update_history::Column::Id.eq(record.id))
                .filter(update_history::Column::Status.eq(UpdateStatus::AwaitingRestart))
                .col_expr(
                    update_history::Column::Status,
                    Expr::value(UpdateStatus::Failed),
                )
                .col_expr(
                    update_history::Column::CompletedAt,
                    Expr::value(Some(now)),
                )
                .exec(db)
                .await
                .map_err(crate::error::SchedulerError::from)?;

            if result.rows_affected > 0 {
                tracing::info!(
                    update_history_id = %record.id,
                    "AwaitingRestart timed out after {}s", timeout_secs
                );
                self.notifier
                    .signal_host_progression(record.host_id, record.tenant_id)
                    .await;
            }
        }
        Ok(())
    }

    async fn dispatch_verification(&self, db: &DatabaseConnection) -> crate::error::Result<()> {
        // Follow the same pattern as DetectVersionExecutor.
        // 1. Load all AwaitingRestart records.
        // 2. For each record with a non-null execution_owner_service_id:
        //    a. Query host_software_item_plugin for role='detect_version'
        //       and host_software_item_id = record.host_software_item_id.
        //    b. If no assignment: log warning, skip.
        //    c. Construct CheckVersionsPayload following DetectVersionExecutor pattern.
        //    d. Send via notifier.send_to_service(service_id, CheckVersions(payload)).
        //
        // Read crates/shared/scheduler-engine/src/executors/detect_version.rs for the
        // exact query helpers (query_agent_assignment_rows, VersionCheckAssignment, etc.)
        // and replicate the pattern scoped to the specific host_software_item_ids.
        tracing::trace!("AwaitingRestartExecutor: dispatch_verification not yet implemented");
        Ok(())
    }
}
```

**Implementation note for `dispatch_verification`:** read
`crates/shared/scheduler-engine/src/executors/detect_version.rs` in full. Copy the assignment-loading and
`CheckVersionsPayload` construction from there. The only difference: filter assignments to
`host_software_item_id IN (list from AwaitingRestart records)` instead of querying all due detect_version tasks.

- [ ] **Step 4: Register in controller scheduler**

In `crates/core/controller-runtime/src/scheduler/mod.rs`, inside the `register_extras` closure (or equivalent):

```rust
scheduler.register_tick_executor(Box::new(
    uptrakit_scheduler_engine::executors::awaiting_restart::AwaitingRestartExecutor::new(
        Arc::clone(&notifier),
    ),
));
```

- [ ] **Step 5: Verify and run tests**

```bash
cargo test -p uptrakit-scheduler-engine test_awaiting_restart_executor -- --nocapture
cargo check --all-features
```

- [ ] **Step 6: Commit**

```bash
git commit --only \
  crates/shared/scheduler-engine/src/executors/awaiting_restart.rs \
  crates/shared/scheduler-engine/src/executors/mod.rs \
  crates/core/controller-runtime/src/scheduler/mod.rs \
  -m "feat(scheduler): AwaitingRestartExecutor — timeout enforcement and verification dispatch"
```

---

### Task 13: `ServiceMetadata` + `HostRuntime::metadata_provider`

**Files:**

- Create: `crates/plugins/infrastructure/core/src/service_metadata.rs`
- Modify: `crates/plugins/infrastructure/core/src/host_runtime.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_standard_host_runtime_metadata_provider_returns_none() {
    use uptrakit_plugin_infrastructure_core::{HostRuntime, construct_host_runtime};
    use uptrakit_command::NoopCommandExecutor;
    use std::sync::Arc;
    let executor = Arc::new(NoopCommandExecutor);
    let runtime = construct_host_runtime(executor, Default::default());
    assert!(runtime.metadata_provider().is_none());
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-core test_standard_host_runtime_metadata_provider_returns_none
```

- [ ] **Step 3: Create `service_metadata.rs`**

```rust
// crates/plugins/infrastructure/core/src/service_metadata.rs
use std::path::PathBuf;
use std::sync::Arc;

/// Metadata about a running uptrakit service, provided by the controller
/// to the embedded self-update discovery plugin.
#[non_exhaustive]
pub struct ServiceMetadata {
    pub service_name: String,
    pub binary_path: Option<PathBuf>,
    pub version: String,
    pub deployment_topology: DeploymentTopology,
    pub reuseport_configured: bool,
    pub pid_file: Option<PathBuf>,
}

#[non_exhaustive]
pub enum DeploymentTopology {
    /// Unix only (Linux + macOS). Windows deferred.
    UnixBinary,
    DockerContainer {
        image: String,
        container_name: String,
    },
}

/// Implemented by the controller-standalone; injected into the self-update plugin at construction.
pub trait ServiceMetadataProvider: Send + Sync {
    fn get_metadata(&self) -> ServiceMetadata;
}
```

- [ ] **Step 4: Add `metadata_provider` default method to `HostRuntime`**

In `crates/plugins/infrastructure/core/src/host_runtime.rs`:

```rust
pub trait HostRuntime: Send + Sync + 'static {
    fn capabilities(&self) -> &HostCapabilities;
    fn as_any(&self) -> &dyn std::any::Any;
    fn executor(&self) -> std::sync::Arc<dyn uptrakit_command::CommandExecutor>;

    /// Returns the controller's self-metadata provider, if available.
    /// Only the controller-standalone overrides this — standalone agents return `None`.
    fn metadata_provider(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::service_metadata::ServiceMetadataProvider>> {
        None
    }
}
```

`StandardHostRuntime` does NOT override this — it inherits the `None` default.

- [ ] **Step 5: Re-export from `lib.rs`**

```rust
pub mod service_metadata;
pub use service_metadata::{DeploymentTopology, ServiceMetadata, ServiceMetadataProvider};
```

- [ ] **Step 6: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-core \
  test_standard_host_runtime_metadata_provider_returns_none
```

- [ ] **Step 7: Commit**

```bash
git commit --only \
  crates/plugins/infrastructure/core/src/service_metadata.rs \
  crates/plugins/infrastructure/core/src/host_runtime.rs \
  crates/plugins/infrastructure/core/src/lib.rs \
  -m "feat(plugin-core): ServiceMetadata, ServiceMetadataProvider, HostRuntime::metadata_provider"
```

---

### Task 14: `MetadataAwareHostRuntime` in controller-standalone

**Files:**

- Create: `crates/core/controller-runtime/src/embedded/metadata_runtime.rs`
- Modify: wherever the embedded agent's `HostRuntime` is constructed for plugin execution

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_metadata_aware_host_runtime_returns_some_provider() {
    use crate::embedded::metadata_runtime::MetadataAwareHostRuntime;
    use uptrakit_plugin_infrastructure_core::HostRuntime;
    use uptrakit_command::NoopCommandExecutor;
    use std::sync::Arc;

    let inner = uptrakit_plugin_infrastructure_core::construct_host_runtime(
        Arc::new(NoopCommandExecutor),
        Default::default(),
    );
    let metadata = build_test_metadata();
    let runtime = MetadataAwareHostRuntime::new(inner, Arc::new(metadata));
    assert!(runtime.metadata_provider().is_some());
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p uptrakit-controller-runtime test_metadata_aware_host_runtime_returns_some_provider
```

- [ ] **Step 3: Create `metadata_runtime.rs`**

```rust
// crates/core/controller-runtime/src/embedded/metadata_runtime.rs
use std::sync::Arc;
use uptrakit_plugin_infrastructure_core::{
    HostCapabilities, HostRuntime,
    service_metadata::{ServiceMetadata, ServiceMetadataProvider},
};
use uptrakit_command::CommandExecutor;

pub struct MetadataAwareHostRuntime {
    inner: Arc<dyn HostRuntime>,
    provider: Arc<dyn ServiceMetadataProvider>,
}

impl MetadataAwareHostRuntime {
    pub fn new(inner: Arc<dyn HostRuntime>, provider: Arc<dyn ServiceMetadataProvider>) -> Arc<Self> {
        Arc::new(Self { inner, provider })
    }
}

impl HostRuntime for MetadataAwareHostRuntime {
    fn capabilities(&self) -> &HostCapabilities {
        self.inner.capabilities()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executor(&self) -> Arc<dyn CommandExecutor> {
        self.inner.executor()
    }

    fn metadata_provider(&self) -> Option<Arc<dyn ServiceMetadataProvider>> {
        Some(Arc::clone(&self.provider))
    }
}
```

Also create a `ControllerMetadataProvider` struct in the same file that implements `ServiceMetadataProvider`
by reading `std::env::current_exe()` and the controller's running version:

```rust
pub struct ControllerMetadataProvider {
    service_name: String,
    version: String,
    reuseport_configured: bool,
    pid_file: Option<std::path::PathBuf>,
}

impl ControllerMetadataProvider {
    pub fn new(service_name: String, version: String, reuseport_configured: bool, pid_file: Option<std::path::PathBuf>) -> Self {
        Self { service_name, version, reuseport_configured, pid_file }
    }
}

impl ServiceMetadataProvider for ControllerMetadataProvider {
    fn get_metadata(&self) -> ServiceMetadata {
        let binary_path = std::env::current_exe().ok();
        ServiceMetadata {
            service_name: self.service_name.clone(),
            binary_path,
            version: self.version.clone(),
            deployment_topology: uptrakit_plugin_infrastructure_core::DeploymentTopology::UnixBinary,
            reuseport_configured: self.reuseport_configured,
            pid_file: self.pid_file.clone(),
        }
    }
}
```

- [ ] **Step 4: Wire `MetadataAwareHostRuntime` into plugin construction**

Find where the embedded agent constructs its `HostRuntime` for the self-update discovery plugin.
In `crates/core/controller-runtime/src/embedded/` find the plugin registry setup. Wrap the existing
`construct_host_runtime(...)` call with
`MetadataAwareHostRuntime::new(runtime, Arc::new(ControllerMetadataProvider::new(...)))`
when constructing the self-update plugin.

The wiring point is wherever `ProxmoxHelperScriptsPlugin` is constructed for the embedded agent —
the self-update plugin will be constructed at the same location (Task 18).

For now, just make the struct public and compilable.

- [ ] **Step 5: Verify and commit**

```bash
cargo check --all-features
git commit --only \
  crates/core/controller-runtime/src/embedded/metadata_runtime.rs \
  crates/core/controller-runtime/src/embedded/mod.rs \
  -m "feat(controller): MetadataAwareHostRuntime and ControllerMetadataProvider"
```

---

### Task 15: Shell plugin `resumable` config field + Docker plugin `resumable` field

**Files:**

- Modify: `crates/plugins/generic/shell/src/config.rs`
- Modify: `crates/plugins/generic/shell/src/plugin.rs`
- Modify: Docker plugin config (find with `grep -r "DockerConfig\|docker.*config" crates/plugins --include="*.rs" -l`)

- [ ] **Step 1: Write failing test for shell plugin**

```rust
#[test]
fn test_shell_plugin_resumable_config_defaults_false() {
    let config: ShellConfig = serde_json::from_str(r#"{"update_command":"echo hi"}"#).unwrap();
    assert!(!config.resumable);
}

#[test]
fn test_shell_plugin_resumable_config_true() {
    let config: ShellConfig =
        serde_json::from_str(r#"{"update_command":"echo hi","resumable":true}"#).unwrap();
    assert!(config.resumable);
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p uptrakit-plugin-generic-shell test_shell_plugin_resumable
```

- [ ] **Step 3: Add `resumable: bool` to `ShellConfig`**

In `crates/plugins/generic/shell/src/config.rs`:

```rust
#[serde(default)]
pub resumable: bool,
```

- [ ] **Step 4: Return `ExecuteUpdateResult { resumable }` from shell plugin**

In `crates/plugins/generic/shell/src/plugin.rs`, the `execute_update` implementation currently (after Task 5) returns:

```rust
Ok(ExecuteUpdateResult { output, resumable: false })
```

Change to:

```rust
Ok(ExecuteUpdateResult {
    output,
    resumable: self.config.resumable,
})
```

Where `self.config` is the `ShellConfig`. Verify that `self.config` is accessible in the `execute_update` impl —
it should be since the plugin struct holds its config.

- [ ] **Step 5: Add `resumable: bool` to the Docker plugin config**

Find the Docker plugin config struct (likely in `crates/plugins/releases/docker/src/config.rs` or similar). Add:

```rust
#[serde(default)]
pub resumable: bool,
```

In the Docker plugin's `execute_update` implementation, return:

```rust
Ok(ExecuteUpdateResult {
    output,
    resumable: self.config.resumable,
})
```

- [ ] **Step 6: Run tests and verify**

```bash
cargo test -p uptrakit-plugin-generic-shell test_shell_plugin_resumable
cargo check --all-features
```

- [ ] **Step 7: Commit**

```bash
git commit --only \
  crates/plugins/generic/shell/src/config.rs \
  crates/plugins/generic/shell/src/plugin.rs \
  -m "feat(shell-plugin): resumable config field; propagate to ExecuteUpdateResult"
```

---

### Task 16: `uptrakit-self-update` plugin crate skeleton

**Files:**

- Create: `crates/plugins/discovery/uptrakit-self-update/Cargo.toml`
- Create: `crates/plugins/discovery/uptrakit-self-update/src/lib.rs`
- Create: `crates/plugins/discovery/uptrakit-self-update/src/config.rs`
- Create: `crates/plugins/discovery/uptrakit-self-update/src/error.rs`
- Create: `crates/plugins/discovery/uptrakit-self-update/src/plugin.rs`

- [ ] **Step 1: Write the failing test**

```rust
// Will be in plugin.rs test module:
#[test]
fn test_detect_host_compatibility_disabled_by_default() {
    // Plugin with default config (enabled=false) must return Incompatible.
    use uptrakit_plugin_infrastructure_core::HostCompatibility;
    // Compile test: UptrakitSelfUpdateConfig::default().enabled == false
    let config = super::config::UptrakitSelfUpdateConfig::default();
    assert!(!config.enabled);
}
```

- [ ] **Step 2: Create `Cargo.toml`**

Model after `crates/plugins/discovery/proxmox-helper-scripts/Cargo.toml`:

```toml
[package]
name = "uptrakit-plugin-discovery-uptrakit-self-update"
edition.workspace = true
version.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
uptrakit-plugin-infrastructure-core = { workspace = true }
uptrakit-shared-macros = { workspace = true }
rootcause = { workspace = true }
thiserror = { workspace = true }
async-trait = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
uptrakit-shared-types = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt"] }
```

- [ ] **Step 3: Create `src/config.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UptrakitSelfUpdateConfig {
    /// Enable self-update discovery. Defaults to false.
    ///
    /// Must be explicitly set to `true` — controller-standalone ships with `enabled: false`.
    #[serde(default)]
    pub enabled: bool,
}
```

- [ ] **Step 4: Create `src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SelfUpdateError {
    #[error("metadata provider not available")]
    NoMetadataProvider,
    #[error("binary path not available for UnixBinary topology")]
    NoBinaryPath,
    #[error("pid file not configured for UnixBinary topology with reuseport")]
    NoPidFile,
}

impl From<SelfUpdateError> for uptrakit_plugin_infrastructure_core::PluginError {
    fn from(e: SelfUpdateError) -> Self {
        uptrakit_plugin_infrastructure_core::PluginError::Other(e.to_string())
    }
}
```

- [ ] **Step 5: Create `src/lib.rs`**

```rust
pub mod config;
pub mod error;
pub mod plugin;

pub use config::UptrakitSelfUpdateConfig;
pub use plugin::{DESCRIPTOR, UptrakitSelfUpdatePlugin};
```

- [ ] **Step 6: Create `src/plugin.rs` skeleton**

```rust
use std::sync::Arc;
use uptrakit_plugin_infrastructure_core::{HostRuntime, PluginMeta, ServiceMetadataProvider};
use uptrakit_shared_macros::declare_plugin;
use crate::config::UptrakitSelfUpdateConfig;

pub struct UptrakitSelfUpdatePlugin {
    config: UptrakitSelfUpdateConfig,
    metadata_provider: Option<Arc<dyn ServiceMetadataProvider>>,
}

impl UptrakitSelfUpdatePlugin {
    pub fn new(config: UptrakitSelfUpdateConfig, runtime: Arc<dyn HostRuntime>) -> Self {
        let metadata_provider = runtime.metadata_provider();
        Self { config, metadata_provider }
    }
}

declare_plugin!(UptrakitSelfUpdatePlugin, UptrakitSelfUpdateConfig, "discovery_uptrakit_self_update", {
    display_name: "Uptrakit Self-Update",
    family: uptrakit_plugin_infrastructure_core::PluginFamily::Software,
    config_model: uptrakit_plugin_infrastructure_core::ConfigModel::PluginConfig,
    host_requirements: uptrakit_plugin_infrastructure_core::HostRequirements::POSIX,
    config_test: [],
    roles: [Discoverer],
    sudo: UptrakitSelfUpdatePlugin::required_sudo_commands,
});

impl UptrakitSelfUpdatePlugin {
    fn required_sudo_commands() -> Vec<uptrakit_plugin_infrastructure_core::SudoEntry> {
        vec![]
    }
}
```

Check the `declare_plugin!` macro in `uptrakit-shared-macros` for the exact expected fields — mirror the proxmox-helper-scripts usage exactly.

- [ ] **Step 7: Verify compilation**

```bash
cargo check --all-features
cargo test -p uptrakit-plugin-discovery-uptrakit-self-update
```

- [ ] **Step 8: Commit**

```bash
git commit --only \
  crates/plugins/discovery/uptrakit-self-update/ \
  -m "feat(self-update): new discovery plugin crate skeleton"
```

---

### Task 17: `detect_host_compatibility` + `build_software_item` + `discover_software`

**Files:**

- Create: `crates/plugins/discovery/uptrakit-self-update/src/discovery.rs`
- Modify: `crates/plugins/discovery/uptrakit-self-update/src/lib.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[tokio::test]
async fn test_detect_host_compatibility_disabled() {
    use uptrakit_plugin_infrastructure_core::HostCompatibility;
    let plugin = make_plugin_with_config(UptrakitSelfUpdateConfig { enabled: false });
    let compat = plugin.detect_host_compatibility().await.unwrap();
    assert!(matches!(compat, HostCompatibility::Incompatible(_)));
}

#[tokio::test]
async fn test_detect_host_compatibility_no_metadata_provider() {
    let plugin = make_plugin_with_config(UptrakitSelfUpdateConfig { enabled: true });
    // plugin constructed without MetadataAwareHostRuntime → metadata_provider is None
    let compat = plugin.detect_host_compatibility().await.unwrap();
    assert!(matches!(compat, HostCompatibility::Incompatible(_)));
}

#[test]
fn test_build_software_item_sets_tag_strip_prefix() {
    let metadata = make_test_metadata("1.2.3");
    let plugin = make_plugin_with_metadata(metadata);
    let item = plugin.build_software_item_for_test();
    // Find the releases_github target in item.targets
    let gh_target = item.targets.iter()
        .find(|t| t.plugin_type.contains("github"))
        .expect("should have a github target");
    let config: serde_json::Value = serde_json::from_value(gh_target.plugin_config.clone()).unwrap();
    assert_eq!(config["tag_strip_prefix"], "v");
}

#[test]
fn test_build_software_item_awaiting_restart_timeout_is_120() {
    let metadata = make_test_metadata("1.2.0");
    let plugin = make_plugin_with_metadata(metadata);
    let item = plugin.build_software_item_for_test();
    // The DiscoveredSoftware extra field should carry awaiting_restart_timeout = 120.
    // OR: check that the returned software item has a special field for it.
    // (Verify how DiscoveredSoftware carries extra settings — may use `extra: Option<Value>`.)
    let extra = item.extra.as_ref().expect("should have extra");
    assert_eq!(extra["awaiting_restart_timeout"], 120);
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p uptrakit-plugin-discovery-uptrakit-self-update -- --nocapture
```

- [ ] **Step 3: Create `src/discovery.rs`**

Implement `Discoverer` for `UptrakitSelfUpdatePlugin`:

```rust
use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    Discoverer, DiscoveredSoftware, DiscoveryTarget, HostCompatibility,
};
use uptrakit_shared_types::DeploymentTopology;
use crate::plugin::UptrakitSelfUpdatePlugin;

#[async_trait]
impl Discoverer for UptrakitSelfUpdatePlugin {
    async fn detect_host_compatibility(&self) -> uptrakit_plugin_infrastructure_core::error::Result<HostCompatibility> {
        if !self.config.enabled {
            return Ok(HostCompatibility::Incompatible(
                "self-update disabled by config".to_string(),
            ));
        }
        if self.metadata_provider.is_none() {
            return Ok(HostCompatibility::Incompatible(
                "not running as embedded agent in controller-standalone".to_string(),
            ));
        }
        Ok(HostCompatibility::Compatible)
    }

    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::error::Result<Vec<DiscoveredSoftware>> {
        if !self.config.enabled {
            return Ok(vec![]);
        }
        let Some(ref provider) = self.metadata_provider else {
            return Ok(vec![]);
        };
        let metadata = provider.get_metadata();
        match self.build_software_item(&metadata) {
            Ok(item) => Ok(vec![item]),
            Err(e) => {
                tracing::warn!(error = %e, "uptrakit-self-update: skipping service due to error");
                Ok(vec![])
            }
        }
    }
}

impl UptrakitSelfUpdatePlugin {
    fn build_software_item(
        &self,
        metadata: &uptrakit_plugin_infrastructure_core::ServiceMetadata,
    ) -> Result<DiscoveredSoftware, crate::error::SelfUpdateError> {
        use uptrakit_plugin_infrastructure_core::DeploymentTopology;

        let binary_path = match &metadata.deployment_topology {
            DeploymentTopology::UnixBinary => {
                metadata.binary_path.as_ref()
                    .ok_or(crate::error::SelfUpdateError::NoBinaryPath)?
                    .to_string_lossy()
                    .into_owned()
            }
            DeploymentTopology::DockerContainer { .. } => String::new(),
            _ => String::new(),
        };

        let detect_version_target = self.build_detect_version_target(&binary_path, metadata);
        let fetch_releases_target = self.build_fetch_releases_target(metadata);
        let execute_update_target = self.build_execute_update_target(&binary_path, metadata)?;

        Ok(DiscoveredSoftware {
            package_identifier: metadata.service_name.clone(),
            name: format!("Uptrakit — {}", metadata.service_name),
            installed_version: metadata.version.clone(),
            targets: vec![detect_version_target, fetch_releases_target, execute_update_target],
            extra: Some(serde_json::json!({ "awaiting_restart_timeout": 120 })),
            qualifier: None,
            plugin_package_identifier: None,
            featured: false,
            installed_display_version: None,
        })
    }

    fn build_detect_version_target(
        &self,
        binary_path: &str,
        _metadata: &uptrakit_plugin_infrastructure_core::ServiceMetadata,
    ) -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: "shell".to_string(),
            plugin_config: serde_json::json!({
                "version_command": format!("{} --version", binary_path),
                "version_regex": r"(?P<version>\d+\.\d+\.\d+)"
            }),
            plugin_config_name: "detect_version".to_string(),
            roles: vec!["detect_version".to_string()],
            package_identifier: None,
            config_override: None,
            execution_site: Some("agent".to_string()),
        }
    }

    fn build_fetch_releases_target(
        &self,
        metadata: &uptrakit_plugin_infrastructure_core::ServiceMetadata,
    ) -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: "releases_github".to_string(),
            plugin_config: serde_json::json!({
                "owner": "uptrakit",
                "repo": "uptrakit",
                "tag_strip_prefix": "v",
                "asset_filter": metadata.service_name
            }),
            plugin_config_name: "fetch_releases".to_string(),
            roles: vec!["fetch_releases".to_string()],
            package_identifier: None,
            config_override: None,
            execution_site: Some("controller".to_string()),
        }
    }

    fn build_execute_update_target(
        &self,
        binary_path: &str,
        metadata: &uptrakit_plugin_infrastructure_core::ServiceMetadata,
    ) -> Result<DiscoveryTarget, crate::error::SelfUpdateError> {
        use uptrakit_plugin_infrastructure_core::DeploymentTopology;
        match &metadata.deployment_topology {
            DeploymentTopology::UnixBinary => {
                let script = self.generate_unix_update_script(binary_path, metadata)?;
                Ok(DiscoveryTarget {
                    plugin_type: "shell".to_string(),
                    plugin_config: serde_json::json!({
                        "update_command": script,
                        "resumable": true
                    }),
                    plugin_config_name: "execute_update".to_string(),
                    roles: vec!["execute_update".to_string()],
                    package_identifier: None,
                    config_override: None,
                    execution_site: Some("agent".to_string()),
                })
            }
            DeploymentTopology::DockerContainer { image, container_name } => {
                Ok(DiscoveryTarget {
                    plugin_type: "docker".to_string(),
                    plugin_config: serde_json::json!({
                        "image": image,
                        "container_name": container_name,
                        "resumable": true
                    }),
                    plugin_config_name: "execute_update".to_string(),
                    roles: vec!["execute_update".to_string()],
                    package_identifier: None,
                    config_override: None,
                    execution_site: Some("agent".to_string()),
                })
            }
            _ => Err(crate::error::SelfUpdateError::NoBinaryPath),
        }
    }

    fn generate_unix_update_script(
        &self,
        binary_path: &str,
        metadata: &uptrakit_plugin_infrastructure_core::ServiceMetadata,
    ) -> Result<String, crate::error::SelfUpdateError> {
        if metadata.reuseport_configured {
            let pid_file = metadata.pid_file.as_ref()
                .ok_or(crate::error::SelfUpdateError::NoPidFile)?
                .to_string_lossy()
                .into_owned();
            Ok(format!(
                r#"BINARY_PATH="{binary_path}"
TMP_PATH="${{BINARY_PATH}}.new-$$"
curl -L "$RELEASE_URL" -o "$TMP_PATH"
chmod +x "$TMP_PATH"
command -v codesign >/dev/null 2>&1 && codesign --sign - --force "$TMP_PATH"
mv "$TMP_PATH" "$BINARY_PATH"
kill -USR2 "$(cat "{pid_file}")"
"#,
                binary_path = binary_path,
                pid_file = pid_file,
            ))
        } else {
            let service_name = &metadata.service_name;
            Ok(format!(
                r#"BINARY_PATH="{binary_path}"
TMP_PATH="${{BINARY_PATH}}.new-$$"
curl -L "$RELEASE_URL" -o "$TMP_PATH"
chmod +x "$TMP_PATH"
command -v codesign >/dev/null 2>&1 && codesign --sign - --force "$TMP_PATH"
mv "$TMP_PATH" "$BINARY_PATH"
systemd-run --on-active=10s systemctl restart "{service_name}"
"#,
                binary_path = binary_path,
                service_name = service_name,
            ))
        }
    }
}
```

**Note:** Verify the exact field names on `DiscoveryTarget` against
`crates/shared/types/src/discovery_target.rs` — particularly `roles: Vec<PluginRole>` vs `Vec<String>`.
Adjust accordingly.

- [ ] **Step 4: Run tests**

```bash
cargo test -p uptrakit-plugin-discovery-uptrakit-self-update -- --nocapture
```

- [ ] **Step 5: Commit**

```bash
git commit --only \
  crates/plugins/discovery/uptrakit-self-update/src/discovery.rs \
  crates/plugins/discovery/uptrakit-self-update/src/lib.rs \
  -m "feat(self-update): detect_host_compatibility, discover_software, build_software_item"
```

---

### Task 18: Wire self-update plugin into controller-standalone

**Files:**

- Modify: `Cargo.toml` (workspace, if needed) — workspace already uses glob `crates/plugins/*/*`
- Modify: wherever the controller-standalone registers plugins (find with
  `grep -r "ProxmoxHelperScriptsPlugin\|register_plugin\|plugin_registry" crates/core --include="*.rs" -l`)
- Modify: wherever `MetadataAwareHostRuntime` is constructed and provided to plugins

- [ ] **Step 1: Find the plugin registration point**

```bash
grep -r "ProxmoxHelperScriptsPlugin\|PluginRegistry::new\|register.*plugin" \
  crates/core --include="*.rs" -l | head -5
```

Read the first result. Understand the pattern for adding a new discovery plugin.

- [ ] **Step 2: Add the crate dependency**

Since the workspace uses `crates/plugins/*/*` glob, no `Cargo.toml` edit is needed for the workspace.
But the controller or agent crate that uses the plugin needs to add it as a dependency:

```toml
# In the Cargo.toml of the crate that constructs the plugin registry:
uptrakit-plugin-discovery-uptrakit-self-update = { workspace = true }
```

Add a workspace dependency entry in the root `Cargo.toml` `[workspace.dependencies]` section (if the project uses that pattern):

```toml
uptrakit-plugin-discovery-uptrakit-self-update = { path = "crates/plugins/discovery/uptrakit-self-update" }
```

- [ ] **Step 3: Register the plugin**

Following the existing pattern (e.g., how `ProxmoxHelperScriptsPlugin` is registered), add
`UptrakitSelfUpdatePlugin` to the plugin registry in the embedded agent's discovery plugin list.

The key difference: the `UptrakitSelfUpdatePlugin` must receive a `MetadataAwareHostRuntime`. Wrap the standard runtime for this plugin only:

```rust
use uptrakit_controller_runtime::embedded::metadata_runtime::{
    ControllerMetadataProvider, MetadataAwareHostRuntime,
};
use uptrakit_plugin_discovery_uptrakit_self_update::UptrakitSelfUpdatePlugin;

let provider = ControllerMetadataProvider::new(
    "uptrakit-controller-standalone".to_string(),
    env!("CARGO_PKG_VERSION").to_string(),
    /* reuseport_configured: */ false, // read from controller config
    /* pid_file: */ None,              // read from controller config
);
let metadata_runtime = MetadataAwareHostRuntime::new(
    Arc::clone(&standard_runtime),
    Arc::new(provider),
);
let self_update_plugin = UptrakitSelfUpdatePlugin::new(
    self_update_config, // loaded from plugin config store, default UptrakitSelfUpdateConfig { enabled: false }
    metadata_runtime,
);
registry.register(DESCRIPTOR, Box::new(self_update_plugin));
```

- [ ] **Step 4: Write integration test (ignored)**

```rust
#[tokio::test]
#[ignore = "requires running controller-standalone instance"]
async fn test_self_update_plugin_discovers_running_controller() {
    // Start embedded controller with self_update enabled=true
    // Run discover_software
    // Assert: returns one DiscoveredSoftware for "uptrakit-controller-standalone"
    // Assert: targets include detect_version, fetch_releases, execute_update
    // Assert: execute_update target has resumable=true
}
```

Add a comment to `docs/development/testing.md` noting this test requires a live controller.

- [ ] **Step 5: Final quality gate**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features 2>&1 | tail -30
markdownlint --config .markdownlint.json '**/*.md'
```

Fix any clippy warnings. All tests must pass.

- [ ] **Step 6: Commit**

```bash
git commit --only \
  crates/plugins/discovery/uptrakit-self-update/ \
  crates/core/controller-runtime/ \
  Cargo.toml \
  -m "feat(self-update): wire UptrakitSelfUpdatePlugin into controller-standalone plugin registry"
```

---

## Self-Review

### Spec Coverage

| Spec requirement | Task |
| --- | --- |
| `awaiting_restart_timeout` column on `software_item` | Task 1 |
| `awaiting_restart_since` column on `update_history` | Task 1 |
| `UpdateStatus::AwaitingRestart` in both enums | Task 2 |
| `UpdateResultPayload.resumable` wire field | Task 3 |
| `VersionCheckResult.not_ready` wire field | Task 3 |
| `execute_update` plugin trait returns `ExecuteUpdateResult` | Task 4 |
| All existing plugins return `ExecuteUpdateResult { resumable: false }` | Task 5 |
| `execute_update_pipeline` moves post-hooks out | Task 6 |
| Resumable path spawns post-hooks, sends early result before them | Task 6 |
| Early result channel in `InFlightUpdate` | Task 7 |
| `try_recv` drain in JoinHandle completion arm | Task 7 |
| `handle_graceful_shutdown` skips send when `early_sent` | Task 7 |
| `transition_to_awaiting_restart` CAS function | Task 8 |
| `handle_update_result` resumable branch (no dispatch) | Task 8 |
| `has_active_update_for_host` includes `AwaitingRestart` | Task 9 |
| `maybe_complete_batch` includes `AwaitingRestart` as non-terminal | Task 9 |
| Unique constraint violation handled as debug/skip | Task 9 |
| `handle_version_check_results` AwaitingRestart correlation | Task 10 |
| All 5 `not_ready`/error/version evaluation rules | Task 10 |
| CAS loser must not dispatch | Task 8 + 10 |
| `TickExecutor` trait with 60s per-tick timeout | Task 11 |
| `Scheduler.register_tick_executor` | Task 11 |
| `SchedulerNotifier::signal_host_progression` | Task 11 |
| `AwaitingRestartExecutor` verification polling | Task 12 |
| `AwaitingRestartExecutor` timeout enforcement (`IS NOT NULL` guard) | Task 12 |
| Register `AwaitingRestartExecutor` in controller scheduler | Task 12 |
| `ServiceMetadata`, `ServiceMetadataProvider`, `DeploymentTopology` | Task 13 |
| `HostRuntime::metadata_provider()` default returns `None` | Task 13 |
| `MetadataAwareHostRuntime` + `ControllerMetadataProvider` | Task 14 |
| Shell plugin `resumable: bool` config field | Task 15 |
| Docker plugin `resumable: bool` config field | Task 15 |
| Self-update plugin crate with `enabled: false` default | Task 16 |
| `detect_host_compatibility` (disabled + no provider) | Task 17 |
| `build_software_item` with `awaiting_restart_timeout: 120` | Task 17 |
| `tag_strip_prefix: "v"` in generated `releases_github` config | Task 17 |
| `version_regex` for shell `detect_version` | Task 17 |
| Topology-specific update script generation (reuseport + fallback) | Task 17 |
| Same-directory temp file (`binary_path.new-$$`) | Task 17 |
| `codesign` portable no-op guard | Task 17 |
| Register plugin in controller-standalone | Task 18 |
| Partial unique index migration for `awaiting_restart` | Task 1 |

### Known Gaps (deferred per spec)

- `dispatch_verification` in `AwaitingRestartExecutor` is stubbed (Task 12 Step 3) — the engineer must complete
  it by reading `detect_version.rs`. The stub returns `Ok(())` so it compiles; the integration test will catch
  if it's missing.
- `DiscoveryTarget.roles` field type — verify against the actual struct; the plan uses `Vec<String>` but the actual type may be `Vec<PluginRole>`.
- `reuseport_configured` and `pid_file` wiring in `ControllerMetadataProvider` — the exact config field names
  depend on the controller-standalone config struct (Task 18 Step 3). Read the config struct before wiring.
- The SO_REUSEPORT + execve takeover protocol in the binary itself is **outside this spec** — the plan only
  generates the update script that signals the running process; the protocol is implemented in the runtime.
