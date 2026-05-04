# Proxmox Resource Scaling v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend Proxmox resource scaling with delta mode, explicit opt-in semantics, and two new UI surfaces.

**Architecture:** New `proxmox_scaling_defaults` / `proxmox_scaling_item_overrides` tables replace the `update_cores` / `update_memory_mb` columns
on the protection tables. A new `scaling_store.rs` module owns all scaling CRUD. The existing surface registrations are renamed from
`update-protection` to `update-hooks` and extended with a "Resource Scaling" section whose form fields are mode-gated via `FormVisibleWhen`.

**Tech Stack:** Rust / SeaORM / sea-query migrations, Svelte 5 (surface rendered generically via SchemaForm — no new component code).

---

## File Map

| Status | Path                                                                                   |
| ------ | -------------------------------------------------------------------------------------- |
| Modify | `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`                    |
| Create | `crates/plugins/infrastructure/proxmox/src/entity/proxmox_scaling_default.rs`          |
| Create | `crates/plugins/infrastructure/proxmox/src/entity/proxmox_scaling_item_override.rs`    |
| Modify | `crates/plugins/infrastructure/proxmox/src/entity/mod.rs`                              |
| Modify | `crates/plugins/infrastructure/proxmox/src/entity/proxmox_resource_scaling_record.rs`  |
| Modify | `crates/plugins/infrastructure/proxmox/src/entity/proxmox_protection_default.rs`       |
| Modify | `crates/plugins/infrastructure/proxmox/src/entity/proxmox_protection_item_override.rs` |
| Create | `crates/plugins/infrastructure/proxmox/src/scaling_store.rs`                           |
| Modify | `crates/plugins/infrastructure/proxmox/src/lib.rs`                                     |
| Modify | `crates/plugins/infrastructure/proxmox/src/policy_store.rs`                            |
| Modify | `crates/plugins/infrastructure/proxmox/src/protection_store.rs`                        |
| Modify | `crates/plugins/infrastructure/proxmox/src/resource_scaling.rs`                        |
| Modify | `crates/plugins/infrastructure/proxmox/src/surfaces.rs`                                |
| Modify | `crates/plugins/infrastructure/proxmox/src/plugin.rs`                                  |
| Modify | `crates/plugins/infrastructure/proxmox/src/reset.rs`                                   |
| Modify | `docs/development/coding-standards.md`                                                 |

---

## Task 1: Migrations A–B (new scaling tables)

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`

- [ ] **Step 1: Write the failing tests for Migrations A and B**

Add to the `#[cfg(test)] mod tests` block at the bottom of `controller_migration.rs`:

```rust
#[tokio::test]
async fn migration_new_a_creates_proxmox_scaling_defaults_table() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);

    CreateProxmoxScalingDefaults.up(&manager).await.unwrap();

    let cols = column_names(&db, "proxmox_scaling_defaults").await;
    for expected in &[
        "id", "tenant_id", "plugin_config_id", "scaling_mode",
        "absolute_cores", "absolute_memory_mb", "delta_cores",
        "delta_memory_mb", "created_at", "updated_at",
    ] {
        assert!(cols.contains(&expected.to_string()), "missing column: {expected}");
    }
}

#[tokio::test]
async fn migration_new_a_check_constraint_rejects_zero_absolute_cores() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    CreateProxmoxScalingDefaults.up(&manager).await.unwrap();

    let tid = "00000000-0000-0000-0000-000000000001";
    let cid = "00000000-0000-0000-0000-000000000002";
    let id  = "00000000-0000-0000-0000-000000000003";
    let result = db
        .execute_unprepared(&format!(
            "INSERT INTO proxmox_scaling_defaults \
             (id, tenant_id, plugin_config_id, scaling_mode, absolute_cores, created_at, updated_at) \
             VALUES ('{id}', '{tid}', '{cid}', 'absolute', 0, '2026-01-01', '2026-01-01')"
        ))
        .await;
    assert!(result.is_err(), "CHECK constraint should reject absolute_cores = 0");
}

#[tokio::test]
async fn migration_new_b_creates_proxmox_scaling_item_overrides_table() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);

    CreateProxmoxScalingItemOverrides.up(&manager).await.unwrap();

    let cols = column_names(&db, "proxmox_scaling_item_overrides").await;
    for expected in &[
        "id", "tenant_id", "software_item_id", "plugin_config_id", "scaling_mode",
        "absolute_cores", "absolute_memory_mb", "delta_cores",
        "delta_memory_mb", "created_at", "updated_at",
    ] {
        assert!(cols.contains(&expected.to_string()), "missing column: {expected}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox controller_migration -- --nocapture 2>&1 | grep -E "FAILED|error|CreateProxmoxScaling"
```

Expected: compile error — `CreateProxmoxScalingDefaults` / `CreateProxmoxScalingItemOverrides` not defined yet.

- [ ] **Step 3: Add the DeriveIden enums and migration structs**

Insert after the `ProxmoxResourceScalingPolicyCols` enum (line ~1294) and before `CreateProxmoxResourceScalingRecord`:

```rust
// ── Scaling tables DeriveIden enums ────────────────────────────────────────

#[derive(DeriveIden)]
enum ProxmoxScalingDefaults {
    Table,
}

#[derive(DeriveIden)]
enum ProxmoxScalingItemOverrides {
    Table,
}

// ── Migration A: create proxmox_scaling_defaults ────────────────────────────

pub struct CreateProxmoxScalingDefaults;

impl MigrationName for CreateProxmoxScalingDefaults {
    fn name(&self) -> &str {
        "m20260504_000001_proxmox_scaling_defaults"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxScalingDefaults {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE proxmox_scaling_defaults (
                    id TEXT NOT NULL PRIMARY KEY,
                    tenant_id TEXT NOT NULL,
                    plugin_config_id TEXT NOT NULL,
                    scaling_mode VARCHAR(16) NOT NULL DEFAULT 'none',
                    absolute_cores INTEGER \
                        CHECK (absolute_cores IS NULL OR absolute_cores >= 1),
                    absolute_memory_mb INTEGER \
                        CHECK (absolute_memory_mb IS NULL OR absolute_memory_mb >= 1),
                    delta_cores INTEGER \
                        CHECK (delta_cores IS NULL OR delta_cores >= 1),
                    delta_memory_mb INTEGER \
                        CHECK (delta_memory_mb IS NULL OR delta_memory_mb >= 1),
                    created_at TIMESTAMP NOT NULL,
                    updated_at TIMESTAMP NOT NULL,
                    UNIQUE (tenant_id, plugin_config_id)
                )",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxScalingDefaults::Table)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration B: create proxmox_scaling_item_overrides ──────────────────────

pub struct CreateProxmoxScalingItemOverrides;

impl MigrationName for CreateProxmoxScalingItemOverrides {
    fn name(&self) -> &str {
        "m20260504_000002_proxmox_scaling_item_overrides"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxScalingItemOverrides {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE proxmox_scaling_item_overrides (
                    id TEXT NOT NULL PRIMARY KEY,
                    tenant_id TEXT NOT NULL,
                    software_item_id TEXT NOT NULL,
                    plugin_config_id TEXT NOT NULL,
                    scaling_mode VARCHAR(16) NOT NULL DEFAULT 'none',
                    absolute_cores INTEGER \
                        CHECK (absolute_cores IS NULL OR absolute_cores >= 1),
                    absolute_memory_mb INTEGER \
                        CHECK (absolute_memory_mb IS NULL OR absolute_memory_mb >= 1),
                    delta_cores INTEGER \
                        CHECK (delta_cores IS NULL OR delta_cores >= 1),
                    delta_memory_mb INTEGER \
                        CHECK (delta_memory_mb IS NULL OR delta_memory_mb >= 1),
                    created_at TIMESTAMP NOT NULL,
                    updated_at TIMESTAMP NOT NULL,
                    UNIQUE (software_item_id, plugin_config_id)
                )",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxScalingItemOverrides::Table)
                    .to_owned(),
            )
            .await
    }
}
```

- [ ] **Step 4: Append to `migrations()` vec**

In the `migrations()` function, append after `Box::new(CreateProxmoxResourceScalingRecord)`:

```rust
Box::new(CreateProxmoxScalingDefaults),
Box::new(CreateProxmoxScalingItemOverrides),
```

So the final two entries become:

```rust
Box::new(AddProxmoxResourceScalingPolicyColumns),
Box::new(CreateProxmoxResourceScalingRecord),
Box::new(CreateProxmoxScalingDefaults),
Box::new(CreateProxmoxScalingItemOverrides),
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox controller_migration::tests::migration_new_a -- --nocapture
cargo test -p uptrakit-plugin-infrastructure-proxmox controller_migration::tests::migration_new_b -- --nocapture
```

Expected: PASS for both.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/controller_migration.rs
git commit -m "feat(plugin-infrastructure-proxmox): add migrations A-B for scaling tables

Create proxmox_scaling_defaults and proxmox_scaling_item_overrides with
scaling_mode discriminant, dimension columns, CHECK constraints (>= 1),
and unique indexes on (tenant_id, plugin_config_id) /
(software_item_id, plugin_config_id) respectively.

Rollout: additive schema — no breaking changes to existing tables.
Testing: run migration_new_a_* and migration_new_b_* tests.
Migration: applies automatically on first startup; no manual steps."
```

---

## Task 2: Migrations C–E (data migration, drop old columns, add scaling_mode_used)

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`

- [ ] **Step 1: Write the failing tests**

Add to the test module:

```rust
#[tokio::test]
async fn migration_c_transfers_protection_rows_to_scaling_tables() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);

    // Run prerequisites
    CreateProxmoxProtectionPolicyTables.up(&manager).await.unwrap();
    AddProxmoxProtectionTimeoutColumns.up(&manager).await.unwrap();
    AddProxmoxResourceScalingPolicyColumns.up(&manager).await.unwrap();
    CreateProxmoxScalingDefaults.up(&manager).await.unwrap();
    CreateProxmoxScalingItemOverrides.up(&manager).await.unwrap();

    // Seed: one row with scaling, one without
    db.execute_unprepared(
        "INSERT INTO proxmox_protection_defaults \
         (tenant_id, plugin_config_id, mode, update_cores, update_memory_mb, \
          created_at, updated_at) \
         VALUES \
         ('aaaaaaaa-0000-0000-0000-000000000001', \
          'bbbbbbbb-0000-0000-0000-000000000001', \
          'do_nothing', 8, 4096, '2026-01-01', '2026-01-01'), \
         ('aaaaaaaa-0000-0000-0000-000000000002', \
          'bbbbbbbb-0000-0000-0000-000000000002', \
          'do_nothing', NULL, NULL, '2026-01-01', '2026-01-01')"
    ).await.unwrap();

    MigrateProxmoxScalingFromProtectionTables.up(&manager).await.unwrap();

    // Row with scaling fields → scaling table row
    let count: i64 = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) as c FROM proxmox_scaling_defaults WHERE scaling_mode = 'absolute'",
        ))
        .await.unwrap().unwrap()
        .try_get("", "c").unwrap();
    assert_eq!(count, 1, "one row should be migrated");

    // Row without scaling fields → no scaling row
    let count2: i64 = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) as c FROM proxmox_scaling_defaults",
        ))
        .await.unwrap().unwrap()
        .try_get("", "c").unwrap();
    assert_eq!(count2, 1, "null-only row must not generate a scaling row");

    // Source columns set to NULL after migration
    let cores: Option<i64> = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT update_cores FROM proxmox_protection_defaults \
             WHERE tenant_id = 'aaaaaaaa-0000-0000-0000-000000000001'",
        ))
        .await.unwrap().unwrap()
        .try_get("", "update_cores").unwrap();
    assert!(cores.is_none(), "source update_cores must be NULL'd after migration C");
}

#[tokio::test]
async fn migration_c_transfers_item_override_rows_to_scaling_tables() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);

    // Run prerequisites (plugin_configs table must exist for the JOIN in C.2)
    CreateProxmoxProtectionPolicyTables.up(&manager).await.unwrap();
    AddProxmoxProtectionTimeoutColumns.up(&manager).await.unwrap();
    AddProxmoxResourceScalingPolicyColumns.up(&manager).await.unwrap();
    CreateProxmoxScalingDefaults.up(&manager).await.unwrap();
    CreateProxmoxScalingItemOverrides.up(&manager).await.unwrap();

    let tid = "cccccccc-0000-0000-0000-000000000001";
    let cid = "dddddddd-0000-0000-0000-000000000001";
    let sid = "eeeeeeee-0000-0000-0000-000000000001";
    // Insert a plugin_config row so the JOIN resolves tenant_id
    db.execute_unprepared(&format!(
        "INSERT INTO plugin_configs (id, tenant_id, name, plugin_type, config, created_at, updated_at) \
         VALUES ('{cid}', '{tid}', 'test', 'infrastructure_proxmox', '{{}}', '2026-01-01', '2026-01-01')"
    ))
    .await
    .unwrap();
    // Insert item override row with non-null scaling fields
    db.execute_unprepared(&format!(
        "INSERT INTO proxmox_protection_item_overrides \
         (software_item_id, plugin_config_id, mode, update_cores, update_memory_mb, created_at, updated_at) \
         VALUES ('{sid}', '{cid}', 'do_nothing', 4, 2048, '2026-01-01', '2026-01-01')"
    ))
    .await
    .unwrap();

    MigrateProxmoxScalingFromProtectionTables.up(&manager).await.unwrap();

    let count: i64 = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) as c FROM proxmox_scaling_item_overrides WHERE scaling_mode = 'absolute'",
        ))
        .await.unwrap().unwrap()
        .try_get("", "c").unwrap();
    assert_eq!(count, 1, "item override row must be migrated to proxmox_scaling_item_overrides");
}

#[tokio::test]
async fn migration_d_drops_scaling_columns_from_protection_tables() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);

    CreateProxmoxProtectionPolicyTables.up(&manager).await.unwrap();
    AddProxmoxProtectionTimeoutColumns.up(&manager).await.unwrap();
    AddProxmoxResourceScalingPolicyColumns.up(&manager).await.unwrap();
    CreateProxmoxScalingDefaults.up(&manager).await.unwrap();
    CreateProxmoxScalingItemOverrides.up(&manager).await.unwrap();
    MigrateProxmoxScalingFromProtectionTables.up(&manager).await.unwrap();
    DropProxmoxScalingColumnsFromProtectionTables.up(&manager).await.unwrap();

    let defaults = column_names(&db, "proxmox_protection_defaults").await;
    assert!(!defaults.contains(&"update_cores".to_string()), "update_cores must be dropped");
    assert!(!defaults.contains(&"update_memory_mb".to_string()), "update_memory_mb must be dropped");

    let overrides = column_names(&db, "proxmox_protection_item_overrides").await;
    assert!(!overrides.contains(&"update_cores".to_string()), "update_cores must be dropped");
    assert!(!overrides.contains(&"update_memory_mb".to_string()), "update_memory_mb must be dropped");
}

#[tokio::test]
async fn migration_e_adds_scaling_mode_used_to_scaling_records() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);

    CreateProxmoxResourceScalingRecord.up(&manager).await.unwrap();
    AddScalingModeUsedToScalingRecord.up(&manager).await.unwrap();

    let cols = column_names(&db, "proxmox_resource_scaling_records").await;
    assert!(cols.contains(&"scaling_mode_used".to_string()), "scaling_mode_used must exist");

    // Existing rows should default to 'absolute'
    db.execute_unprepared(
        "INSERT INTO proxmox_resource_scaling_records \
         (update_history_id, tenant_id, host_id, software_item_id, plugin_config_id, \
          mapping_id, vm_type, original_cores, original_memory_mb, scaled_cores, \
          scaled_memory_mb, scale_status, restore_status, created_at, updated_at) \
         VALUES ('h1', 't1', 'h2', 's1', 'p1', 'm1', 'qemu', 4, 4096, 8, 8192, \
                 'scaled', 'pending', '2026-01-01', '2026-01-01')"
    ).await.unwrap();
    let mode: String = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT scaling_mode_used FROM proxmox_resource_scaling_records WHERE update_history_id = 'h1'",
        ))
        .await.unwrap().unwrap()
        .try_get("", "scaling_mode_used").unwrap();
    assert_eq!(mode, "absolute");
}
```

- [ ] **Step 2: Run tests to see compile errors**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox controller_migration::tests::migration_c -- --nocapture 2>&1 | grep error
```

Expected: compile errors — structs not defined yet.

- [ ] **Step 3: Add `ScalingModeUsed` to `ProxmoxResourceScalingRecords` DeriveIden**

Find the `ProxmoxResourceScalingRecords` enum (around line 1297) and add `ScalingModeUsed`:

```rust
#[derive(DeriveIden)]
enum ProxmoxResourceScalingRecords {
    Table,
    UpdateHistoryId,
    TenantId,
    HostId,
    SoftwareItemId,
    PluginConfigId,
    MappingId,
    VmType,
    OriginalCores,
    OriginalMemoryMb,
    ScaledCores,
    ScaledMemoryMb,
    ScaleStatus,
    RestoreStatus,
    ErrorMessage,
    CreatedAt,
    UpdatedAt,
    ScalingModeUsed,    // <-- add this
}
```

- [ ] **Step 4: Add Migrations C, D, E structs**

Insert after the `CreateProxmoxScalingItemOverrides` struct definition (the one added in Task 1):

```rust
// ── Migration C: migrate scaling config from protection tables ──────────────

pub struct MigrateProxmoxScalingFromProtectionTables;

impl MigrationName for MigrateProxmoxScalingFromProtectionTables {
    fn name(&self) -> &str {
        "m20260504_000003_migrate_scaling_from_protection_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for MigrateProxmoxScalingFromProtectionTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Wrap all five statements in an explicit transaction.
        // Without this, if C.2 fails after C.1 succeeds, the migration is permanently
        // broken: on retry, C.1 hits the UNIQUE constraint and fails again with no recovery path.
        let txn = manager.get_connection().begin().await?;

        // C.1 — copy from proxmox_protection_defaults
        txn.execute_unprepared(
            "INSERT INTO proxmox_scaling_defaults \
             (id, tenant_id, plugin_config_id, scaling_mode, \
              absolute_cores, absolute_memory_mb, delta_cores, delta_memory_mb, \
              created_at, updated_at) \
             SELECT \
               lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || \
               lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || \
               lower(hex(randomblob(6))), \
               tenant_id, plugin_config_id, 'absolute', \
               update_cores, update_memory_mb, NULL, NULL, \
               created_at, updated_at \
             FROM proxmox_protection_defaults \
             WHERE update_cores IS NOT NULL OR update_memory_mb IS NOT NULL",
        )
        .await?;

        // C.2 — copy from proxmox_protection_item_overrides (join plugin_configs for tenant_id)
        txn.execute_unprepared(
            "INSERT INTO proxmox_scaling_item_overrides \
             (id, tenant_id, software_item_id, plugin_config_id, scaling_mode, \
              absolute_cores, absolute_memory_mb, delta_cores, delta_memory_mb, \
              created_at, updated_at) \
             SELECT \
               lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || \
               lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || \
               lower(hex(randomblob(6))), \
               pc.tenant_id, \
               pio.software_item_id, pio.plugin_config_id, 'absolute', \
               pio.update_cores, pio.update_memory_mb, NULL, NULL, \
               pio.created_at, pio.updated_at \
             FROM proxmox_protection_item_overrides pio \
             JOIN plugin_configs pc ON pc.id = pio.plugin_config_id \
             WHERE pio.update_cores IS NOT NULL OR pio.update_memory_mb IS NOT NULL",
        )
        .await?;

        // C.3 — null out source columns (D will drop them; C leaves DB coherent if D fails)
        txn.execute_unprepared(
            "UPDATE proxmox_protection_defaults SET update_cores = NULL, update_memory_mb = NULL",
        )
        .await?;
        txn.execute_unprepared(
            "UPDATE proxmox_protection_item_overrides SET update_cores = NULL, update_memory_mb = NULL",
        )
        .await?;

        txn.commit().await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Not reversible without the original data; truncate scaling tables.
        manager.get_connection()
            .execute_unprepared("DELETE FROM proxmox_scaling_defaults")
            .await?;
        manager.get_connection()
            .execute_unprepared("DELETE FROM proxmox_scaling_item_overrides")
            .await?;
        Ok(())
    }
}

// ── Migration D: drop scaling columns from protection tables ────────────────

pub struct DropProxmoxScalingColumnsFromProtectionTables;

impl MigrationName for DropProxmoxScalingColumnsFromProtectionTables {
    fn name(&self) -> &str {
        "m20260504_000004_drop_scaling_columns_from_protection_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for DropProxmoxScalingColumnsFromProtectionTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateCores)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateCores)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Columns were migrated and nulled in C; re-adding would lose data.
        Ok(())
    }
}

// ── Migration E: add scaling_mode_used to scaling records ──────────────────

pub struct AddScalingModeUsedToScalingRecord;

impl MigrationName for AddScalingModeUsedToScalingRecord {
    fn name(&self) -> &str {
        "m20260504_000005_add_scaling_mode_used_to_scaling_record"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddScalingModeUsedToScalingRecord {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxResourceScalingRecords::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ScalingModeUsed)
                            .string_len(16)
                            .not_null()
                            .default("absolute"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxResourceScalingRecords::Table)
                    .drop_column(ProxmoxResourceScalingRecords::ScalingModeUsed)
                    .to_owned(),
            )
            .await
    }
}
```

You also need DeriveIden aliases for `ProxmoxProtectionDefaults` and `ProxmoxProtectionItemOverrides` to be usable in migration D. Check if they
already exist in the file (search for `ProxmoxProtectionDefaults`). If not, add minimal enums:

```rust
#[derive(DeriveIden)]
enum ProxmoxProtectionDefaults {
    Table,
}

#[derive(DeriveIden)]
enum ProxmoxProtectionItemOverrides {
    Table,
}
```

Place them near `ProxmoxResourceScalingPolicyCols` if not already present.

- [ ] **Step 5: Append 3 more entries to `migrations()` vec**

The final vec must end with:

```rust
Box::new(AddProxmoxResourceScalingPolicyColumns),
Box::new(CreateProxmoxResourceScalingRecord),
Box::new(CreateProxmoxScalingDefaults),
Box::new(CreateProxmoxScalingItemOverrides),
Box::new(MigrateProxmoxScalingFromProtectionTables),
Box::new(DropProxmoxScalingColumnsFromProtectionTables),
Box::new(AddScalingModeUsedToScalingRecord),
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox controller_migration -- --nocapture
```

Expected: all migration tests PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/controller_migration.rs
git commit -m "feat(plugin-infrastructure-proxmox): add migrations C-E

Migrate update_cores/update_memory_mb from protection tables to dedicated
scaling tables (absolute mode). Drop old columns. Add scaling_mode_used
column to scaling records with default 'absolute' for existing rows.

Rollout risk: Migration C reads then writes in-place; run against a backup
first on production. Migration D drops columns — irreversible without a
restore. If D fails mid-flight, C has already nulled the source columns.
Testing: run migration_c_*, migration_d_*, migration_e_* tests.
Migration: must run C before D; SeaORM migrator handles ordering automatically."
```

---

## Task 3: New entity files (DO NOT COMMIT YET)

**Files:**

- Create: `crates/plugins/infrastructure/proxmox/src/entity/proxmox_scaling_default.rs`
- Create: `crates/plugins/infrastructure/proxmox/src/entity/proxmox_scaling_item_override.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/entity/mod.rs`

- [ ] **Step 1: Create `entity/proxmox_scaling_default.rs`**

```rust
#![allow(
    unreachable_pub,
    reason = "entity lives in pub(crate) mod entity; pub items are crate-internal by design"
)]

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "proxmox_scaling_defaults")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub plugin_config_id: Uuid,
    pub scaling_mode: String,
    pub absolute_cores: Option<i32>,
    pub absolute_memory_mb: Option<i32>,
    pub delta_cores: Option<i32>,
    pub delta_memory_mb: Option<i32>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 2: Create `entity/proxmox_scaling_item_override.rs`**

```rust
#![allow(
    unreachable_pub,
    reason = "entity lives in pub(crate) mod entity; pub items are crate-internal by design"
)]

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "proxmox_scaling_item_overrides")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    pub scaling_mode: String,
    pub absolute_cores: Option<i32>,
    pub absolute_memory_mb: Option<i32>,
    pub delta_cores: Option<i32>,
    pub delta_memory_mb: Option<i32>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 3: Update `entity/mod.rs`**

Add two new entries (keep alphabetical order):

```rust
pub(crate) mod proxmox_backup_target_cache;
pub(crate) mod proxmox_host_mapping;
pub(crate) mod proxmox_protection_audit;
pub(crate) mod proxmox_protection_default;
pub(crate) mod proxmox_protection_item_override;
pub(crate) mod proxmox_resource_scaling_record;
pub(crate) mod proxmox_scaling_default;          // <-- new
pub(crate) mod proxmox_scaling_item_override;    // <-- new
```

- [ ] **Step 4: Verify entity files compile**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features 2>&1 | grep "^error"
```

Expected: no errors (or only errors about missing `scaling_store` module — that's fine at this stage, it gets added in Task 4).

Do NOT commit yet — entity changes must land in the same commit as Task 5.

---

## Task 4: `scaling_store.rs` (DO NOT COMMIT YET)

**Files:**

- Create: `crates/plugins/infrastructure/proxmox/src/scaling_store.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/lib.rs`

- [ ] **Step 1: Create `scaling_store.rs`**

```rust
//! Controller-side scaling policy storage for Proxmox resource scaling v2.

use crate::entity::{proxmox_scaling_default, proxmox_scaling_item_override};
use proxmox_scaling_default::Entity as ProxmoxScalingDefault;
use proxmox_scaling_item_override::Entity as ProxmoxScalingItemOverride;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{ProxmoxError, Result};

/// Scaling mode discriminant. Internal-only; not sent over any network
/// boundary. Not `#[non_exhaustive]` — must be exhaustively matched everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScalingMode {
    #[default]
    None,
    Absolute,
    Delta,
}

impl ScalingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Absolute => "absolute",
            Self::Delta => "delta",
        }
    }
}

impl std::str::FromStr for ScalingMode {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::None),
            "absolute" => Ok(Self::Absolute),
            "delta" => Ok(Self::Delta),
            _ => Err(()),
        }
    }
}

/// Effective scaling policy resolved for a software item update.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScalingPolicy {
    pub mode: ScalingMode,
    pub absolute_cores: Option<i32>,
    pub absolute_memory_mb: Option<i32>,
    pub delta_cores: Option<i32>,
    pub delta_memory_mb: Option<i32>,
}

impl ScalingPolicy {
    pub fn none() -> Self {
        Self {
            mode: ScalingMode::None,
            absolute_cores: None,
            absolute_memory_mb: None,
            delta_cores: None,
            delta_memory_mb: None,
        }
    }

    /// True when the policy will result in at least one dimension being scaled.
    pub fn is_active(&self) -> bool {
        match self.mode {
            ScalingMode::None => false,
            ScalingMode::Absolute => {
                self.absolute_cores.is_some() || self.absolute_memory_mb.is_some()
            }
            ScalingMode::Delta => self.delta_cores.is_some() || self.delta_memory_mb.is_some(),
        }
    }
}

fn model_to_policy(model: &proxmox_scaling_default::Model) -> ScalingPolicy {
    let mode = model
        .scaling_mode
        .parse::<ScalingMode>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                value = %model.scaling_mode,
                "unrecognised scaling_mode in proxmox_scaling_defaults; treating as None"
            );
            ScalingMode::None
        });
    ScalingPolicy {
        mode,
        absolute_cores: model.absolute_cores,
        absolute_memory_mb: model.absolute_memory_mb,
        delta_cores: model.delta_cores,
        delta_memory_mb: model.delta_memory_mb,
    }
}

fn item_model_to_policy(model: &proxmox_scaling_item_override::Model) -> ScalingPolicy {
    let mode = model
        .scaling_mode
        .parse::<ScalingMode>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                value = %model.scaling_mode,
                "unrecognised scaling_mode in proxmox_scaling_item_overrides; treating as None"
            );
            ScalingMode::None
        });
    ScalingPolicy {
        mode,
        absolute_cores: model.absolute_cores,
        absolute_memory_mb: model.absolute_memory_mb,
        delta_cores: model.delta_cores,
        delta_memory_mb: model.delta_memory_mb,
    }
}

/// Load the global scaling default. Returns `ScalingPolicy::none()` if no row exists.
pub async fn load_scaling_global_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ScalingPolicy> {
    let row = ProxmoxScalingDefault::find()
        .filter(proxmox_scaling_default::Column::TenantId.eq(tenant_id))
        .filter(proxmox_scaling_default::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to load scaling global default: {e}"
            )))
        })?;
    Ok(row.as_ref().map(model_to_policy).unwrap_or_else(ScalingPolicy::none))
}

/// Load per-item scaling override. Returns `None` if no row (inherit global).
pub async fn load_scaling_item_override(
    db: &DatabaseConnection,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<Option<ScalingPolicy>> {
    let row = ProxmoxScalingItemOverride::find()
        .filter(
            proxmox_scaling_item_override::Column::SoftwareItemId.eq(software_item_id),
        )
        .filter(
            proxmox_scaling_item_override::Column::PluginConfigId.eq(plugin_config_id),
        )
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to load scaling item override: {e}"
            )))
        })?;
    Ok(row.as_ref().map(item_model_to_policy))
}

/// Resolve effective scaling policy. Item override wins over global default.
/// Dimension cascade is gated by the resolved effective mode — cross-mode
/// inheritance is forbidden (spec Goal 6).
pub async fn resolve_effective_scaling_policy(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ScalingPolicy> {
    let item = load_scaling_item_override(db, software_item_id, plugin_config_id).await?;
    let global = load_scaling_global_default(db, tenant_id, plugin_config_id).await?;

    let Some(item_policy) = item else {
        // No override row — use global as-is.
        return Ok(global);
    };

    // Resolved mode: item wins.
    let effective_mode = item_policy.mode;

    // Within the effective mode, cascade per-field from global when item is null.
    let (absolute_cores, absolute_memory_mb, delta_cores, delta_memory_mb) = match effective_mode {
        ScalingMode::Absolute => (
            item_policy.absolute_cores.or(global.absolute_cores),
            item_policy.absolute_memory_mb.or(global.absolute_memory_mb),
            None,
            None,
        ),
        ScalingMode::Delta => (
            None,
            None,
            item_policy.delta_cores.or(global.delta_cores),
            item_policy.delta_memory_mb.or(global.delta_memory_mb),
        ),
        ScalingMode::None => (None, None, None, None),
    };

    Ok(ScalingPolicy {
        mode: effective_mode,
        absolute_cores,
        absolute_memory_mb,
        delta_cores,
        delta_memory_mb,
    })
}

/// Upsert global scaling default. Uses `BEGIN IMMEDIATE` (read-then-write).
pub async fn upsert_scaling_global_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    policy: &ScalingPolicy,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to begin transaction for scaling global default upsert: {e}"
            )))
        })?;

    let existing = ProxmoxScalingDefault::find()
        .filter(proxmox_scaling_default::Column::TenantId.eq(tenant_id))
        .filter(proxmox_scaling_default::Column::PluginConfigId.eq(plugin_config_id))
        .one(&txn)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing scaling global default: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: proxmox_scaling_default::ActiveModel = existing.into();
        active.scaling_mode = Set(policy.mode.as_str().to_string());
        active.absolute_cores = Set(policy.absolute_cores);
        active.absolute_memory_mb = Set(policy.absolute_memory_mb);
        active.delta_cores = Set(policy.delta_cores);
        active.delta_memory_mb = Set(policy.delta_memory_mb);
        active.updated_at = Set(now);
        active.update(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to update scaling global default: {e}"
            )))
        })?;
    } else {
        let active = proxmox_scaling_default::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            plugin_config_id: Set(plugin_config_id),
            scaling_mode: Set(policy.mode.as_str().to_string()),
            absolute_cores: Set(policy.absolute_cores),
            absolute_memory_mb: Set(policy.absolute_memory_mb),
            delta_cores: Set(policy.delta_cores),
            delta_memory_mb: Set(policy.delta_memory_mb),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to insert scaling global default: {e}"
            )))
        })?;
    }

    txn.commit().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to commit scaling global default upsert: {e}"
        )))
    })?;
    Ok(())
}

/// Upsert per-item scaling override. Uses `BEGIN IMMEDIATE`.
pub async fn upsert_scaling_item_override(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
    policy: &ScalingPolicy,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to begin transaction for scaling item override upsert: {e}"
            )))
        })?;

    let existing = ProxmoxScalingItemOverride::find()
        .filter(
            proxmox_scaling_item_override::Column::SoftwareItemId.eq(software_item_id),
        )
        .filter(
            proxmox_scaling_item_override::Column::PluginConfigId.eq(plugin_config_id),
        )
        .one(&txn)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing scaling item override: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: proxmox_scaling_item_override::ActiveModel = existing.into();
        active.scaling_mode = Set(policy.mode.as_str().to_string());
        active.absolute_cores = Set(policy.absolute_cores);
        active.absolute_memory_mb = Set(policy.absolute_memory_mb);
        active.delta_cores = Set(policy.delta_cores);
        active.delta_memory_mb = Set(policy.delta_memory_mb);
        active.updated_at = Set(now);
        active.update(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to update scaling item override: {e}"
            )))
        })?;
    } else {
        let active = proxmox_scaling_item_override::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            software_item_id: Set(software_item_id),
            plugin_config_id: Set(plugin_config_id),
            scaling_mode: Set(policy.mode.as_str().to_string()),
            absolute_cores: Set(policy.absolute_cores),
            absolute_memory_mb: Set(policy.absolute_memory_mb),
            delta_cores: Set(policy.delta_cores),
            delta_memory_mb: Set(policy.delta_memory_mb),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to insert scaling item override: {e}"
            )))
        })?;
    }

    txn.commit().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to commit scaling item override upsert: {e}"
        )))
    })?;
    Ok(())
}

/// Delete per-item scaling override (revert item to global inheritance).
pub async fn delete_scaling_item_override(
    db: &DatabaseConnection,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<()> {
    if let Some(existing) = ProxmoxScalingItemOverride::find()
        .filter(
            proxmox_scaling_item_override::Column::SoftwareItemId.eq(software_item_id),
        )
        .filter(
            proxmox_scaling_item_override::Column::PluginConfigId.eq(plugin_config_id),
        )
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query scaling item override for delete: {e}"
            )))
        })?
    {
        let active: proxmox_scaling_item_override::ActiveModel = existing.into();
        active.delete(db).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to delete scaling item override: {e}"
            )))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_mode_round_trips() {
        for (s, expected) in &[
            ("none", ScalingMode::None),
            ("absolute", ScalingMode::Absolute),
            ("delta", ScalingMode::Delta),
        ] {
            let parsed: ScalingMode = s.parse().expect("known value must parse");
            assert_eq!(parsed, *expected);
            assert_eq!(parsed.as_str(), *s);
        }
    }

    #[test]
    fn scaling_mode_unknown_string_returns_err() {
        let result = "invalid".parse::<ScalingMode>();
        assert!(result.is_err());
    }

    #[test]
    fn scaling_policy_is_active_none_mode() {
        let policy = ScalingPolicy::none();
        assert!(!policy.is_active());
    }

    #[test]
    fn scaling_policy_is_active_absolute_requires_at_least_one_dimension() {
        let mut policy = ScalingPolicy {
            mode: ScalingMode::Absolute,
            ..Default::default()
        };
        assert!(!policy.is_active(), "no dimensions = not active");
        policy.absolute_cores = Some(4);
        assert!(policy.is_active());
    }

    #[test]
    fn scaling_policy_is_active_delta_requires_at_least_one_dimension() {
        let mut policy = ScalingPolicy {
            mode: ScalingMode::Delta,
            ..Default::default()
        };
        assert!(!policy.is_active(), "no dimensions = not active");
        policy.delta_memory_mb = Some(1024);
        assert!(policy.is_active());
    }

    #[test]
    fn resolve_effective_policy_cross_mode_gate() {
        // item mode = Delta, global mode = Absolute — delta dimensions cascade but absolute do not
        let item = Some(ScalingPolicy {
            mode: ScalingMode::Delta,
            delta_cores: None,
            delta_memory_mb: Some(1024),
            ..Default::default()
        });
        let global = ScalingPolicy {
            mode: ScalingMode::Absolute,
            absolute_cores: Some(8),
            absolute_memory_mb: Some(8192),
            delta_cores: Some(2),
            delta_memory_mb: None,
            ..Default::default()
        };

        // Simulate the merge logic from resolve_effective_scaling_policy
        let effective_mode = ScalingMode::Delta; // item wins
        let (_, _, delta_cores, delta_memory_mb) = match effective_mode {
            ScalingMode::Delta => (
                None::<i32>,
                None::<i32>,
                item.as_ref().and_then(|p| p.delta_cores).or(global.delta_cores),
                item.as_ref().and_then(|p| p.delta_memory_mb).or(global.delta_memory_mb),
            ),
            _ => unreachable!(),
        };
        assert_eq!(delta_cores, Some(2), "delta_cores cascades from global");
        assert_eq!(delta_memory_mb, Some(1024), "delta_memory_mb from item");
    }
}
```

- [ ] **Step 2: Add `scaling_store` module to `lib.rs`**

In `crates/plugins/infrastructure/proxmox/src/lib.rs`, add after `pub mod policy_store;`:

```rust
pub(crate) mod scaling_store;
```

(Not behind `plugin-ops` — store functions are needed for surface handlers which run in controller context unconditionally.)

- [ ] **Step 3: Verify it compiles in isolation**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features 2>&1 | grep error
```

Expected: no errors (or only errors about unused imports from Tasks yet to be done — that's fine at this stage).

Do NOT commit yet.

---

## Task 5: Joint commit — entity field changes + policy/protection store cleanup + test fixes

**Files:**

- Modify: `entity/proxmox_resource_scaling_record.rs`
- Modify: `entity/proxmox_protection_default.rs`
- Modify: `entity/proxmox_protection_item_override.rs`
- Modify: `policy_store.rs`
- Modify: `protection_store.rs`
- Modify: `surfaces.rs` (fix two ProtectionPolicy literal constructions)

All changes in this task go into one commit together with Tasks 3 and 4.

- [ ] **Step 1: Update `entity/proxmox_resource_scaling_record.rs`**

Remove the `dead_code` allow (entity is now fully used) and add `scaling_mode_used`:

```rust
#![allow(
    unreachable_pub,
    reason = "entity lives in pub(crate) mod entity; pub items are crate-internal by design"
)]

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "proxmox_resource_scaling_records")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub update_history_id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    pub mapping_id: Uuid,
    pub vm_type: String,
    pub original_cores: i32,
    pub original_memory_mb: i64,
    pub scaled_cores: i32,
    pub scaled_memory_mb: i64,
    pub scale_status: String,
    pub restore_status: String,
    pub error_message: Option<String>,
    pub scaling_mode_used: String,   // <-- new field
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 2: Update `entity/proxmox_protection_default.rs`**

Remove `update_cores` and `update_memory_mb`:

```rust
#![allow(
    unreachable_pub,
    reason = "entity lives in pub(crate) mod entity; pub items are crate-internal by design"
)]

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "proxmox_protection_defaults")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub plugin_config_id: Uuid,
    pub mode: String,
    pub backup_target_key: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 3: Update `entity/proxmox_protection_item_override.rs`**

Remove `update_cores` and `update_memory_mb` (same pattern as Step 2).

```rust
#![allow(
    unreachable_pub,
    reason = "entity lives in pub(crate) mod entity; pub items are crate-internal by design"
)]

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "proxmox_protection_item_overrides")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub software_item_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub plugin_config_id: Uuid,
    pub mode: String,
    pub backup_target_key: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 4: Update `policy_store.rs` — remove `update_cores` / `update_memory_mb` from `ProtectionPolicy`**

The `ProtectionPolicy` struct (around line 54) becomes:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionPolicy {
    pub mode: ProtectionMode,
    pub backup_target_key: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
}
```

Update `do_nothing()` (remove the two fields):

```rust
pub fn do_nothing() -> Self {
    Self {
        mode: ProtectionMode::DoNothing,
        backup_target_key: None,
        snapshot_timeout_seconds: None,
        backup_timeout_seconds: None,
    }
}
```

Update `resolve_effective_policy` (remove the `update_cores` / `update_memory_mb` cascade blocks and the struct literal fields).

Update `load_global_default` — remove `update_cores: model.update_cores` and `update_memory_mb: model.update_memory_mb` from the
`ProtectionPolicy { ... }` literal.

Update `upsert_global_default` — remove `active.update_cores = Set(policy.update_cores);` and
`active.update_memory_mb = Set(policy.update_memory_mb);`, and remove those fields from the insert `ActiveModel` literal.

Update `load_item_override` and `upsert_item_override` — same removals.

- [ ] **Step 5: Add `scaling_mode_used: ScalingMode` to `ScalingRecord` in `policy_store.rs`**

The `ScalingRecord` struct (around line 606) gains a new field:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalingRecord {
    pub update_history_id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    pub mapping_id: Uuid,
    pub vm_type: String,
    pub original_cores: i32,
    pub original_memory_mb: i64,
    pub scaled_cores: i32,
    pub scaled_memory_mb: i64,
    pub scale_status: String,
    pub restore_status: String,
    pub error_message: Option<String>,
    pub scaling_mode_used: crate::scaling_store::ScalingMode,   // <-- new
}
```

Update `load_scaling_record` to populate the new field with a warn-on-unknown pattern:

```rust
Ok(row.map(|m| ScalingRecord {
    update_history_id: m.update_history_id,
    tenant_id: m.tenant_id,
    host_id: m.host_id,
    software_item_id: m.software_item_id,
    plugin_config_id: m.plugin_config_id,
    mapping_id: m.mapping_id,
    vm_type: m.vm_type,
    original_cores: m.original_cores,
    original_memory_mb: m.original_memory_mb,
    scaled_cores: m.scaled_cores,
    scaled_memory_mb: m.scaled_memory_mb,
    scale_status: m.scale_status,
    restore_status: m.restore_status,
    error_message: m.error_message,
    scaling_mode_used: m.scaling_mode_used.parse::<crate::scaling_store::ScalingMode>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                value = %m.scaling_mode_used,
                "unrecognised scaling_mode_used in DB; treating as None"
            );
            crate::scaling_store::ScalingMode::None
        }),
}))
```

Update `upsert_scaling_record` to include `scaling_mode_used` in both update and insert branches:

In the update branch, after `active.error_message = Set(record.error_message.clone());` add:

```rust
active.scaling_mode_used = Set(record.scaling_mode_used.as_str().to_string());
```

In the insert branch `ActiveModel { ... }`, add:

```rust
scaling_mode_used: Set(record.scaling_mode_used.as_str().to_string()),
```

- [ ] **Step 6: Update `protection_store.rs` — remove fields from `ProxmoxProtectionPolicyRecord`**

The struct (around line 54) becomes:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ProxmoxProtectionPolicyRecord {
    pub mode: ProxmoxProtectionMode,
    pub backup_target_key: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
}
```

Search for any code in `protection_store.rs` that sets or reads `update_cores` or `update_memory_mb` and remove it.

- [ ] **Step 7: Fix `surfaces.rs` — remove dead fields from two ProtectionPolicy literals**

In `handle_save_global_defaults` (around line 1133), the `ProtectionPolicy { ... }` literal passes `update_cores: None, update_memory_mb: None`.
Remove those two lines.

In `handle_save_item_overrides` (around line 1266), same fix.

- [ ] **Step 8: Fix `policy_store.rs` tests — update all ProtectionPolicy literals**

The tests in `policy_store.rs` (around lines 748–868) construct `ProtectionPolicy` literals with `update_cores` and `update_memory_mb`. Remove those
fields from every literal. The five affected tests are:

- `effective_policy_prefers_item_override`
- `effective_policy_defaults_to_do_nothing` (no literal fields to remove — just asserts)
- `effective_policy_inherits_global_timeouts_per_field`
- `effective_policy_keeps_explicit_item_timeout`
- `protection_policy_carries_scaling_fields` — **delete this test entirely** (it tested the now-removed fields)
- `do_nothing_policy_has_no_scaling` — **delete this test entirely**
- `resolve_effective_policy_cascades_scaling_fields` — **delete this test entirely** (cascading scaling fields is now in `scaling_store.rs`)

- [ ] **Step 9: Run `cargo check` to verify everything compiles**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features 2>&1 | grep "^error"
```

Expected: no errors. (Some warnings about unused imports may appear — fix them.)

- [ ] **Step 10: Run all tests to see current state**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features 2>&1 | grep -E "FAILED|test result"
```

Fix any remaining test failures before committing.

- [ ] **Step 11: Commit everything from Tasks 3, 4, and 5 together**

```bash
git add \
  crates/plugins/infrastructure/proxmox/src/entity/proxmox_scaling_default.rs \
  crates/plugins/infrastructure/proxmox/src/entity/proxmox_scaling_item_override.rs \
  crates/plugins/infrastructure/proxmox/src/entity/mod.rs \
  crates/plugins/infrastructure/proxmox/src/entity/proxmox_resource_scaling_record.rs \
  crates/plugins/infrastructure/proxmox/src/entity/proxmox_protection_default.rs \
  crates/plugins/infrastructure/proxmox/src/entity/proxmox_protection_item_override.rs \
  crates/plugins/infrastructure/proxmox/src/scaling_store.rs \
  crates/plugins/infrastructure/proxmox/src/lib.rs \
  crates/plugins/infrastructure/proxmox/src/policy_store.rs \
  crates/plugins/infrastructure/proxmox/src/protection_store.rs \
  crates/plugins/infrastructure/proxmox/src/surfaces.rs
git commit -m "feat(plugin-infrastructure-proxmox): add ScalingMode, ScalingPolicy, scaling_store

New scaling_store.rs owns all scaling CRUD with BEGIN IMMEDIATE upserts.
ProtectionPolicy and ProxmoxProtectionPolicyRecord lose update_cores /
update_memory_mb (now in scaling tables). ScalingRecord gains
scaling_mode_used field. Entity files updated to match schema changes.

Rollout: requires Migrations A-E to be applied first (Tasks 1-2).
Risk: compile error if entity changes and policy_store changes land in
separate commits — this commit bundles all interdependent changes.
Testing: cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features"
```

---

## Task 6: Rewrite `resource_scaling.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/resource_scaling.rs`

- [ ] **Step 1: Write the new failing tests**

In the `#[cfg(test)] mod tests` block, add:

```rust
fn mock_scaling_global_default(
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    mode: crate::scaling_store::ScalingMode,
    absolute_cores: Option<i32>,
    absolute_memory_mb: Option<i32>,
    delta_cores: Option<i32>,
    delta_memory_mb: Option<i32>,
) -> crate::entity::proxmox_scaling_default::Model {
    let now = OffsetDateTime::now_utc();
    crate::entity::proxmox_scaling_default::Model {
        id: Uuid::now_v7(),
        tenant_id,
        plugin_config_id,
        scaling_mode: mode.as_str().to_string(),
        absolute_cores,
        absolute_memory_mb,
        delta_cores,
        delta_memory_mb,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn is_active_returns_false_for_none_mode() {
    use crate::scaling_store::{ScalingMode, ScalingPolicy};
    let policy = ScalingPolicy::none();
    assert!(!policy.is_active());
}

#[test]
fn is_active_returns_false_for_absolute_with_no_dimensions() {
    use crate::scaling_store::{ScalingMode, ScalingPolicy};
    let policy = ScalingPolicy { mode: ScalingMode::Absolute, ..Default::default() };
    assert!(!policy.is_active());
}

#[test]
fn delta_target_computation() {
    // delta_cores = +2, original = 4 → target = 6 (no .max(1) clamp — guard fires first)
    let original_cores: u32 = 4;
    let delta_cores: i32 = 2;
    // The integrity guard rejects delta < 1 before this path is reached,
    // so no .max(1) clamp is needed or present.
    let target = (original_cores as i64 + delta_cores as i64) as u32;
    assert_eq!(target, 6u32);
}

#[test]
fn delta_integrity_guard_condition() {
    // Verifies the guard condition fires for delta_cores = 0,
    // ensuring the hook aborts before computing a target value.
    use crate::scaling_store::{ScalingMode, ScalingPolicy};
    let policy_zero_delta = ScalingPolicy {
        mode: ScalingMode::Delta,
        delta_cores: Some(0), // violates DB CHECK constraint
        ..Default::default()
    };
    // is_active() returns true (Some(0) is Some), so without the guard the hook would proceed.
    assert!(policy_zero_delta.is_active());
    // The guard fires: delta_cores < 1
    let guard_fires = policy_zero_delta.delta_cores.map_or(false, |v| v < 1);
    assert!(guard_fires, "integrity guard must fire when delta_cores = 0");

    let policy_valid = ScalingPolicy {
        mode: ScalingMode::Delta,
        delta_cores: Some(1),
        ..Default::default()
    };
    let guard_fires_valid = policy_valid.delta_cores.map_or(false, |v| v < 1);
    assert!(!guard_fires_valid, "guard must not fire for delta_cores = 1");
}

#[test]
fn delta_partial_config_cores_only() {
    // delta_cores = 2, delta_memory_mb = None → only cores scaled
    use crate::scaling_store::{ScalingMode, ScalingPolicy};
    let policy = ScalingPolicy {
        mode: ScalingMode::Delta,
        delta_cores: Some(2),
        delta_memory_mb: None,
        ..Default::default()
    };
    assert!(policy.is_active());
    assert!(policy.delta_memory_mb.is_none());
}
```

Run to verify they compile and pass (these are pure logic tests, no DB needed):

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox resource_scaling::tests::delta_target -- --nocapture
```

- [ ] **Step 2: Update `resource_scaling.rs` imports**

Add `crate::scaling_store` to imports. Remove the direct import of `policy_store::ProtectionPolicy` / `load_effective_policy` if the pre-hook no
longer uses it for scaling. Keep the imports for `ScalingRecord` and `upsert_scaling_record` / `load_scaling_record`.

The top of the file should include:

```rust
use crate::{
    client::ProxmoxClient,
    config::ProxmoxConfig,
    policy_store::{self, ScalingRecord},
    protection_store::{DbProxmoxProtectionStore, ProxmoxProtectionStore as _},
    scaling_store::{self, ScalingMode, ScalingPolicy},
};
```

- [ ] **Step 3: Rewrite `prepare_pre_update_hook` — replace scaling policy loading**

Replace the block starting at:

```rust
// Load effective policy
let policy = match store
    .load_effective_policy(tenant_id, software_item_id, mapping.plugin_config_id)
    .await
```

With:

```rust
// Load effective scaling policy
let policy: ScalingPolicy = match scaling_store::resolve_effective_scaling_policy(
    db,
    tenant_id,
    software_item_id,
    mapping.plugin_config_id,
)
.await
{
    Ok(p) => p,
    Err(e) => {
        tracing::warn!(
            %update_history_id, error = %e,
            "resource scaling: failed to load effective scaling policy"
        );
        return;
    }
};

if !policy.is_active() {
    return;
}
```

Remove the old `if policy.update_cores.is_none() && policy.update_memory_mb.is_none() { return; }` line.

- [ ] **Step 4: Rewrite target value computation in `prepare_pre_update_hook`**

Replace the "Compute target values" block (currently reading `policy.update_cores.map(|c| c as u32).unwrap_or(...)`) with:

```rust
// Validate delta integrity: DB CHECK constraints enforce >= 1, but guard against violations.
if policy.mode == ScalingMode::Delta {
    if policy.delta_cores.map_or(false, |v| v < 1) {
        tracing::error!(
            %update_history_id, delta_cores = ?policy.delta_cores,
            "resource scaling: delta_cores < 1 violates DB CHECK constraint; \
             aborting scale-up (DB integrity violation)"
        );
        return;
    }
    if policy.delta_memory_mb.map_or(false, |v| v < 1) {
        tracing::error!(
            %update_history_id, delta_memory_mb = ?policy.delta_memory_mb,
            "resource scaling: delta_memory_mb < 1 violates DB CHECK constraint; \
             aborting scale-up (DB integrity violation)"
        );
        return;
    }
}

// No .max(1) clamp here — the integrity guard above already returned early for v < 1.
let target_cores: Option<u32> = match policy.mode {
    ScalingMode::Absolute => policy.absolute_cores.map(|v| v as u32),
    ScalingMode::Delta => policy.delta_cores.map(|v| {
        (original_cores_u32 as i64 + v as i64) as u32
    }),
    ScalingMode::None => unreachable!("is_active() returned true"),
};
let target_memory_mb: Option<u64> = match policy.mode {
    ScalingMode::Absolute => policy.absolute_memory_mb.map(|v| v as u64),
    ScalingMode::Delta => policy.delta_memory_mb.map(|v| {
        (original_memory_u64 as i64 + v as i64) as u64
    }),
    ScalingMode::None => unreachable!("is_active() returned true"),
};

// If neither dimension is configured, nothing to do.
if target_cores.is_none() && target_memory_mb.is_none() {
    return;
}

let effective_cores = target_cores.unwrap_or(original_cores_u32);
let effective_memory_mb = target_memory_mb.unwrap_or(original_memory_u64);
```

Update all downstream uses of `target_cores` and `target_memory_mb` to use `effective_cores` and `effective_memory_mb`.

- [ ] **Step 5: Add `scaling_mode_used` to `ScalingRecord` construction**

Update the `ScalingRecord { ... }` literal to include the new field:

```rust
let scaling_record = ScalingRecord {
    update_history_id,
    tenant_id,
    host_id,
    software_item_id,
    plugin_config_id: mapping.plugin_config_id,
    mapping_id: mapping.id,
    vm_type: mapping.proxmox_type.clone(),
    original_cores: i32::try_from(original_cores_u32).unwrap_or(i32::MAX),
    original_memory_mb: i64::try_from(original_memory_u64).unwrap_or(i64::MAX),
    scaled_cores: i32::try_from(effective_cores).unwrap_or(i32::MAX),
    scaled_memory_mb: i64::try_from(effective_memory_mb).unwrap_or(i64::MAX),
    scale_status: "scaling".to_string(),
    restore_status: "pending".to_string(),
    error_message: None,
    scaling_mode_used: policy.mode,    // <-- new field
};
```

(The saturating `try_from(...).unwrap_or(MAX)` also applies to `original_cores` and `original_memory_mb` — update those casts too if they currently
use `as i32` / `as i64`.)

- [ ] **Step 6: Add unmatch-race guard in `finalize_post_update_hook`**

Replace the `.ok_or_else(|| { rootcause::report!(...  "host mapping {} not found for restore" ...) })?;` call with a soft guard:

```rust
let mapping_row = match proxmox_host_mapping::Entity::find_by_id(record.mapping_id)
    .one(db)
    .await
    .map_err(|e| {
        rootcause::report!(
            uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                format!("failed to load host mapping {}: {e}", record.mapping_id)
            )
        )
    })? {
    Some(row) => row,
    None => {
        tracing::warn!(
            %update_history_id,
            mapping_id = %record.mapping_id,
            "resource scaling: host mapping deleted before restore; \
             writing skipped_mapping_deleted"
        );
        let mut skipped = record.clone();
        skipped.restore_status = "skipped_mapping_deleted".to_string();
        if let Err(e) = policy_store::upsert_scaling_record(db, &skipped).await {
            tracing::warn!(%update_history_id, error = %e,
                "resource scaling: failed to persist skipped_mapping_deleted record");
        }
        return Ok(());
    }
};
```

- [ ] **Step 7: Update the test helpers**

Replace `mock_protection_default_with_scaling` with `mock_scaling_global_default` (defined in Step 1 above).

Update `pre_update_hook_no_op_when_no_scaling_configured` — the mock DB sequence changes:

- Instead of mocking `proxmox_protection_item_override` + `proxmox_protection_default` queries for the policy, mock the
  `proxmox_scaling_item_overrides` + `proxmox_scaling_defaults` queries (empty results → `ScalingPolicy::none()` → `is_active()` = false → early
  return).

Update `pre_update_hook_skips_scaling_when_qemu_hotplug_absent` — replace `mock_protection_default_with_scaling(...)` call with
`mock_scaling_global_default(...)` call using `ScalingMode::Absolute`, `absolute_cores: Some(8)`, `absolute_memory_mb: Some(4096)`.

Update `mock_scaling_record` to include `scaling_mode_used: "absolute".to_string()` in the entity model literal.

- [ ] **Step 8: Run tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox resource_scaling -- --nocapture
```

Expected: all PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/resource_scaling.rs
git commit -m "feat(plugin-infrastructure-proxmox): rewrite resource_scaling with delta mode

Replace absolute-only policy lookup with resolve_effective_scaling_policy.
Delta mode computes target = current + delta; both dimensions independently
optional. Add scaling_mode_used to ScalingRecord. Unmatch-race guard writes
skipped_mapping_deleted instead of hard error. Saturating i32/i64 casts.

Rollout: existing absolute-mode users unaffected — policy resolution falls
through to global default as before. Unmatch race guard is backward-safe
(was a hard error; now a soft skipped_mapping_deleted write).
Testing: cargo test -p uptrakit-plugin-infrastructure-proxmox resource_scaling"
```

---

## Task 7: `surfaces.rs` — rename consts, new scaling actions and handlers; `reset.rs` update

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/surfaces.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/reset.rs`

- [ ] **Step 1: Write new failing tests in `surfaces.rs`**

Add to the test module:

```rust
#[test]
fn surface_actions_include_scaling_actions_with_correct_permissions() {
    let actions = surface_actions();
    // Now 17: 13 original + 4 scaling
    assert_eq!(actions.len(), 17);
    let ids: Vec<&str> = actions.iter().map(|a| a.action_id.as_str()).collect();
    assert!(ids.contains(&ACTION_PRELOAD_SCALING_GLOBAL_DEFAULTS));
    assert!(ids.contains(&ACTION_SAVE_SCALING_GLOBAL_DEFAULTS));
    assert!(ids.contains(&ACTION_PRELOAD_SCALING_ITEM_OVERRIDES));
    assert!(ids.contains(&ACTION_SAVE_SCALING_ITEM_OVERRIDES));

    let save_global_scaling = actions
        .iter()
        .find(|a| a.action_id == ACTION_SAVE_SCALING_GLOBAL_DEFAULTS)
        .expect("save-scaling-global-defaults must be exported");
    assert_eq!(save_global_scaling.permission, Permission::ManageGlobalSettings.as_str());

    let save_item_scaling = actions
        .iter()
        .find(|a| a.action_id == ACTION_SAVE_SCALING_ITEM_OVERRIDES)
        .expect("save-scaling-item-overrides must be exported");
    assert_eq!(save_item_scaling.permission, Permission::UpdateSoftware.as_str());
}
```

Also update the existing `surface_actions_include_host_and_policy_actions_with_permissions` test — change `assert_eq!(actions.len(), 13)` to
`assert_eq!(actions.len(), 17)`.

- [ ] **Step 2: Run tests to see failures**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox surfaces -- --nocapture 2>&1 | grep -E "FAILED|error"
```

- [ ] **Step 3: Rename surface constants and add new ones**

Replace:

```rust
const SURFACE_SETTINGS_UPDATE_PROTECTION: &str = "proxmox.settings.update-protection";
const SURFACE_SOFTWARE_ITEM_UPDATE_PROTECTION: &str = "proxmox.software-item.update-protection";
```

With:

```rust
const SURFACE_SETTINGS_UPDATE_HOOKS: &str = "proxmox.settings.update-hooks";
const SURFACE_SOFTWARE_ITEM_UPDATE_HOOKS: &str = "proxmox.software-item.update-hooks";

const ACTION_PRELOAD_SCALING_GLOBAL_DEFAULTS: &str = "preload-scaling-global-defaults";
const ACTION_SAVE_SCALING_GLOBAL_DEFAULTS: &str = "save-scaling-global-defaults";
const ACTION_PRELOAD_SCALING_ITEM_OVERRIDES: &str = "preload-scaling-item-overrides";
const ACTION_SAVE_SCALING_ITEM_OVERRIDES: &str = "save-scaling-item-overrides";
```

- [ ] **Step 4: Add new `ControllerSurfaceAction` variants**

In the `enum ControllerSurfaceAction`:

```rust
enum ControllerSurfaceAction {
    // ... existing variants ...
    PreloadScalingGlobalDefaults,
    SaveScalingGlobalDefaults,
    PreloadScalingItemOverrides,
    SaveScalingItemOverrides,
}
```

- [ ] **Step 5: Update `resolve_controller_surface_action`**

Replace the two arms that used `SURFACE_SETTINGS_UPDATE_PROTECTION` and `SURFACE_SOFTWARE_ITEM_UPDATE_PROTECTION` to use the new
`SURFACE_SETTINGS_UPDATE_HOOKS` and `SURFACE_SOFTWARE_ITEM_UPDATE_HOOKS` constants. Add four new arms:

```rust
(SURFACE_SETTINGS_UPDATE_HOOKS, ACTION_PRELOAD_SCALING_GLOBAL_DEFAULTS) => {
    Some(ControllerSurfaceAction::PreloadScalingGlobalDefaults)
}
(SURFACE_SETTINGS_UPDATE_HOOKS, ACTION_SAVE_SCALING_GLOBAL_DEFAULTS) => {
    Some(ControllerSurfaceAction::SaveScalingGlobalDefaults)
}
(SURFACE_SOFTWARE_ITEM_UPDATE_HOOKS, ACTION_PRELOAD_SCALING_ITEM_OVERRIDES) => {
    Some(ControllerSurfaceAction::PreloadScalingItemOverrides)
}
(SURFACE_SOFTWARE_ITEM_UPDATE_HOOKS, ACTION_SAVE_SCALING_ITEM_OVERRIDES) => {
    Some(ControllerSurfaceAction::SaveScalingItemOverrides)
}
```

- [ ] **Step 6: Add new request structs**

After `ProxmoxItemOverrideSaveRequest`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProxmoxScalingGlobalDefaultsSaveRequest {
    pub plugin_config_id: uuid::Uuid,
    pub scaling_mode: String,
    pub absolute_cores: Option<i32>,
    pub absolute_memory_mb: Option<i32>,
    pub delta_cores: Option<i32>,
    pub delta_memory_mb: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProxmoxScalingItemOverridesSaveRequest {
    pub software_item_id: uuid::Uuid,
    pub plugin_config_id: uuid::Uuid,
    /// "inherit" | "none" | "absolute" | "delta"
    pub scaling_mode: String,
    pub absolute_cores: Option<i32>,
    pub absolute_memory_mb: Option<i32>,
    pub delta_cores: Option<i32>,
    pub delta_memory_mb: Option<i32>,
}
```

- [ ] **Step 7: Add 4 new action descriptor functions**

```rust
fn preload_scaling_global_defaults_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new(
        ACTION_PRELOAD_SCALING_GLOBAL_DEFAULTS,
        "Preload Scaling Global Defaults",
    )
    .with_permission(Permission::ManageGlobalSettings)
}

fn save_scaling_global_defaults_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new(
        ACTION_SAVE_SCALING_GLOBAL_DEFAULTS,
        "Save Scaling Global Defaults",
    )
    .with_permission(Permission::ManageGlobalSettings)
}

fn preload_scaling_item_overrides_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new(
        ACTION_PRELOAD_SCALING_ITEM_OVERRIDES,
        "Preload Per-item Scaling Overrides",
    )
    .with_permission(Permission::ViewSoftware)
}

fn save_scaling_item_overrides_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new(
        ACTION_SAVE_SCALING_ITEM_OVERRIDES,
        "Save Per-item Scaling Overrides",
    )
    .with_permission(Permission::UpdateSoftware)
}
```

- [ ] **Step 8: Update `surface_actions()` vec**

Add the four new calls:

```rust
pub fn surface_actions() -> Vec<SurfaceActionDescriptor> {
    vec![
        // ... existing 13 entries unchanged ...
        preload_scaling_global_defaults_action(),
        save_scaling_global_defaults_action(),
        preload_scaling_item_overrides_action(),
        save_scaling_item_overrides_action(),
    ]
}
```

- [ ] **Step 9: Add scaling import to surfaces.rs**

Near the top, with existing policy_store imports, add:

```rust
use crate::scaling_store::{
    ScalingMode, ScalingPolicy, delete_scaling_item_override,
    load_scaling_global_default, load_scaling_item_override,
    upsert_scaling_global_default, upsert_scaling_item_override,
};
```

- [ ] **Step 10: Add 4 validation helper functions**

```rust
// Use ScalingMode's FromStr impl (defined in scaling_store.rs) rather than duplicating the match.
fn parse_scaling_mode_global(value: &str) -> std::result::Result<ScalingMode, String> {
    value.trim().parse::<ScalingMode>().map_err(|_| {
        format!(
            "invalid scaling_mode '{}'; expected none, absolute, or delta",
            value.trim()
        )
    })
}

fn parse_scaling_mode_item(value: &str) -> std::result::Result<Option<ScalingMode>, String> {
    // Returns None to signal "inherit" (delete override row).
    match value.trim() {
        "inherit" => Ok(None),
        other => other.parse::<ScalingMode>().map(Some).map_err(|_| {
            format!(
                "invalid scaling_mode '{}'; expected inherit, none, absolute, or delta",
                other
            )
        }),
    }
}

fn validate_scaling_dimensions(
    mode: ScalingMode,
    absolute_cores: Option<i32>,
    absolute_memory_mb: Option<i32>,
    delta_cores: Option<i32>,
    delta_memory_mb: Option<i32>,
) -> std::result::Result<ScalingPolicy, String> {
    match mode {
        ScalingMode::None => Ok(ScalingPolicy::none()),
        ScalingMode::Absolute => {
            if delta_cores.is_some() || delta_memory_mb.is_some() {
                return Err(
                    "cross-mode fields rejected: delta_cores/delta_memory_mb must be null \
                     when scaling_mode = absolute"
                        .to_string(),
                );
            }
            for (val, name) in [(absolute_cores, "absolute_cores"), (absolute_memory_mb, "absolute_memory_mb")] {
                if let Some(v) = val {
                    if v < 1 {
                        return Err(format!("{name} must be >= 1"));
                    }
                }
            }
            if absolute_cores.is_none() && absolute_memory_mb.is_none() {
                return Err(
                    "at least one dimension (absolute_cores or absolute_memory_mb) \
                     must be set when scaling_mode = absolute"
                        .to_string(),
                );
            }
            Ok(ScalingPolicy {
                mode,
                absolute_cores,
                absolute_memory_mb,
                delta_cores: None,
                delta_memory_mb: None,
            })
        }
        ScalingMode::Delta => {
            if absolute_cores.is_some() || absolute_memory_mb.is_some() {
                return Err(
                    "cross-mode fields rejected: absolute_cores/absolute_memory_mb must be null \
                     when scaling_mode = delta"
                        .to_string(),
                );
            }
            for (val, name) in [(delta_cores, "delta_cores"), (delta_memory_mb, "delta_memory_mb")] {
                if let Some(v) = val {
                    if v < 1 {
                        return Err(format!("{name} must be >= 1"));
                    }
                }
            }
            if delta_cores.is_none() && delta_memory_mb.is_none() {
                return Err(
                    "at least one dimension (delta_cores or delta_memory_mb) \
                     must be set when scaling_mode = delta"
                        .to_string(),
                );
            }
            Ok(ScalingPolicy {
                mode,
                absolute_cores: None,
                absolute_memory_mb: None,
                delta_cores,
                delta_memory_mb,
            })
        }
    }
}
```

- [ ] **Step 11: Add 4 handler functions**

```rust
async fn handle_preload_scaling_global_defaults(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxScopeSelectionRequest,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = require_tenant_id(tenant_id, "scaling global defaults preload")?;
    let configs = resolve_scope_plugin_configs(db, tenant_id, &request).await?;

    let Some(selected_config) = configs.first() else {
        return Ok(json!({
            "plugin_config_id": "",
            "scaling_mode": "none",
            "absolute_cores": serde_json::Value::Null,
            "absolute_memory_mb": serde_json::Value::Null,
            "delta_cores": serde_json::Value::Null,
            "delta_memory_mb": serde_json::Value::Null,
        }));
    };

    let policy = load_scaling_global_default(db, tenant_id, selected_config.id)
        .await
        .map_err(|e| format!("failed to load scaling global defaults: {e}"))?;

    Ok(json!({
        "plugin_config_id": selected_config.id.to_string(),
        "scaling_mode": policy.mode.as_str(),
        "absolute_cores": policy.absolute_cores,
        "absolute_memory_mb": policy.absolute_memory_mb,
        "delta_cores": policy.delta_cores,
        "delta_memory_mb": policy.delta_memory_mb,
    }))
}

async fn handle_save_scaling_global_defaults(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxScalingGlobalDefaultsSaveRequest,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = require_tenant_id(tenant_id, "scaling global defaults save")?;
    let plugin_config_id = request.plugin_config_id;

    let mode = parse_scaling_mode_global(&request.scaling_mode)?;
    let policy = validate_scaling_dimensions(
        mode,
        request.absolute_cores,
        request.absolute_memory_mb,
        request.delta_cores,
        request.delta_memory_mb,
    )?;

    ensure_proxmox_plugin_config_exists(db, tenant_id, plugin_config_id).await?;

    upsert_scaling_global_default(db, tenant_id, plugin_config_id, &policy)
        .await
        .map_err(|e| format!("failed to save scaling global defaults: {e}"))?;

    Ok(json!({ "success": true, "plugin_config_id": plugin_config_id.to_string() }))
}

async fn handle_preload_scaling_item_overrides(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxItemOverridePreloadRequest,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = require_tenant_id(tenant_id, "scaling item overrides preload")?;
    let software_item_id = request.software_item_id;
    let configs = resolve_scope_plugin_configs(
        db,
        tenant_id,
        &ProxmoxScopeSelectionRequest {
            plugin_config_id: request.plugin_config_id,
            software_item_id: Some(software_item_id),
        },
    )
    .await?;

    let Some(selected_config) = configs.first() else {
        return Ok(json!({
            "software_item_id": software_item_id.to_string(),
            "plugin_config_id": "",
            "scaling_mode": "inherit",
            "absolute_cores": serde_json::Value::Null,
            "absolute_memory_mb": serde_json::Value::Null,
            "delta_cores": serde_json::Value::Null,
            "delta_memory_mb": serde_json::Value::Null,
        }));
    };

    let item_override =
        load_scaling_item_override(db, software_item_id, selected_config.id)
            .await
            .map_err(|e| format!("failed to load scaling item override: {e}"))?;

    let (scaling_mode_str, abs_c, abs_m, del_c, del_m) = match item_override {
        None => ("inherit".to_string(), None, None, None, None),
        Some(p) => (
            p.mode.as_str().to_string(),
            p.absolute_cores,
            p.absolute_memory_mb,
            p.delta_cores,
            p.delta_memory_mb,
        ),
    };

    Ok(json!({
        "software_item_id": software_item_id.to_string(),
        "plugin_config_id": selected_config.id.to_string(),
        "scaling_mode": scaling_mode_str,
        "absolute_cores": abs_c,
        "absolute_memory_mb": abs_m,
        "delta_cores": del_c,
        "delta_memory_mb": del_m,
    }))
}

async fn handle_save_scaling_item_overrides(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxScalingItemOverridesSaveRequest,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = require_tenant_id(tenant_id, "scaling item overrides save")?;
    let software_item_id = request.software_item_id;
    let plugin_config_id = request.plugin_config_id;

    ensure_proxmox_plugin_config_exists(db, tenant_id, plugin_config_id).await?;
    ensure_plugin_config_assigned_to_software_item(
        db,
        tenant_id,
        software_item_id,
        plugin_config_id,
    )
    .await?;

    let mode_opt = parse_scaling_mode_item(&request.scaling_mode)?;

    let Some(mode) = mode_opt else {
        // "inherit" → delete override row
        delete_scaling_item_override(db, software_item_id, plugin_config_id)
            .await
            .map_err(|e| format!("failed to clear scaling item override: {e}"))?;
        return Ok(json!({
            "success": true,
            "cleared": true,
            "software_item_id": software_item_id.to_string(),
            "plugin_config_id": plugin_config_id.to_string(),
        }));
    };

    let policy = validate_scaling_dimensions(
        mode,
        request.absolute_cores,
        request.absolute_memory_mb,
        request.delta_cores,
        request.delta_memory_mb,
    )?;

    upsert_scaling_item_override(db, tenant_id, software_item_id, plugin_config_id, &policy)
        .await
        .map_err(|e| format!("failed to save scaling item override: {e}"))?;

    Ok(json!({
        "success": true,
        "cleared": false,
        "software_item_id": software_item_id.to_string(),
        "plugin_config_id": plugin_config_id.to_string(),
    }))
}
```

- [ ] **Step 12: Wire new handlers into `execute_controller_surface_action_typed`**

In the `match route { ... }` block, add four new arms:

```rust
ControllerSurfaceAction::PreloadScalingGlobalDefaults => {
    handle_preload_scaling_global_defaults(
        db,
        tenant_id,
        parse_action_params::<ProxmoxScopeSelectionRequest>(params, action_id)?,
    )
    .await
    .map_err(map_controller_action_error)
}
ControllerSurfaceAction::SaveScalingGlobalDefaults => {
    handle_save_scaling_global_defaults(
        db,
        tenant_id,
        parse_action_params::<ProxmoxScalingGlobalDefaultsSaveRequest>(params, action_id)?,
    )
    .await
    .map_err(map_controller_action_error)
}
ControllerSurfaceAction::PreloadScalingItemOverrides => {
    handle_preload_scaling_item_overrides(
        db,
        tenant_id,
        parse_action_params::<ProxmoxItemOverridePreloadRequest>(params, action_id)?,
    )
    .await
    .map_err(map_controller_action_error)
}
ControllerSurfaceAction::SaveScalingItemOverrides => {
    handle_save_scaling_item_overrides(
        db,
        tenant_id,
        parse_action_params::<ProxmoxScalingItemOverridesSaveRequest>(params, action_id)?,
    )
    .await
    .map_err(map_controller_action_error)
}
```

- [ ] **Step 13: Update `reset.rs` — add deletion of new scaling tables**

Add imports and deletions before the `proxmox_resource_scaling_record` deletion block:

```rust
use crate::entity::{proxmox_scaling_default, proxmox_scaling_item_override};

proxmox_scaling_item_override::Entity::delete_many()
    .filter(proxmox_scaling_item_override::Column::TenantId.eq(tenant_id))
    .exec(txn)
    .await?;

proxmox_scaling_default::Entity::delete_many()
    .filter(proxmox_scaling_default::Column::TenantId.eq(tenant_id))
    .exec(txn)
    .await?;
```

Place these before the `proxmox_host_mapping` deletion (FK-safe order). The scaling tables have no FK to host_mapping.

- [ ] **Step 14: Run tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox surfaces -- --nocapture
```

Expected: all PASS including the new scaling action tests.

- [ ] **Step 15: Commit**

```bash
git add \
  crates/plugins/infrastructure/proxmox/src/surfaces.rs \
  crates/plugins/infrastructure/proxmox/src/reset.rs
git commit -m "feat(plugin-infrastructure-proxmox): scaling surface actions and handlers

Rename update-protection surfaces to update-hooks. Add 4 new surface
actions for resource scaling (preload/save × global/item). Handlers use
scaling_store; validate cross-mode fields rejected. reset.rs deletes
proxmox_scaling_defaults and proxmox_scaling_item_overrides on tenant reset.

Rollout risk: surface ID rename (update-protection → update-hooks) is a
breaking change for any hardcoded surface references. Dynamic discovery
(frontend registry) handles this transparently — verify with Task 10 Step 3.
Testing: cargo test -p uptrakit-plugin-infrastructure-proxmox surfaces"
```

---

## Task 8: `plugin.rs` — rename surfaces, add scaling interactions

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs`

- [ ] **Step 1: Rename `proxmox_settings_update_protection_surface`**

Rename the function to `proxmox_settings_update_hooks_surface`. Update:

- `surface_id` from `"proxmox.settings.update-protection"` to `"proxmox.settings.update-hooks"`
- `label` from `"Proxmox Update Protection"` to `"Proxmox Update Hooks"`

- [ ] **Step 2: Restructure root_node to nested sections**

Replace the existing `root_node`:

```rust
SurfaceNode::Section {
    title: None,
    children: vec![
        SurfaceNode::Callout { ... },
        SurfaceNode::Form {
            interaction_id: ..., // save-global-defaults
        },
    ],
}
```

With:

```rust
SurfaceNode::Section {
    title: None,
    children: vec![
        SurfaceNode::Callout {
            level: surfaces::CalloutLevel::Info,
            text: callout,
        },
        SurfaceNode::Section {
            title: Some("Update Protection".to_string()),
            children: vec![
                SurfaceNode::Form {
                    interaction_id: surfaces::InteractionId::new("save-global-defaults")
                        .expect("literal interaction id is valid"),
                },
            ],
        },
        SurfaceNode::Section {
            title: Some("Resource Scaling".to_string()),
            children: vec![
                SurfaceNode::Form {
                    interaction_id: surfaces::InteractionId::new("save-scaling-global-defaults")
                        .expect("literal interaction id is valid"),
                },
            ],
        },
    ],
}
```

- [ ] **Step 3: Add 4 new `InteractionDescriptor` entries to settings surface**

After the existing `save-global-defaults` interaction, add:

```rust
surfaces::InteractionDescriptor {
    interaction_id: surfaces::InteractionId::new("preload-scaling-global-defaults")
        .expect("literal interaction id is valid"),
    kind: surfaces::InteractionKind::DataLoad,
    label: "Preload Scaling Global Defaults".to_string(),
    required_permission: Some(Permission::ManageGlobalSettings.to_string()),
    input_schema: Some(surfaces::SchemaContract::Object),
    result_schema: Some(surfaces::SchemaContract::Object),
    sensitive_fields: vec![],
    timeout_seconds: None,
    confirmation: None,
    transport: surfaces::InteractionTransport::ControllerLocal,
    workflow_steps: vec![],
    form_ui: None,
    icon: None,
},
surfaces::InteractionDescriptor {
    interaction_id: surfaces::InteractionId::new("save-scaling-global-defaults")
        .expect("literal interaction id is valid"),
    kind: surfaces::InteractionKind::MutationAction,
    label: "Save Scaling Global Defaults".to_string(),
    required_permission: Some(Permission::ManageGlobalSettings.to_string()),
    input_schema: Some(surfaces::SchemaContract::Object),
    result_schema: Some(surfaces::SchemaContract::Any),
    sensitive_fields: vec![],
    timeout_seconds: None,
    confirmation: None,
    transport: surfaces::InteractionTransport::ControllerLocal,
    workflow_steps: vec![],
    form_ui: Some(surfaces::FormUiDescriptor {
        fields: vec![
            surfaces::FormFieldDescriptor {
                key: "plugin_config_id".to_string(),
                label: "Proxmox Configuration".to_string(),
                field_type: "select".to_string(),
                required: true,
                placeholder: None,
                help_text: Some(
                    "Select the Proxmox plugin configuration this default applies to."
                        .to_string(),
                ),
                default_value: None,
                options: vec![],
                select_source: Some(surfaces::FormSelectSource::RestApi {
                    path: "/api/v1/plugin-configs?plugin_type=infrastructure_proxmox"
                        .to_string(),
                    value_field: "id".to_string(),
                    label_field: "name".to_string(),
                }),
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "scaling_mode".to_string(),
                label: "Scaling Mode".to_string(),
                field_type: "select".to_string(),
                required: true,
                placeholder: None,
                help_text: Some(
                    "None: no scaling. Absolute: set fixed cores/memory. \
                     Delta: add cores/memory to current values."
                        .to_string(),
                ),
                default_value: Some("none".to_string()),
                options: vec![
                    surfaces::FormSelectOption {
                        value: "none".to_string(),
                        label: "None (disabled)".to_string(),
                    },
                    surfaces::FormSelectOption {
                        value: "absolute".to_string(),
                        label: "Absolute".to_string(),
                    },
                    surfaces::FormSelectOption {
                        value: "delta".to_string(),
                        label: "Delta (+N)".to_string(),
                    },
                ],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "absolute_cores".to_string(),
                label: "CPU Cores (absolute)".to_string(),
                field_type: "number".to_string(),
                required: false,
                placeholder: Some("4".to_string()),
                help_text: Some("Fixed number of vCPU cores during update.".to_string()),
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: Some(surfaces::FormVisibleWhen {
                    field: "scaling_mode".to_string(),
                    values: vec!["absolute".to_string()],
                }),
            },
            surfaces::FormFieldDescriptor {
                key: "absolute_memory_mb".to_string(),
                label: "Memory MB (absolute)".to_string(),
                field_type: "number".to_string(),
                required: false,
                placeholder: Some("4096".to_string()),
                help_text: Some("Fixed RAM in MB during update.".to_string()),
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: Some(surfaces::FormVisibleWhen {
                    field: "scaling_mode".to_string(),
                    values: vec!["absolute".to_string()],
                }),
            },
            surfaces::FormFieldDescriptor {
                key: "delta_cores".to_string(),
                label: "CPU Cores (+delta)".to_string(),
                field_type: "number".to_string(),
                required: false,
                placeholder: Some("2".to_string()),
                help_text: Some("Cores to add to current vCPU count during update.".to_string()),
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: Some(surfaces::FormVisibleWhen {
                    field: "scaling_mode".to_string(),
                    values: vec!["delta".to_string()],
                }),
            },
            surfaces::FormFieldDescriptor {
                key: "delta_memory_mb".to_string(),
                label: "Memory MB (+delta)".to_string(),
                field_type: "number".to_string(),
                required: false,
                placeholder: Some("1024".to_string()),
                help_text: Some("MB to add to current RAM during update.".to_string()),
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: Some(surfaces::FormVisibleWhen {
                    field: "scaling_mode".to_string(),
                    values: vec!["delta".to_string()],
                }),
            },
        ],
        pre_load_interaction_id: Some(
            surfaces::InteractionId::new("preload-scaling-global-defaults")
                .expect("literal interaction id is valid"),
        ),
    }),
    icon: None,
},
```

- [ ] **Step 4: Apply same pattern to `proxmox_software_item_update_hooks_surface`**

Rename `proxmox_software_item_update_protection_surface` → `proxmox_software_item_update_hooks_surface`. Update:

- `surface_id` from `"proxmox.software-item.update-protection"` to `"proxmox.software-item.update-hooks"`
- `label` from `"Proxmox Update Protection"` to `"Proxmox Update Hooks"`

Restructure root_node with nested sections (same pattern as settings surface in Step 2). Add 2 new `InteractionDescriptor` entries after the
existing `save-item-overrides` interaction:

```rust
surfaces::InteractionDescriptor {
    interaction_id: surfaces::InteractionId::new("preload-scaling-item-overrides")
        .expect("literal interaction id is valid"),
    kind: surfaces::InteractionKind::DataLoad,
    label: "Preload Per-item Scaling Overrides".to_string(),
    required_permission: Some(Permission::ViewSoftware.to_string()),
    input_schema: Some(surfaces::SchemaContract::Object),
    result_schema: Some(surfaces::SchemaContract::Object),
    sensitive_fields: vec![],
    timeout_seconds: None,
    confirmation: None,
    transport: surfaces::InteractionTransport::ControllerLocal,
    workflow_steps: vec![],
    form_ui: None,
    icon: None,
},
surfaces::InteractionDescriptor {
    interaction_id: surfaces::InteractionId::new("save-scaling-item-overrides")
        .expect("literal interaction id is valid"),
    kind: surfaces::InteractionKind::MutationAction,
    label: "Save Per-item Scaling Overrides".to_string(),
    required_permission: Some(Permission::UpdateSoftware.to_string()),
    input_schema: Some(surfaces::SchemaContract::Object),
    result_schema: Some(surfaces::SchemaContract::Any),
    sensitive_fields: vec![],
    timeout_seconds: None,
    confirmation: None,
    transport: surfaces::InteractionTransport::ControllerLocal,
    workflow_steps: vec![],
    form_ui: Some(surfaces::FormUiDescriptor {
        fields: vec![
            surfaces::FormFieldDescriptor {
                key: "software_item_id".to_string(),
                label: "Software Item".to_string(),
                field_type: "hidden".to_string(),
                required: true,
                placeholder: None,
                help_text: None,
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "plugin_config_id".to_string(),
                label: "Proxmox Configuration".to_string(),
                field_type: "hidden".to_string(),
                required: true,
                placeholder: None,
                help_text: None,
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "scaling_mode".to_string(),
                label: "Scaling Mode".to_string(),
                field_type: "select".to_string(),
                required: true,
                placeholder: None,
                help_text: Some(
                    "Inherit: use global default. None: opt out. \
                     Absolute: set fixed values. Delta: add to current values."
                        .to_string(),
                ),
                default_value: Some("inherit".to_string()),
                options: vec![
                    surfaces::FormSelectOption {
                        value: "inherit".to_string(),
                        label: "Inherit global default".to_string(),
                    },
                    surfaces::FormSelectOption {
                        value: "none".to_string(),
                        label: "None (opt out)".to_string(),
                    },
                    surfaces::FormSelectOption {
                        value: "absolute".to_string(),
                        label: "Absolute".to_string(),
                    },
                    surfaces::FormSelectOption {
                        value: "delta".to_string(),
                        label: "Delta (+N)".to_string(),
                    },
                ],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "absolute_cores".to_string(),
                label: "CPU Cores (absolute)".to_string(),
                field_type: "number".to_string(),
                required: false,
                placeholder: Some("4".to_string()),
                help_text: Some("Fixed number of vCPU cores during update.".to_string()),
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: Some(surfaces::FormVisibleWhen {
                    field: "scaling_mode".to_string(),
                    values: vec!["absolute".to_string()],
                }),
            },
            surfaces::FormFieldDescriptor {
                key: "absolute_memory_mb".to_string(),
                label: "Memory MB (absolute)".to_string(),
                field_type: "number".to_string(),
                required: false,
                placeholder: Some("4096".to_string()),
                help_text: Some("Fixed RAM in MB during update.".to_string()),
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: Some(surfaces::FormVisibleWhen {
                    field: "scaling_mode".to_string(),
                    values: vec!["absolute".to_string()],
                }),
            },
            surfaces::FormFieldDescriptor {
                key: "delta_cores".to_string(),
                label: "CPU Cores (+delta)".to_string(),
                field_type: "number".to_string(),
                required: false,
                placeholder: Some("2".to_string()),
                help_text: Some("Cores to add to current vCPU count during update.".to_string()),
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: Some(surfaces::FormVisibleWhen {
                    field: "scaling_mode".to_string(),
                    values: vec!["delta".to_string()],
                }),
            },
            surfaces::FormFieldDescriptor {
                key: "delta_memory_mb".to_string(),
                label: "Memory MB (+delta)".to_string(),
                field_type: "number".to_string(),
                required: false,
                placeholder: Some("1024".to_string()),
                help_text: Some("MB to add to current RAM during update.".to_string()),
                default_value: None,
                options: vec![],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: Some(surfaces::FormVisibleWhen {
                    field: "scaling_mode".to_string(),
                    values: vec!["delta".to_string()],
                }),
            },
        ],
        pre_load_interaction_id: Some(
            surfaces::InteractionId::new("preload-scaling-item-overrides")
                .expect("literal interaction id is valid"),
        ),
    }),
    icon: None,
},
```

- [ ] **Step 5: Update call sites**

Search for `proxmox_settings_update_protection_surface()` and `proxmox_software_item_update_protection_surface()` in plugin.rs and rename to the new
function names.

- [ ] **Step 6: Verify compilation**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/plugin.rs
git commit -m "feat(plugin-infrastructure-proxmox): rename surfaces to update-hooks, add scaling UI

Rename update-protection surfaces to update-hooks (new IDs and labels).
Nest existing protection form in Update Protection section. Add Resource
Scaling section with absolute/delta mode selector and mode-gated dimension
fields using FormVisibleWhen single-field conditions.

Rollout: surface ID rename reflected here must match surfaces.rs rename from
Task 7 — both commits required before running the frontend. Screenshots
required in PR body (UI changes).
Testing: cargo check + Task 10 full UI verification with screenshots."
```

---

## Task 9: Quality gates

**Files:** none (run-only task)

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

- [ ] **Step 2: Clippy (SQLite features)**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 3: Clippy (all features)**

```bash
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | grep "^error"
```

Expected: no errors. Fix any `#[allow(...)]` that should be `#[expect(..., reason = "...")]`.

- [ ] **Step 4: `cargo deny check`**

```bash
cargo deny check
```

Expected: PASS.

- [ ] **Step 5: Full test suite**

```bash
cargo test --all-features 2>&1 | grep -E "FAILED|test result"
```

Expected: 0 failures.

- [ ] **Step 6: Commit format changes if any**

If `cargo fmt` changed files:

```bash
git add -p  # stage only fmt changes
git commit -m "style(plugin-infrastructure-proxmox): cargo fmt"
```

---

## Task 10: Frontend verification and screenshots

**Files:** none (run app + screenshots task)

- [ ] **Step 1: Start the backend**

```bash
cargo run --all-features
```

- [ ] **Step 2: Start the frontend dev server**

```bash
cd frontend && npm run dev
```

- [ ] **Step 3: Verify no hardcoded surface ID references need updating**

```bash
grep -r "update-protection" frontend/src/ --include="*.ts" --include="*.svelte"
```

Expected: no matches (surfaces are discovered dynamically).

- [ ] **Step 4: Exercise settings surface — Proxmox Update Hooks → Resource Scaling**

Navigate to Settings → Proxmox Update Hooks. Verify:

- The page has two sections: "Update Protection" and "Resource Scaling".
- Default state: `scaling_mode = none`, no dimension fields visible.
- Switching to `absolute`: `absolute_cores` and `absolute_memory_mb` appear; delta fields hidden.
- Switching to `delta`: `delta_cores` and `delta_memory_mb` appear; absolute fields hidden.
- Saving `absolute` with no dimensions filled: server returns validation error.
- Saving `delta` with `delta_cores = 0`: server returns validation error.
- Saving `delta` with `delta_cores = 2`, `delta_memory_mb` empty: succeeds.
- Reload: saved values appear pre-populated.

Take a screenshot with each mode selected (3 screenshots).

- [ ] **Step 5: Exercise software-item surface**

Navigate to a Software Item → Proxmox Update Hooks → Resource Scaling section. Verify:

- Default: `scaling_mode = inherit`, no dimension fields visible.
- Switching to `none`: no fields; save succeeds; reload shows `none`.
- Switching to `absolute`: cores/memory fields appear; delta fields hidden.
- Saving `absolute` with no dimensions: server validation error.
- Saving `absolute` with `absolute_cores = 4`: succeeds; reload shows pre-populated value.
- Switching to `inherit` and saving: clears override; subsequent reload shows `inherit`.

Take a screenshot with each mode selected (4 screenshots).

- [ ] **Step 6: Frontend build**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: all PASS.

---

## Task 11: Documentation update

**Files:**

- Modify: `docs/development/coding-standards.md`

- [ ] **Step 1: Add three-state override pattern note**

Find the "Database Query Patterns" section in `docs/development/coding-standards.md` (or the appropriate per-item policy section). Add a new
subsection:

```markdown
### Per-item policy override pattern

Use the **three-state override** model for per-item policy configuration:

- **Inherit** — no override row exists; effective policy comes from global defaults.
- **Disable** — override row with the "none"/"disabled" mode; item opts out regardless of global.
- **Configure** — override row with a real mode + field values; item has an explicit policy.

Row-level inheritance is signalled by the absence of a row, not by null field values.

Within a configured override row, a null dimension value inherits the global default for that dimension (per-field cascade). For example: an item
override with `scaling_mode = delta`, `delta_cores = 2`, and `delta_memory_mb = NULL` will use the global default's `delta_memory_mb` at runtime.
The UI should communicate this by labeling null/empty fields "inherit from global".

When implementing surfaces for three-state policies: use a 4-value `scaling_mode` selector (`inherit` / `none` / mode-specific values) so
`FormVisibleWhen`'s single-field condition can gate dimension fields without compound logic. Cross-mode field inheritance is forbidden: if the
effective mode is `delta`, only `delta_*` dimensions cascade from global; `absolute_*` dimensions are cleared even if the global has them set.
```

- [ ] **Step 2: Verify markdown passes lint**

```bash
npx prettier --write docs/development/coding-standards.md
markdownlint --config .markdownlint.json docs/development/coding-standards.md
```

Expected: no lint errors.

- [ ] **Step 3: Commit**

```bash
git add docs/development/coding-standards.md
git commit -m "docs(coding-standards): document three-state per-item policy override pattern

Based on Proxmox resource scaling v2 implementation. Replaces any
implicit 'null = inherit' anti-pattern with explicit absence-of-row
semantics for per-item overrides.

No rollout risk (docs only). Testing: markdownlint passes."
```

---

## Self-review

### Spec coverage

| Spec requirement                                              | Task                                        |
| ------------------------------------------------------------- | ------------------------------------------- |
| Delta mode: target = current + delta                          | Task 6                                      |
| `scaling_mode` discriminant (none/absolute/delta)             | Task 4                                      |
| Global defaults + per-item overrides default to `none`        | Tasks 3–4                                   |
| Cross-mode field inheritance gated by effective mode          | Task 4 (`resolve_effective_scaling_policy`) |
| `scaling_mode_used` in scaling records                        | Tasks 1E, 5, 6                              |
| Unmatch race: `skipped_mapping_deleted`                       | Task 6                                      |
| DB CHECK constraints ≥ 1                                      | Task 1 (raw SQL CREATE TABLE)               |
| API-layer validation rejects cross-mode fields                | Task 7 (`validate_scaling_dimensions`)      |
| Per-item 4-state `scaling_mode` (inherit/none/absolute/delta) | Tasks 7–8                                   |
| Surface rename to `proxmox.settings.update-hooks`             | Tasks 7–8                                   |
| Form fields mode-gated via `FormVisibleWhen`                  | Task 8                                      |
| `reset_tenant_data` deletes new tables                        | Task 7                                      |
| Data migration C from protection tables                       | Task 2                                      |
| Drop old columns D                                            | Task 2                                      |
| Frontend verification + screenshots                           | Task 10                                     |
| `docs/development/coding-standards.md` update                 | Task 11                                     |

### No placeholder scan

All steps contain complete code. No "TBD", "TODO", or "similar to above" entries.

### Type consistency

- `ScalingMode` defined in `scaling_store.rs`, imported everywhere as `crate::scaling_store::ScalingMode`
- `ScalingPolicy` defined in `scaling_store.rs`
- `ScalingRecord.scaling_mode_used` field type is `crate::scaling_store::ScalingMode` in the struct and `String` in the entity — consistent with the
  pattern used for all other enum fields
- `ACTION_PRELOAD_SCALING_GLOBAL_DEFAULTS` constant name used in surfaces.rs (definition) and plugin.rs (InteractionDescriptor) consistently
- `handle_preload_scaling_global_defaults` / `handle_save_scaling_global_defaults` / `handle_preload_scaling_item_overrides` /
  `handle_save_scaling_item_overrides` — function names match arm labels in `execute_controller_surface_action_typed`
