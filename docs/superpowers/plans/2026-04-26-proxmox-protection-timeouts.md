# Proxmox Protection Timeouts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split Proxmox controller-side pre-update protection timeouts by mode, add global and per-item timeout
overrides, and preserve clear fallback semantics from item scope to global scope to built-in defaults.

**Architecture:** The change stays within the existing Proxmox policy flow. Persistence grows by two nullable
timeout columns per policy table; the policy layer and controller-side query store switch from row-level fallback
to field-level timeout merging; the registered Proxmox surfaces add mode-aware numeric timeout inputs; and the
runtime protection path replaces the shared 120-second wait with snapshot- and backup-specific resolved durations.

**Tech Stack:** Rust, SeaORM, SeaORM migrations, SQLite/MySQL mock DB tests, tokio, shared-surface form
descriptors, Svelte `SchemaForm` numeric coercion, `rootcause`.

---

## File Map

| File | Action | Responsibility |
| --- | --- | --- |
| `crates/shared/db/src/entity/proxmox_protection_default.rs` | Modify | Add nullable timeout columns to the SeaORM entity |
| `crates/shared/db/src/entity/proxmox_protection_item_override.rs` | Modify | Add nullable timeout columns to the SeaORM entity |
| `crates/plugins/infrastructure/core/src/roles.rs` | Modify | Extend typed Proxmox policy record and save request structs |
| `crates/plugins/infrastructure/proxmox/src/policy_store.rs` | Modify | Persist/load timeout fields and merge item/global timeout overrides per field |
| `crates/ui/web-api-queries/src/queries/update_dispatch.rs` | Modify | Build an effective runtime `ProxmoxProtectionPolicyRecord` with merged timeout fields |
| `crates/plugins/infrastructure/proxmox/src/controller_migration.rs` | Modify | Add new timeout columns to fresh schema and create a forward-safe migration for existing DBs |
| `crates/ui/web-api/src/surface_proxy.rs` | Modify | Keep controller-owned test/bootstrap tables in sync with the new schema |
| `crates/plugins/infrastructure/proxmox/src/plugin.rs` | Modify | Add `Snapshot timeout` and `Backup timeout` numeric fields to both policy surfaces |
| `crates/plugins/infrastructure/proxmox/src/surfaces.rs` | Modify | Preload/save timeout fields and preserve null/inherit semantics |
| `crates/plugins/infrastructure/proxmox/src/update_protection.rs` | Modify | Replace the shared wait timeout with resolved snapshot/backup durations |

---

### Task 1: Extend Proxmox Policy Data Contracts

**Files:**

- Modify: `crates/shared/db/src/entity/proxmox_protection_default.rs`
- Modify: `crates/shared/db/src/entity/proxmox_protection_item_override.rs`
- Modify: `crates/plugins/infrastructure/core/src/roles.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/policy_store.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`
- Test: `crates/plugins/infrastructure/proxmox/src/policy_store.rs`
- Test: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`

- [ ] **Step 1: Write the failing policy-merge tests**

  In `crates/plugins/infrastructure/proxmox/src/policy_store.rs`, add these tests inside the existing `tests` module:

  ```rust
  #[test]
  fn effective_policy_inherits_global_timeouts_per_field() {
      let item = ProtectionPolicy {
          mode: ProtectionMode::Snapshot,
          backup_target_key: None,
          snapshot_timeout_seconds: None,
          backup_timeout_seconds: None,
      };
      let global = ProtectionPolicy {
          mode: ProtectionMode::Backup,
          backup_target_key: Some("pbs-home:pbs".to_string()),
          snapshot_timeout_seconds: Some(180),
          backup_timeout_seconds: Some(1200),
      };

      let effective = resolve_effective_policy(Some(item), Some(global));
      assert_eq!(effective.mode, ProtectionMode::Snapshot);
      assert_eq!(effective.snapshot_timeout_seconds, Some(180));
      assert_eq!(effective.backup_timeout_seconds, Some(1200));
  }

  #[test]
  fn effective_policy_keeps_explicit_item_timeout() {
      let item = ProtectionPolicy {
          mode: ProtectionMode::Backup,
          backup_target_key: Some("pbs-home:pbs".to_string()),
          snapshot_timeout_seconds: Some(90),
          backup_timeout_seconds: Some(1500),
      };
      let global = ProtectionPolicy {
          mode: ProtectionMode::Backup,
          backup_target_key: Some("pbs-home:pbs".to_string()),
          snapshot_timeout_seconds: Some(180),
          backup_timeout_seconds: Some(1200),
      };

      let effective = resolve_effective_policy(Some(item), Some(global));
      assert_eq!(effective.snapshot_timeout_seconds, Some(90));
      assert_eq!(effective.backup_timeout_seconds, Some(1500));
  }
  ```

  In `crates/ui/web-api-queries/src/queries/update_dispatch.rs`, add this test inside the existing `tests` module:

  ```rust
  #[tokio::test]
  async fn proxmox_protection_store_merges_item_mode_and_global_timeouts() {
      use uptrakit_plugin_infrastructure_core::ProxmoxProtectionMode;
      use uptrakit_shared_db::entity::{
          proxmox_protection_default, proxmox_protection_item_override,
      };

      let tenant_id = Uuid::now_v7();
      let software_item_id = Uuid::now_v7();
      let plugin_config_id = Uuid::now_v7();
      let now = OffsetDateTime::now_utc();

      let item = proxmox_protection_item_override::Model {
          software_item_id,
          plugin_config_id,
          mode: "snapshot".to_string(),
          backup_target_key: None,
          snapshot_timeout_seconds: None,
          backup_timeout_seconds: None,
          created_at: now,
          updated_at: now,
      };
      let global = proxmox_protection_default::Model {
          tenant_id,
          plugin_config_id,
          mode: "backup".to_string(),
          backup_target_key: Some("pbs-home:pbs".to_string()),
          snapshot_timeout_seconds: Some(180),
          backup_timeout_seconds: Some(1200),
          created_at: now,
          updated_at: now,
      };

      let db = MockDatabase::new(DbBackend::MySql)
          .append_query_results([vec![item]])
          .append_query_results([vec![global]])
          .into_connection();
      let store = store_with_db(db);

      let policy = store
          .load_effective_policy(tenant_id, software_item_id, plugin_config_id)
          .await
          .expect("policy should load");

      assert_eq!(policy.mode, ProxmoxProtectionMode::Snapshot);
      assert_eq!(policy.snapshot_timeout_seconds, Some(180));
      assert_eq!(policy.backup_timeout_seconds, Some(1200));
  }
  ```

- [ ] **Step 2: Run the failing tests**

  Run:

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-proxmox effective_policy_inherits_global_timeouts_per_field -- --exact
  cargo test -p uptrakit-web-api-queries proxmox_protection_store_merges_item_mode_and_global_timeouts -- --exact
  ```

  Expected:
  - compile errors because `snapshot_timeout_seconds` and `backup_timeout_seconds` do not exist yet

- [ ] **Step 3: Extend the entity and typed contract structs**

  In both SeaORM entity files, add the nullable timeout fields after `backup_target_key`:

  ```rust
  pub snapshot_timeout_seconds: Option<i64>,
  pub backup_timeout_seconds: Option<i64>,
  ```

  In `crates/plugins/infrastructure/proxmox/src/policy_store.rs`, update `ProtectionPolicy`:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct ProtectionPolicy {
      pub mode: ProtectionMode,
      pub backup_target_key: Option<String>,
      pub snapshot_timeout_seconds: Option<i64>,
      pub backup_timeout_seconds: Option<i64>,
  }

  impl ProtectionPolicy {
      pub fn do_nothing() -> Self {
          Self {
              mode: ProtectionMode::DoNothing,
              backup_target_key: None,
              snapshot_timeout_seconds: None,
              backup_timeout_seconds: None,
          }
      }
  }
  ```

  In `crates/plugins/infrastructure/core/src/roles.rs`, extend the shared controller-side contract:

  ```rust
  pub struct ProxmoxProtectionPolicyRecord {
      pub mode: ProxmoxProtectionMode,
      pub backup_target_key: Option<String>,
      pub snapshot_timeout_seconds: Option<i64>,
      pub backup_timeout_seconds: Option<i64>,
  }
  ```

  Extend both save request structs:

  ```rust
  pub struct ProxmoxGlobalDefaultsSaveRequest {
      pub plugin_config_id: Uuid,
      pub mode: String,
      pub backup_target_option: Option<String>,
      pub snapshot_timeout_seconds: Option<i64>,
      pub backup_timeout_seconds: Option<i64>,
  }

  pub struct ProxmoxItemOverrideSaveRequest {
      pub software_item_id: Uuid,
      pub plugin_config_id: Uuid,
      pub mode: String,
      pub backup_target_option: Option<String>,
      pub snapshot_timeout_seconds: Option<i64>,
      pub backup_timeout_seconds: Option<i64>,
  }
  ```

  In `crates/plugins/infrastructure/proxmox/src/update_protection.rs`, update `map_policy_record`
  to pass through the new fields (interim form — Task 4 will add the built-in fallback constants):

  ```rust
  fn map_policy_record(policy: ProxmoxProtectionPolicyRecord) -> ProtectionPolicy {
      ProtectionPolicy {
          mode: map_protection_mode_from_record(policy.mode),
          backup_target_key: policy.backup_target_key,
          snapshot_timeout_seconds: policy.snapshot_timeout_seconds,
          backup_timeout_seconds: policy.backup_timeout_seconds,
      }
  }
  ```

  Also update every existing `ProtectionPolicy { mode, backup_target_key }` struct literal in
  `policy_store.rs` tests (lines ~549–556) and `update_protection.rs` tests (lines ~787–789,
  ~812–813, ~858–861, ~913–916) to include the two new fields:

  ```rust
  snapshot_timeout_seconds: None,
  backup_timeout_seconds: None,
  ```

- [ ] **Step 4: Implement field-level timeout merging in both policy stores**

  In `crates/plugins/infrastructure/proxmox/src/policy_store.rs`, replace the current row-level `resolve_effective_policy()` with field-level merge logic:

  ```rust
  pub fn resolve_effective_policy(
      item_override: Option<ProtectionPolicy>,
      global_default: Option<ProtectionPolicy>,
  ) -> ProtectionPolicy {
      let item_ref = item_override.as_ref();
      let global_ref = global_default.as_ref();

      let mode = item_ref
          .map(|policy| policy.mode)
          .or_else(|| global_ref.map(|policy| policy.mode))
          .unwrap_or(ProtectionMode::DoNothing);

      let backup_target_key = item_ref
          .and_then(|policy| policy.backup_target_key.clone())
          .or_else(|| global_ref.and_then(|policy| policy.backup_target_key.clone()));

      let snapshot_timeout_seconds = item_ref
          .and_then(|policy| policy.snapshot_timeout_seconds)
          .or_else(|| global_ref.and_then(|policy| policy.snapshot_timeout_seconds));

      let backup_timeout_seconds = item_ref
          .and_then(|policy| policy.backup_timeout_seconds)
          .or_else(|| global_ref.and_then(|policy| policy.backup_timeout_seconds));

      ProtectionPolicy {
          mode,
          backup_target_key,
          snapshot_timeout_seconds,
          backup_timeout_seconds,
      }
  }
  ```

  Update `load_global_default()` and `load_item_override()` to read the new fields from the entity model:

  ```rust
  Ok(row.map(|model| ProtectionPolicy {
      mode: ProtectionMode::from_db(&model.mode),
      backup_target_key: model.backup_target_key,
      snapshot_timeout_seconds: model.snapshot_timeout_seconds,
      backup_timeout_seconds: model.backup_timeout_seconds,
  }))
  ```

  Update `upsert_global_default()` — in the UPDATE branch add:

  ```rust
  active.snapshot_timeout_seconds = Set(policy.snapshot_timeout_seconds);
  active.backup_timeout_seconds = Set(policy.backup_timeout_seconds);
  ```

  In the INSERT branch, extend the `ActiveModel` literal:

  ```rust
  snapshot_timeout_seconds: Set(policy.snapshot_timeout_seconds),
  backup_timeout_seconds: Set(policy.backup_timeout_seconds),
  ```

  Apply the same changes to both branches of `upsert_item_override()`.

  In `crates/ui/web-api-queries/src/queries/update_dispatch.rs`, update
  `QueryProxmoxProtectionStore::load_effective_policy()` so it does not choose the
  item row wholesale. Build the effective record with row-level mode precedence
  plus per-field timeout fallback:

  ```rust
  let item_mode = item_override
      .as_ref()
      .map(|row| proxmox_mode_from_db(&row.mode));
  let global_mode = global_default
      .as_ref()
      .map(|row| proxmox_mode_from_db(&row.mode));

  let snapshot_timeout_seconds = item_override
      .as_ref()
      .and_then(|row| row.snapshot_timeout_seconds)
      .or_else(|| global_default.as_ref().and_then(|row| row.snapshot_timeout_seconds));

  let backup_timeout_seconds = item_override
      .as_ref()
      .and_then(|row| row.backup_timeout_seconds)
      .or_else(|| global_default.as_ref().and_then(|row| row.backup_timeout_seconds));

  let effective = ProxmoxProtectionPolicyRecord {
      mode: item_mode.or(global_mode).unwrap_or(ProxmoxProtectionMode::DoNothing),
      backup_target_key: item_override
          .as_ref()
          .and_then(|row| row.backup_target_key.clone())
          .or_else(|| global_default.as_ref().and_then(|row| row.backup_target_key.clone())),
      snapshot_timeout_seconds,
      backup_timeout_seconds,
  };
  ```

- [ ] **Step 5: Run the targeted tests**

  Run:

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-proxmox effective_policy_inherits_global_timeouts_per_field -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox effective_policy_keeps_explicit_item_timeout -- --exact
  cargo test -p uptrakit-web-api-queries proxmox_protection_store_merges_item_mode_and_global_timeouts -- --exact
  ```

  Expected:
  - all three tests pass

- [ ] **Step 6: Commit**

  ```bash
  git add \
    crates/shared/db/src/entity/proxmox_protection_default.rs \
    crates/shared/db/src/entity/proxmox_protection_item_override.rs \
    crates/plugins/infrastructure/core/src/roles.rs \
    crates/plugins/infrastructure/proxmox/src/policy_store.rs \
    crates/plugins/infrastructure/proxmox/src/update_protection.rs \
    crates/ui/web-api-queries/src/queries/update_dispatch.rs
  git commit -m "feat(proxmox): extend protection policy with timeout fields"
  ```

### Task 2: Add Forward-Safe Migration And Bootstrap Coverage

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`
- Modify: `crates/ui/web-api/src/surface_proxy.rs`
- Test: `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`
- Test: `crates/ui/web-api/src/surface_proxy.rs`

- [ ] **Step 1: Write the failing schema tests**

  In `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`, add a new test module with this migration smoke test:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use sea_orm::{ConnectionTrait, Database, DbBackend, Statement, TryGetable};
      use sea_orm_migration::{MigrationTrait, SchemaManager};

      async fn column_names(
          db: &sea_orm::DatabaseConnection,
          table: &str,
      ) -> Vec<String> {
          let rows = db
              .query_all(Statement::from_string(
                  DbBackend::Sqlite,
                  format!("PRAGMA table_info({table})"),
              ))
              .await
              .unwrap();
          rows.into_iter()
              .map(|row| row.try_get("", "name").unwrap())
              .collect()
      }

      #[tokio::test]
      async fn forward_timeout_migration_is_noop_after_fresh_create() {
          let db = Database::connect("sqlite::memory:").await.unwrap();
          let manager = SchemaManager::new(&db);

          CreateProxmoxProtectionPolicyTables.up(&manager).await.unwrap();
          AddProxmoxProtectionTimeoutColumns.up(&manager)
              .await
              .expect("forward migration must be safe on fresh DBs");

          let defaults = column_names(&db, "proxmox_protection_defaults").await;
          let overrides = column_names(&db, "proxmox_protection_item_overrides").await;

          assert!(defaults.contains(&"snapshot_timeout_seconds".to_string()));
          assert!(defaults.contains(&"backup_timeout_seconds".to_string()));
          assert!(overrides.contains(&"snapshot_timeout_seconds".to_string()));
          assert!(overrides.contains(&"backup_timeout_seconds".to_string()));
      }

      #[tokio::test]
      async fn forward_timeout_migration_upgrades_existing_schema() {
          let db = Database::connect("sqlite::memory:").await.unwrap();
          let manager = SchemaManager::new(&db);

          manager
              .create_table(
                  sea_orm_migration::prelude::Table::create()
                      .table(ProxmoxProtectionDefaults::Table)
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionDefaults::TenantId,
                          )
                          .uuid()
                          .not_null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionDefaults::PluginConfigId,
                          )
                          .uuid()
                          .not_null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionDefaults::Mode,
                          )
                          .text()
                          .not_null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionDefaults::BackupTargetKey,
                          )
                          .text()
                          .null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionDefaults::CreatedAt,
                          )
                          .timestamp()
                          .not_null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionDefaults::UpdatedAt,
                          )
                          .timestamp()
                          .not_null(),
                      )
                      .to_owned(),
              )
              .await
              .unwrap();

          manager
              .create_table(
                  sea_orm_migration::prelude::Table::create()
                      .table(ProxmoxProtectionItemOverrides::Table)
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionItemOverrides::SoftwareItemId,
                          )
                          .uuid()
                          .not_null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionItemOverrides::PluginConfigId,
                          )
                          .uuid()
                          .not_null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionItemOverrides::Mode,
                          )
                          .text()
                          .not_null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionItemOverrides::BackupTargetKey,
                          )
                          .text()
                          .null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionItemOverrides::CreatedAt,
                          )
                          .timestamp()
                          .not_null(),
                      )
                      .col(
                          sea_orm_migration::prelude::ColumnDef::new(
                              ProxmoxProtectionItemOverrides::UpdatedAt,
                          )
                          .timestamp()
                          .not_null(),
                      )
                      .to_owned(),
              )
              .await
              .unwrap();

          AddProxmoxProtectionTimeoutColumns.up(&manager)
              .await
              .expect("forward migration should upgrade existing schema");

          let defaults = column_names(&db, "proxmox_protection_defaults").await;
          let overrides = column_names(&db, "proxmox_protection_item_overrides").await;

          assert!(defaults.contains(&"snapshot_timeout_seconds".to_string()));
          assert!(defaults.contains(&"backup_timeout_seconds".to_string()));
          assert!(overrides.contains(&"snapshot_timeout_seconds".to_string()));
          assert!(overrides.contains(&"backup_timeout_seconds".to_string()));
      }
  }
  ```

  In `crates/ui/web-api/src/surface_proxy.rs`, add this test near the existing
  Proxmox policy-surface tests:

  ```rust
  #[tokio::test]
  async fn proxmox_update_protection_bootstrap_creates_timeout_columns() {
      use sea_orm::{ConnectionTrait, DbBackend, Statement, TryGetable};

      ensure_master_key();
      let db = setup_notification_db().await;
      ensure_proxmox_update_protection_tables(&db).await;

      let rows = db
          .query_all(Statement::from_string(
              DbBackend::Sqlite,
              "PRAGMA table_info(proxmox_protection_defaults)".to_string(),
          ))
          .await
          .unwrap();
      let names: Vec<String> = rows
          .into_iter()
          .map(|row| row.try_get("", "name").unwrap())
          .collect();

      assert!(names.contains(&"snapshot_timeout_seconds".to_string()));
      assert!(names.contains(&"backup_timeout_seconds".to_string()));
  }
  ```

- [ ] **Step 2: Run the failing tests**

  Run:

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-proxmox forward_timeout_migration_is_noop_after_fresh_create -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox forward_timeout_migration_upgrades_existing_schema -- --exact
  cargo test -p uptrakit-web-api proxmox_update_protection_bootstrap_creates_timeout_columns -- --exact
  ```

  Expected:
  - compile errors because the new migration and columns do not exist yet

- [ ] **Step 3: Implement the new forward-safe migration**

  In `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`:

  1. add the new identifiers to both policy-table enums
  2. update `CreateProxmoxProtectionPolicyTables::up()` so fresh DBs create the new nullable columns
  3. add a new migration struct named `AddProxmoxProtectionTimeoutColumns`
  4. insert it into `migrations()` immediately after `CreateProxmoxProtectionPolicyTables`

  Use this shape for the new migration:

  ```rust
  pub struct AddProxmoxProtectionTimeoutColumns;

  impl MigrationName for AddProxmoxProtectionTimeoutColumns {
      fn name(&self) -> &str {
          "m20260426_000001_proxmox_protection_timeouts"
      }
  }

  #[async_trait::async_trait]
  impl MigrationTrait for AddProxmoxProtectionTimeoutColumns {
      async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
          if !manager
              .has_column(
                  ProxmoxProtectionDefaults::Table,
                  ProxmoxProtectionDefaults::SnapshotTimeoutSeconds,
              )
              .await?
          {
              manager
                  .alter_table(
                      Table::alter()
                          .table(ProxmoxProtectionDefaults::Table)
                          .add_column(
                              ColumnDef::new(ProxmoxProtectionDefaults::SnapshotTimeoutSeconds)
                                  .big_integer()
                                  .null(),
                          )
                          .to_owned(),
                  )
                  .await?;
          }

          if !manager
              .has_column(
                  ProxmoxProtectionDefaults::Table,
                  ProxmoxProtectionDefaults::BackupTimeoutSeconds,
              )
              .await?
          {
              manager
                  .alter_table(
                      Table::alter()
                          .table(ProxmoxProtectionDefaults::Table)
                          .add_column(
                              ColumnDef::new(ProxmoxProtectionDefaults::BackupTimeoutSeconds)
                                  .big_integer()
                                  .null(),
                          )
                          .to_owned(),
                  )
                  .await?;
          }

          if !manager
              .has_column(
                  ProxmoxProtectionItemOverrides::Table,
                  ProxmoxProtectionItemOverrides::SnapshotTimeoutSeconds,
              )
              .await?
          {
              manager
                  .alter_table(
                      Table::alter()
                          .table(ProxmoxProtectionItemOverrides::Table)
                          .add_column(
                              ColumnDef::new(ProxmoxProtectionItemOverrides::SnapshotTimeoutSeconds)
                                  .big_integer()
                                  .null(),
                          )
                          .to_owned(),
                  )
                  .await?;
          }

          if !manager
              .has_column(
                  ProxmoxProtectionItemOverrides::Table,
                  ProxmoxProtectionItemOverrides::BackupTimeoutSeconds,
              )
              .await?
          {
              manager
                  .alter_table(
                      Table::alter()
                          .table(ProxmoxProtectionItemOverrides::Table)
                          .add_column(
                              ColumnDef::new(ProxmoxProtectionItemOverrides::BackupTimeoutSeconds)
                                  .big_integer()
                                  .null(),
                          )
                          .to_owned(),
                  )
                  .await?;
          }

          Ok(())
      }
  }
  ```

- [ ] **Step 4: Update the controller-owned SQL bootstrap helper**

  In `crates/ui/web-api/src/surface_proxy.rs`, extend both bootstrap `CREATE TABLE`
  statements:

  ```sql
  CREATE TABLE IF NOT EXISTS proxmox_protection_defaults (
      tenant_id TEXT NOT NULL,
      plugin_config_id TEXT NOT NULL,
      mode TEXT NOT NULL,
      backup_target_key TEXT NULL,
      snapshot_timeout_seconds INTEGER NULL,
      backup_timeout_seconds INTEGER NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      PRIMARY KEY (tenant_id, plugin_config_id)
  )
  ```

  ```sql
  CREATE TABLE IF NOT EXISTS proxmox_protection_item_overrides (
      software_item_id TEXT NOT NULL,
      plugin_config_id TEXT NOT NULL,
      mode TEXT NOT NULL,
      backup_target_key TEXT NULL,
      snapshot_timeout_seconds INTEGER NULL,
      backup_timeout_seconds INTEGER NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      PRIMARY KEY (software_item_id, plugin_config_id)
  )
  ```

- [ ] **Step 5: Run the schema tests**

  Run:

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-proxmox forward_timeout_migration_is_noop_after_fresh_create -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox forward_timeout_migration_upgrades_existing_schema -- --exact
  cargo test -p uptrakit-web-api proxmox_update_protection_bootstrap_creates_timeout_columns -- --exact
  ```

  Expected:
  - both tests pass

- [ ] **Step 6: Commit**

  ```bash
  git add \
    crates/plugins/infrastructure/proxmox/src/controller_migration.rs \
    crates/ui/web-api/src/surface_proxy.rs
  git commit -m "feat(proxmox): add timeout columns to protection policy schema"
  ```

### Task 3: Update Proxmox Policy Surfaces And Handlers

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/surfaces.rs`
- Test: `crates/plugins/infrastructure/proxmox/src/plugin.rs`
- Test: `crates/plugins/infrastructure/proxmox/src/surfaces.rs`

- [ ] **Step 1: Write the failing surface contract tests**

  In `crates/plugins/infrastructure/proxmox/src/plugin.rs`, extend
  `policy_surfaces_keep_preload_and_backup_options_contract()` to assert the new
  timeout fields exist and are numeric:

  ```rust
  let fields = save_global
      .form_ui
      .as_ref()
      .expect("save-global-defaults should expose a form")
      .fields
      .as_slice();

  let snapshot_timeout = fields
      .iter()
      .find(|field| field.key == "snapshot_timeout_seconds")
      .expect("snapshot timeout field should exist");
  assert_eq!(snapshot_timeout.field_type, "number");
  assert_eq!(
      snapshot_timeout.visible_when.as_ref().map(|rule| rule.field.as_str()),
      Some("mode")
  );
  assert_eq!(
      snapshot_timeout.visible_when.as_ref().map(|rule| rule.values.as_slice()),
      Some(["snapshot".to_string()].as_slice())
  );

  let backup_timeout = fields
      .iter()
      .find(|field| field.key == "backup_timeout_seconds")
      .expect("backup timeout field should exist");
  assert_eq!(backup_timeout.field_type, "number");
  assert_eq!(
      backup_timeout.visible_when.as_ref().map(|rule| rule.values.as_slice()),
      Some(["backup".to_string()].as_slice())
  );
  ```

  In `crates/plugins/infrastructure/proxmox/src/surfaces.rs`, add this handler
  persistence test and this validation test:

  ```rust
  #[tokio::test]
  async fn save_global_defaults_persists_timeout_fields() {
      use sea_orm::MockExecResult;
      use uptrakit_shared_db::entity::proxmox_protection_default;

      let tenant_id = Uuid::now_v7();
      let plugin_config_id = Uuid::now_v7();
      let db = MockDatabase::new(DbBackend::MySql)
          .append_query_results([vec![mock_plugin_config_model(tenant_id, plugin_config_id)]])
          .append_query_results([Vec::<proxmox_protection_default::Model>::new()])
          .append_exec_results([MockExecResult {
              last_insert_id: 0,
              rows_affected: 1,
          }])
          .into_connection();

      handle_save_global_defaults(
          &db,
          Some(tenant_id),
          ProxmoxGlobalDefaultsSaveRequest {
              plugin_config_id,
              mode: "snapshot".to_string(),
              backup_target_option: None,
              snapshot_timeout_seconds: Some(240),
              backup_timeout_seconds: None,
          },
      )
      .await
      .expect("save should succeed");

      let logs = db.into_transaction_log();
      let rendered = logs
          .iter()
          .flat_map(|tx| tx.statements().iter())
          .map(ToString::to_string)
          .collect::<Vec<_>>()
          .join("\n");

      assert!(rendered.contains("240"));
  }

  #[tokio::test]
  async fn save_global_defaults_rejects_zero_snapshot_timeout() {
      let tenant_id = Uuid::now_v7();
      let plugin_config_id = Uuid::now_v7();
      let db = MockDatabase::new(DbBackend::MySql)
          .append_query_results([vec![mock_plugin_config_model(tenant_id, plugin_config_id)]])
          .into_connection();

      let result = handle_save_global_defaults(
          &db,
          Some(tenant_id),
          ProxmoxGlobalDefaultsSaveRequest {
              plugin_config_id,
              mode: "snapshot".to_string(),
              backup_target_option: None,
              snapshot_timeout_seconds: Some(0),
              backup_timeout_seconds: None,
          },
      )
      .await;

      let err = result.expect_err("zero timeout should be rejected");
      assert!(err.contains("snapshot timeout must be a positive integer"));
  }

  #[tokio::test]
  async fn save_global_defaults_rejects_zero_backup_timeout() {
      let tenant_id = Uuid::now_v7();
      let plugin_config_id = Uuid::now_v7();
      let db = MockDatabase::new(DbBackend::MySql)
          .append_query_results([vec![mock_plugin_config_model(tenant_id, plugin_config_id)]])
          .into_connection();

      let result = handle_save_global_defaults(
          &db,
          Some(tenant_id),
          ProxmoxGlobalDefaultsSaveRequest {
              plugin_config_id,
              mode: "backup".to_string(),
              backup_target_option: None,
              snapshot_timeout_seconds: None,
              backup_timeout_seconds: Some(0),
          },
      )
      .await;

      let err = result.expect_err("zero timeout should be rejected");
      assert!(err.contains("backup timeout must be a positive integer"));
  }
  ```

- [ ] **Step 2: Run the failing surface tests**

  Run:

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-proxmox policy_surfaces_keep_preload_and_backup_options_contract -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox save_global_defaults_persists_timeout_fields -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox save_global_defaults_rejects_zero_snapshot_timeout -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox save_global_defaults_rejects_zero_backup_timeout -- --exact
  ```

  Expected:
  - compile errors because the timeout fields do not exist on the forms and request structs yet

- [ ] **Step 3: Extend the registered forms in `plugin.rs`**

  In both `proxmox_settings_update_protection_surface()` and
  `proxmox_software_item_update_protection_surface()`, add these fields to the
  `fields` vector:

  ```rust
  surfaces::FormFieldDescriptor {
      key: "snapshot_timeout_seconds".to_string(),
      label: "Snapshot timeout".to_string(),
      field_type: "number".to_string(),
      required: false,
      placeholder: Some("120".to_string()),
      help_text: Some(
          "Leave empty to use the built-in snapshot timeout of 120 seconds."
              .to_string(),
      ),
      default_value: None,
      options: vec![],
      select_source: None,
      sensitive: false,
      list: false,
      visible_when: Some(surfaces::FormVisibleWhen {
          field: "mode".to_string(),
          values: vec!["snapshot".to_string()],
      }),
  },
  surfaces::FormFieldDescriptor {
      key: "backup_timeout_seconds".to_string(),
      label: "Backup timeout".to_string(),
      field_type: "number".to_string(),
      required: false,
      placeholder: Some("900".to_string()),
      help_text: Some(
          "Leave empty to use the built-in backup timeout of 900 seconds."
              .to_string(),
      ),
      default_value: None,
      options: vec![],
      select_source: None,
      sensitive: false,
      list: false,
      visible_when: Some(surfaces::FormVisibleWhen {
          field: "mode".to_string(),
          values: vec!["backup".to_string()],
      }),
  },
  ```

  For the software-item surface, change the help text to inheritance wording:

  ```rust
  help_text: Some(
      "Leave empty to use the system-wide snapshot timeout for this mode."
          .to_string(),
  ),
  ```

  and:

  ```rust
  help_text: Some(
      "Leave empty to use the system-wide backup timeout for this mode."
          .to_string(),
  ),
  ```

- [ ] **Step 4: Extend the preload/save handlers in `surfaces.rs`**

  Update both preload handlers to include the timeout values directly in the JSON:

  ```rust
  Ok(json!({
      "plugin_config_id": selected_config.id.to_string(),
      "mode": policy.mode.as_str(),
      "backup_target_option": policy
          .backup_target_key
          .as_deref()
          .map(|target_key| encode_backup_target_option(selected_config.id, target_key))
          .unwrap_or_default(),
      "snapshot_timeout_seconds": policy.snapshot_timeout_seconds,
      "backup_timeout_seconds": policy.backup_timeout_seconds,
  }))
  ```

  Add a shared validator in `crates/plugins/infrastructure/proxmox/src/surfaces.rs`
  near the other small parsing helpers:

  ```rust
  fn validate_optional_positive_timeout(
      value: Option<i64>,
      label: &str,
  ) -> std::result::Result<Option<i64>, String> {
      match value {
          Some(seconds) if seconds <= 0 => {
              Err(format!("{label} must be a positive integer number of seconds"))
          }
          other => Ok(other),
      }
  }
  ```

  Update both save handlers to pass validated timeout fields into
  `ProtectionPolicy`:

  ```rust
  &ProtectionPolicy {
      mode,
      backup_target_key,
      snapshot_timeout_seconds: validate_optional_positive_timeout(
          request.snapshot_timeout_seconds,
          "snapshot timeout",
      )?,
      backup_timeout_seconds: validate_optional_positive_timeout(
          request.backup_timeout_seconds,
          "backup timeout",
      )?,
  }
  ```

  For the per-item preload no-config case, return `null` timeout values so the
  numeric form fields stay empty:

  ```rust
  return Ok(json!({
      "software_item_id": software_item_id.to_string(),
      "plugin_config_id": "",
      "mode": "inherit_global",
      "backup_target_option": "",
      "snapshot_timeout_seconds": serde_json::Value::Null,
      "backup_timeout_seconds": serde_json::Value::Null,
  }));
  ```

- [ ] **Step 5: Run the surface tests**

  Run:

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-proxmox policy_surfaces_keep_preload_and_backup_options_contract -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox save_global_defaults_persists_timeout_fields -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox save_global_defaults_rejects_zero_snapshot_timeout -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox save_global_defaults_rejects_zero_backup_timeout -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox save_item_overrides_rejects_unassigned_plugin_config_for_software_item -- --exact
  ```

  Expected:
  - all tests pass
  - the existing assignment-validation test still passes, proving timeout changes did not widen scope incorrectly

- [ ] **Step 6: Commit**

  ```bash
  git add \
    crates/plugins/infrastructure/proxmox/src/plugin.rs \
    crates/plugins/infrastructure/proxmox/src/surfaces.rs
  git commit -m "feat(proxmox): add timeout controls to protection surfaces"
  ```

### Task 4: Split Runtime Snapshot And Backup Wait Durations

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/update_protection.rs`
- Test: `crates/plugins/infrastructure/proxmox/src/update_protection.rs`

- [ ] **Step 1: Write the failing timeout-resolution tests**

  In the `tests` module of `crates/plugins/infrastructure/proxmox/src/update_protection.rs`,
  add these tests:

  ```rust
  #[test]
  fn snapshot_wait_timeout_prefers_policy_value() {
      let policy = ProtectionPolicy {
          mode: ProtectionMode::Snapshot,
          backup_target_key: None,
          snapshot_timeout_seconds: Some(240),
          backup_timeout_seconds: Some(900),
      };

      assert_eq!(snapshot_wait_timeout(&policy), Duration::from_secs(240));
  }

  #[test]
  fn backup_wait_timeout_uses_effective_policy_value() {
      let policy = ProtectionPolicy {
          mode: ProtectionMode::Backup,
          backup_target_key: Some("pbs-home:pbs".to_string()),
          snapshot_timeout_seconds: Some(120),
          backup_timeout_seconds: Some(900),
      };

      assert_eq!(backup_wait_timeout(&policy), Duration::from_secs(900));
  }

  #[test]
  fn map_policy_record_applies_default_snapshot_timeout() {
      let policy = map_policy_record(ProxmoxProtectionPolicyRecord {
          mode: ProxmoxProtectionMode::Snapshot,
          backup_target_key: None,
          snapshot_timeout_seconds: None,
          backup_timeout_seconds: None,
      });
      assert_eq!(snapshot_wait_timeout(&policy), Duration::from_secs(120));
  }

  #[test]
  fn map_policy_record_applies_default_backup_timeout() {
      let policy = map_policy_record(ProxmoxProtectionPolicyRecord {
          mode: ProxmoxProtectionMode::Backup,
          backup_target_key: None,
          snapshot_timeout_seconds: None,
          backup_timeout_seconds: None,
      });
      assert_eq!(backup_wait_timeout(&policy), Duration::from_secs(900));
  }
  ```

- [ ] **Step 2: Run the failing tests**

  Run:

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-proxmox snapshot_wait_timeout_prefers_policy_value -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox backup_wait_timeout_uses_effective_policy_value -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox map_policy_record_applies_default_snapshot_timeout -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox map_policy_record_applies_default_backup_timeout -- --exact
  ```

  Expected:
  - compile errors because the helper functions do not exist yet

- [ ] **Step 3: Replace the shared timeout with mode-specific helpers**

  In `crates/plugins/infrastructure/proxmox/src/update_protection.rs`, replace:

  ```rust
  const PROTECTION_WAIT_TIMEOUT: Duration = Duration::from_secs(120);
  ```

  with:

  ```rust
  const DEFAULT_SNAPSHOT_TIMEOUT_SECONDS: i64 = 120;
  const DEFAULT_BACKUP_TIMEOUT_SECONDS: i64 = 900;
  ```

  Update `map_policy_record()` so the runtime policy already carries built-in
  fallback values:

  ```rust
  fn map_policy_record(policy: ProxmoxProtectionPolicyRecord) -> ProtectionPolicy {
      ProtectionPolicy {
          mode: map_protection_mode_from_record(policy.mode),
          backup_target_key: policy.backup_target_key,
          snapshot_timeout_seconds: Some(
              policy
                  .snapshot_timeout_seconds
                  .unwrap_or(DEFAULT_SNAPSHOT_TIMEOUT_SECONDS),
          ),
          backup_timeout_seconds: Some(
              policy
                  .backup_timeout_seconds
                  .unwrap_or(DEFAULT_BACKUP_TIMEOUT_SECONDS),
          ),
      }
  }
  ```

  Add helper functions near the other pure helpers:

  ```rust
  fn snapshot_wait_timeout(policy: &ProtectionPolicy) -> Duration {
      Duration::from_secs(
          policy
              .snapshot_timeout_seconds
              .expect("effective policy must already resolve snapshot timeout")
              as u64,
      )
  }

  fn backup_wait_timeout(policy: &ProtectionPolicy) -> Duration {
      Duration::from_secs(
          policy
              .backup_timeout_seconds
              .expect("effective policy must already resolve backup timeout")
              as u64,
      )
  }
  ```

  Change the snapshot branch so it receives the policy and uses the snapshot helper:

  ```rust
  ProtectionMode::Snapshot => {
      prepare_snapshot_protection(store, ctx, &mapping, &proxmox_cfg, &policy).await
  }
  ```

  Update the function signature:

  ```rust
  async fn prepare_snapshot_protection(
      store: &dyn ProxmoxProtectionStore,
      ctx: &ControllerProtectionContext<'_>,
      mapping: &ProxmoxHostMappingRecord,
      proxmox_cfg: &ProxmoxConfig,
      policy: &ProtectionPolicy,
  ) -> Result<ControllerProtectionDecision> {
  ```

  And replace the wait call:

  ```rust
  if let Err(error) = client
      .wait_for_task_completion(
          &mapping.proxmox_node,
          &task,
          snapshot_wait_timeout(policy),
      )
      .await
  {
  ```

  Do the same in the backup branch:

  ```rust
  if let Err(error) = client
      .wait_for_task_completion(
          &mapping.proxmox_node,
          &task,
          backup_wait_timeout(policy),
      )
      .await
  {
  ```

- [ ] **Step 4: Run the timeout tests**

  Run:

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-proxmox snapshot_wait_timeout_prefers_policy_value -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox backup_wait_timeout_uses_effective_policy_value -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox map_policy_record_applies_default_snapshot_timeout -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox map_policy_record_applies_default_backup_timeout -- --exact
  cargo test -p uptrakit-plugin-infrastructure-proxmox backup_mode_missing_cached_target_persists_failed_audit -- --exact
  ```

  Expected:
  - all five tests pass

- [ ] **Step 5: Commit**

  ```bash
  git add crates/plugins/infrastructure/proxmox/src/update_protection.rs
  git commit -m "fix(proxmox): split snapshot and backup protection timeouts"
  ```

### Task 5: Full Verification

**Files:**

- Modify: none expected
- Verify: workspace Rust packages touched by Tasks 1-4

- [ ] **Step 1: Run the focused package tests**

  Run:

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features
  cargo test -p uptrakit-web-api-queries
  cargo test -p uptrakit-web-api proxmox_update_protection -- --nocapture
  ```

  Expected:
  - all tests pass

- [ ] **Step 2: Run focused compile checks**

  Run:

  ```bash
  cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
  cargo check -p uptrakit-web-api-queries
  cargo check -p uptrakit-web-api
  ```

  Expected:
  - all three packages compile without errors

- [ ] **Step 3: Run the Markdown guard on the spec and plan docs**

  Run:

  ```bash
  markdownlint --config .markdownlint.json \
    docs/superpowers/specs/2026-04-26-proxmox-protection-timeouts-design.md \
    docs/superpowers/plans/2026-04-26-proxmox-protection-timeouts.md
  ```

  Expected:
  - no markdownlint errors

- [ ] **Step 4: Inspect the final diff**

  Run:

  ```bash
  git diff --stat HEAD~4..HEAD
  git diff --check
  ```

  Expected:
  - changed files limited to the planned schema, policy, surface, and runtime files
  - `git diff --check` prints no whitespace or merge-marker errors

- [ ] **Step 5: Final commit only if verification produced follow-up fixes**

  If verification required no changes, skip this step.

  If verification required a small follow-up patch, commit it with:

  ```bash
  git add -A
  git commit -m "chore: verify Proxmox protection timeout feature"
  ```

---

## Self-Review

- Spec coverage:
  - schema changes: Task 2
  - typed requests and records: Task 1
  - field-level inheritance: Task 1
  - numeric timeout fields and UI visibility: Task 3
  - runtime split between snapshot and backup: Task 4
  - forward migration safety and bootstrap parity: Task 2
  - verification and regressions: Task 5
- Placeholder scan:
  - no unfinished markers or cross-task shorthand shortcuts remain
- Type consistency:
  - timeout field names are consistently `snapshot_timeout_seconds` and
    `backup_timeout_seconds`
  - runtime helper names are consistently `snapshot_wait_timeout` and
    `backup_wait_timeout`
- Fallback semantics: single fallback point in `map_policy_record` via
  `unwrap_or(DEFAULT_*)` constants; `update_dispatch.rs` passes `None`
  through without applying independent defaults
- Migration tracking: `AddProxmoxProtectionTimeoutColumns` has `MigrationName`
  impl with stable name `m20260426_000001_proxmox_protection_timeouts`
- Compile continuity: Task 1 Step 3 includes interim `map_policy_record` update
  and existing struct-literal fixes so the workspace compiles at every commit
  boundary; Task 4 Step 3 replaces the interim pass-through with
  `unwrap_or(DEFAULT_*)` constants
- Validation coverage: both snapshot and backup zero-timeout rejection tested
