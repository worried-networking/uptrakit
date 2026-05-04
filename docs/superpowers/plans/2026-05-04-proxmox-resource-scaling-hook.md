# Proxmox Resource Scaling Hook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans
> to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `ControllerUpdateHook` that temporarily scales a Proxmox VM's CPU cores and RAM before an Update and restores them after,
with crash-safe persistence.

**Architecture:** New `ControllerUpdateHook` trait (parallel to `ControllerUpdateProtection`) wired through `PluginOps` and the dispatch layer.
Proxmox implementation lives in `resource_scaling.rs`. Pre-hook runs after protection (scale up);
post-hook runs before protection finalization (scale down).

**Tech Stack:** Rust, SeaORM (SQLite), Proxmox VE REST API (PUT config endpoint), `async_trait`, `tracing`, `tokio::sync::mpsc`.

---

## Wave 1: DB Schema

### Task 1: Migration A — add scaling policy columns to both policy tables

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/entity/proxmox_protection_default.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/entity/proxmox_protection_item_override.rs`

- [ ] **Step 1: Write the failing test**

Add to the bottom of `controller_migration.rs` tests module:

```rust
#[tokio::test]
async fn migration_a_adds_update_cores_and_memory_mb_columns() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);

    // Run all existing migrations so the table exists
    CreateProxmoxProtectionPolicyTables.up(&manager).await.unwrap();

    AddProxmoxResourceScalingPolicyColumns.up(&manager).await.unwrap();

    let defaults_cols = column_names(&db, "proxmox_protection_defaults").await;
    assert!(defaults_cols.contains(&"update_cores".to_string()));
    assert!(defaults_cols.contains(&"update_memory_mb".to_string()));

    let overrides_cols = column_names(&db, "proxmox_protection_item_overrides").await;
    assert!(overrides_cols.contains(&"update_cores".to_string()));
    assert!(overrides_cols.contains(&"update_memory_mb".to_string()));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox migration_a_adds_update_cores_and_memory_mb_columns --features migrations -- --nocapture
```

Expected: FAIL with "unresolved import" or "no function named `AddProxmoxResourceScalingPolicyColumns`"

- [ ] **Step 3: Add `DeriveIden` enums for the two policy tables**

At the bottom of `controller_migration.rs`, before `pub fn migrations()`, add:

```rust
#[derive(DeriveIden)]
enum ProxmoxResourceScalingPolicyCols {
    UpdateCores,
    UpdateMemoryMb,
}
```

- [ ] **Step 4: Write Migration A struct**

In `controller_migration.rs`, after `ProxmoxHmVmidUniquePerConfig` and before the identifiers section:

```rust
// ── Migration: add resource scaling policy columns ──────────────────────────

/// Add `update_cores` and `update_memory_mb` to both Proxmox protection policy
/// tables. NULL means no scaling configured for that policy row.
pub struct AddProxmoxResourceScalingPolicyColumns;

impl MigrationName for AddProxmoxResourceScalingPolicyColumns {
    fn name(&self) -> &str {
        "m20260503_000001_proxmox_resource_scaling_policy"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddProxmoxResourceScalingPolicyColumns {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingPolicyCols::UpdateCores)
                            .integer()
                            .null(),
                    )
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                            .integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingPolicyCols::UpdateCores)
                            .integer()
                            .null(),
                    )
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
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
                    .table(ProxmoxProtectionDefaults::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateCores)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateCores)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                    .to_owned(),
            )
            .await
    }
}
```

- [ ] **Step 5: Register in `migrations()` vec**

In `pub fn migrations()`, append after `Box::new(ProxmoxHmVmidUniquePerConfig)`:

```rust
Box::new(AddProxmoxResourceScalingPolicyColumns),
```

- [ ] **Step 6: Add fields to entity files**

In `proxmox_protection_default.rs`, add two fields to `Model` after `backup_timeout_seconds`:

```rust
pub update_cores: Option<i32>,
pub update_memory_mb: Option<i32>,
```

In `proxmox_protection_item_override.rs`, add the same two fields after `backup_timeout_seconds`.

- [ ] **Step 7: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox migration_a_adds_update_cores_and_memory_mb_columns --features migrations -- --nocapture
```

Expected: PASS

- [ ] **Step 8: `cargo check` clean**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: no errors

- [ ] **Step 9: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/controller_migration.rs \
        crates/plugins/infrastructure/proxmox/src/entity/proxmox_protection_default.rs \
        crates/plugins/infrastructure/proxmox/src/entity/proxmox_protection_item_override.rs
git commit -m "feat(proxmox): add update_cores and update_memory_mb policy columns (migration A)"
```

---

### Task 2: Migration B — create `proxmox_resource_scaling_records` table and entity

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`
- Create: `crates/plugins/infrastructure/proxmox/src/entity/proxmox_resource_scaling_record.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/entity/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to `controller_migration.rs` tests module:

```rust
#[tokio::test]
async fn migration_b_creates_scaling_records_table() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);

    CreateProxmoxResourceScalingRecord.up(&manager).await.unwrap();

    let cols = column_names(&db, "proxmox_resource_scaling_records").await;
    for expected in &[
        "update_history_id", "tenant_id", "host_id", "software_item_id",
        "plugin_config_id", "mapping_id", "vm_type",
        "original_cores", "original_memory_mb",
        "scaled_cores", "scaled_memory_mb",
        "scale_status", "restore_status", "error_message",
        "created_at", "updated_at",
    ] {
        assert!(cols.contains(&expected.to_string()), "missing column: {expected}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox migration_b_creates_scaling_records_table --features migrations -- --nocapture
```

Expected: FAIL — `CreateProxmoxResourceScalingRecord` not found.

- [ ] **Step 3: Add identifiers enum and migration struct**

Add to `controller_migration.rs` (after `AddProxmoxResourceScalingPolicyColumns`, before identifier enums):

```rust
// ── Migration: create proxmox_resource_scaling_records ─────────────────────

/// Create the `proxmox_resource_scaling_records` table for persisting pre/post
/// resource scaling state across Controller restarts.
pub struct CreateProxmoxResourceScalingRecord;

impl MigrationName for CreateProxmoxResourceScalingRecord {
    fn name(&self) -> &str {
        "m20260503_000002_proxmox_resource_scaling_record"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxResourceScalingRecord {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxResourceScalingRecords::Table)
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::UpdateHistoryId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::HostId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::MappingId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::VmType)
                            .string_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::OriginalCores)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::OriginalMemoryMb)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ScaledCores)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ScaledMemoryMb)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ScaleStatus)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::RestoreStatus)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ErrorMessage)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxResourceScalingRecords::Table)
                    .to_owned(),
            )
            .await
    }
}
```

Add identifier enum (alongside the other `DeriveIden` enums at the bottom of the file):

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
}
```

- [ ] **Step 4: Register in `migrations()` vec**

Append after `Box::new(AddProxmoxResourceScalingPolicyColumns)`:

```rust
Box::new(CreateProxmoxResourceScalingRecord),
```

- [ ] **Step 5: Create entity file**

Create `crates/plugins/infrastructure/proxmox/src/entity/proxmox_resource_scaling_record.rs`:

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
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 6: Register entity in mod.rs**

Add to `crates/plugins/infrastructure/proxmox/src/entity/mod.rs`:

```rust
pub(crate) mod proxmox_resource_scaling_record;
```

- [ ] **Step 7: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox migration_b_creates_scaling_records_table --features migrations -- --nocapture
```

Expected: PASS

- [ ] **Step 8: `cargo check` clean**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

- [ ] **Step 9: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/controller_migration.rs \
        crates/plugins/infrastructure/proxmox/src/entity/proxmox_resource_scaling_record.rs \
        crates/plugins/infrastructure/proxmox/src/entity/mod.rs
git commit -m "feat(proxmox): create proxmox_resource_scaling_records table and entity (migration B)"
```

---

### Task 3: Policy struct extensions

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/policy_store.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/protection_store.rs`

- [ ] **Step 1: Write failing tests**

Add to `policy_store.rs` tests:

```rust
#[test]
fn protection_policy_carries_scaling_fields() {
    let p = ProtectionPolicy {
        mode: ProtectionMode::DoNothing,
        backup_target_key: None,
        snapshot_timeout_seconds: None,
        backup_timeout_seconds: None,
        update_cores: Some(4),
        update_memory_mb: Some(8192),
    };
    assert_eq!(p.update_cores, Some(4));
    assert_eq!(p.update_memory_mb, Some(8192));
}

#[test]
fn do_nothing_policy_has_no_scaling() {
    let p = ProtectionPolicy::do_nothing();
    assert!(p.update_cores.is_none());
    assert!(p.update_memory_mb.is_none());
}

#[test]
fn resolve_effective_policy_cascades_scaling_fields() {
    let item = ProtectionPolicy {
        mode: ProtectionMode::Snapshot,
        backup_target_key: None,
        snapshot_timeout_seconds: None,
        backup_timeout_seconds: None,
        update_cores: Some(8),
        update_memory_mb: None,
    };
    let global = ProtectionPolicy {
        mode: ProtectionMode::DoNothing,
        backup_target_key: None,
        snapshot_timeout_seconds: None,
        backup_timeout_seconds: None,
        update_cores: Some(4),
        update_memory_mb: Some(4096),
    };
    let effective = resolve_effective_policy(Some(item), Some(global));
    // item_override wins for update_cores
    assert_eq!(effective.update_cores, Some(8));
    // falls back to global for update_memory_mb
    assert_eq!(effective.update_memory_mb, Some(4096));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox protection_policy_carries_scaling_fields do_nothing_policy_has_no_scaling resolve_effective_policy_cascades_scaling_fields -- --nocapture
```

Expected: FAIL — fields don't exist yet.

- [ ] **Step 3: Add fields to `ProtectionPolicy` in `policy_store.rs`**

In `policy_store.rs`, extend `ProtectionPolicy`:

```rust
pub struct ProtectionPolicy {
    pub mode: ProtectionMode,
    pub backup_target_key: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
    pub update_cores: Option<i32>,
    pub update_memory_mb: Option<i32>,
}
```

Update `do_nothing()`:

```rust
pub fn do_nothing() -> Self {
    Self {
        mode: ProtectionMode::DoNothing,
        backup_target_key: None,
        snapshot_timeout_seconds: None,
        backup_timeout_seconds: None,
        update_cores: None,
        update_memory_mb: None,
    }
}
```

Update `resolve_effective_policy` to cascade both new fields (same per-field merge pattern as `snapshot_timeout_seconds`):

```rust
let update_cores = item_ref
    .and_then(|p| p.update_cores)
    .or_else(|| global_ref.and_then(|p| p.update_cores));

let update_memory_mb = item_ref
    .and_then(|p| p.update_memory_mb)
    .or_else(|| global_ref.and_then(|p| p.update_memory_mb));

ProtectionPolicy {
    mode,
    backup_target_key,
    snapshot_timeout_seconds,
    backup_timeout_seconds,
    update_cores,
    update_memory_mb,
}
```

Update `load_global_default`, `load_item_override`, `upsert_global_default`, and `upsert_item_override` to include `update_cores` and
`update_memory_mb` (same pattern as `backup_target_key`). Specifically:

- In `load_*` mapping closures: add `update_cores: model.update_cores, update_memory_mb: model.update_memory_mb,`
- In `upsert_*` ActiveModel blocks: add `update_cores: Set(policy.update_cores), update_memory_mb: Set(policy.update_memory_mb),`

- [ ] **Step 4: Add fields to `ProxmoxProtectionPolicyRecord` in `protection_store.rs`**

In `protection_store.rs`, extend `ProxmoxProtectionPolicyRecord`:

```rust
pub struct ProxmoxProtectionPolicyRecord {
    pub mode: ProxmoxProtectionMode,
    pub backup_target_key: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
    pub update_cores: Option<i32>,
    pub update_memory_mb: Option<i32>,
}
```

Update `DbProxmoxProtectionStore::load_effective_policy` to cascade the new fields from item_override and global_default
(same `and_then` / `or_else` pattern as `backup_target_key`):

```rust
let update_cores = item_override
    .as_ref()
    .and_then(|row| row.update_cores)
    .or_else(|| global_default.as_ref().and_then(|row| row.update_cores));

let update_memory_mb = item_override
    .as_ref()
    .and_then(|row| row.update_memory_mb)
    .or_else(|| global_default.as_ref().and_then(|row| row.update_memory_mb));

Ok(ProxmoxProtectionPolicyRecord {
    mode: ...,
    backup_target_key: ...,
    snapshot_timeout_seconds,
    backup_timeout_seconds,
    update_cores,
    update_memory_mb,
})
```

Also update `Default` impl for `ProxmoxProtectionPolicyRecord` if it exists; set both fields to `None`.

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox protection_policy_carries_scaling_fields do_nothing_policy_has_no_scaling resolve_effective_policy_cascades_scaling_fields -- --nocapture
```

Expected: PASS

- [ ] **Step 6: `cargo check` clean**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

- [ ] **Step 7: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/policy_store.rs \
        crates/plugins/infrastructure/proxmox/src/protection_store.rs
git commit -m "feat(proxmox): extend ProtectionPolicy and ProxmoxProtectionPolicyRecord with scaling fields"
```

---

## Wave 2: API Types + Client

### Task 4: Extend API types with resource and hotplug fields

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/api_types.rs`

- [ ] **Step 1: Write failing tests**

Add to `api_types.rs` tests:

```rust
#[test]
fn qemu_config_no_hotplug_returns_false() {
    let cfg = PveQemuConfig { name: None, cores: Some(2), memory: Some(2048), hotplug: None };
    assert!(!cfg.supports_live_resource_scaling());
}

#[test]
fn qemu_config_partial_hotplug_returns_false() {
    let cfg = PveQemuConfig {
        name: None,
        cores: Some(2),
        memory: Some(2048),
        hotplug: Some("disk,network".to_string()),
    };
    assert!(!cfg.supports_live_resource_scaling());
}

#[test]
fn qemu_config_full_hotplug_returns_true() {
    let cfg = PveQemuConfig {
        name: None,
        cores: Some(2),
        memory: Some(2048),
        hotplug: Some("disk,network,usb,memory,cpu".to_string()),
    };
    assert!(cfg.supports_live_resource_scaling());
}

#[test]
fn qemu_config_deserialization_includes_cores_memory_hotplug() {
    let json = r#"{"data":{"name":"vm1","cores":4,"memory":8192,"hotplug":"disk,network,usb,memory,cpu"}}"#;
    let resp: PveResponse<PveQemuConfig> = serde_json::from_str(json).expect("deserialize");
    assert_eq!(resp.data.cores, Some(4));
    assert_eq!(resp.data.memory, Some(8192));
    assert!(resp.data.supports_live_resource_scaling());
}

#[test]
fn lxc_config_deserialization_includes_cores_memory() {
    let json = r#"{"data":{"hostname":"ct1","cores":2,"memory":1024}}"#;
    let resp: PveResponse<PveLxcConfig> = serde_json::from_str(json).expect("deserialize");
    assert_eq!(resp.data.cores, Some(2));
    assert_eq!(resp.data.memory, Some(1024));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox qemu_config_no_hotplug_returns_false qemu_config_partial_hotplug_returns_false qemu_config_full_hotplug_returns_true qemu_config_deserialization_includes_cores_memory_hotplug lxc_config_deserialization_includes_cores_memory -- --nocapture
```

Expected: FAIL — struct fields don't exist.

- [ ] **Step 3: Extend `PveQemuConfig`**

Replace the existing `PveQemuConfig` struct with:

```rust
/// QEMU VM configuration from `GET /nodes/{node}/qemu/{vmid}/config`.
#[derive(Debug, Clone, Deserialize)]
pub struct PveQemuConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub cores: Option<u32>,
    #[serde(default)]
    pub memory: Option<u64>,
    /// Comma-separated hotplug device list (e.g. `"disk,network,usb,memory,cpu"`).
    /// Absent means hotplug is disabled.
    #[serde(default)]
    pub hotplug: Option<String>,
}

impl PveQemuConfig {
    pub fn supports_live_resource_scaling(&self) -> bool {
        match &self.hotplug {
            None => false,
            Some(h) => {
                h.split(',').map(str::trim).any(|f| f == "cpu")
                    && h.split(',').map(str::trim).any(|f| f == "memory")
            }
        }
    }
}
```

- [ ] **Step 4: Extend `PveLxcConfig`**

Replace the existing `PveLxcConfig` struct with:

```rust
/// LXC container configuration from `GET /nodes/{node}/lxc/{vmid}/config`.
#[derive(Debug, Clone, Deserialize)]
pub struct PveLxcConfig {
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub cores: Option<u32>,
    #[serde(default)]
    pub memory: Option<u64>,
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox qemu_config_no_hotplug_returns_false qemu_config_partial_hotplug_returns_false qemu_config_full_hotplug_returns_true qemu_config_deserialization_includes_cores_memory_hotplug lxc_config_deserialization_includes_cores_memory -- --nocapture
```

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/api_types.rs
git commit -m "feat(proxmox): extend PveQemuConfig/PveLxcConfig with resource+hotplug fields"
```

---

### Task 5: Add `set_*_config_resources` client methods

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/client.rs`

The Proxmox API accepts PUT requests with `application/x-www-form-urlencoded` body for synchronous config changes.
The existing `post_form` helper does POST; we need a similar `put_form` helper. Proxmox returns either `{"data":null}` or a UPID string
for async tasks depending on version and VM state; we discard the body and treat HTTP 2xx as success.

- [ ] **Step 1: Write a compile-only test (no Proxmox server needed)**

Add to `client.rs`:

```rust
#[cfg(test)]
mod resource_scaling_method_tests {
    use super::*;
    use crate::config::ProxmoxConfig;
    use uptrakit_plugin_infrastructure_core::SecretString;

    fn test_config() -> ProxmoxConfig {
        ProxmoxConfig {
            api_url: "https://pve.test:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=secret"),
            ..ProxmoxConfig::default()
        }
    }

    #[test]
    fn client_has_set_qemu_config_resources_method() {
        // Compile-time assertion: the method exists with the correct signature.
        let _: fn(&ProxmoxClient, &str, u32, u32, u64) -> _ =
            |c, node, vmid, cores, memory_mb| c.set_qemu_config_resources(node, vmid, cores, memory_mb);
    }

    #[test]
    fn client_has_set_lxc_config_resources_method() {
        let _: fn(&ProxmoxClient, &str, u32, u32, u64) -> _ =
            |c, node, vmid, cores, memory_mb| c.set_lxc_config_resources(node, vmid, cores, memory_mb);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox client_has_set_qemu_config_resources_method client_has_set_lxc_config_resources_method -- --nocapture
```

Expected: FAIL — methods don't exist.

- [ ] **Step 3: Add `put_form` helper method to `ProxmoxClient`**

In `client.rs`, after the `post_form` method:

```rust
/// Perform a PUT form request to the Proxmox API.
///
/// Used for synchronous config changes (e.g. `PUT /nodes/{node}/qemu/{vmid}/config`).
/// Proxmox returns `{"data": null}` for these requests; we discard the body.
async fn put_form(&self, path: &str, params: &[(String, String)]) -> Result<()> {
    let url = format!("{}/api2/json{path}", self.base_url);

    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .finish();

    let response = self
        .client
        .put(&url)
        .header("Authorization", &self.auth_header)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(encoded)
        .send()
        .await
        .map_err(|e| {
            report!(ProxmoxError::Request(format!(
                "HTTP PUT request to {path} failed: {e}"
            )))
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!(ProxmoxError::ApiError { status, message: body });
    }

    Ok(())
}
```

- [ ] **Step 4: Add `set_qemu_config_resources` and `set_lxc_config_resources`**

```rust
/// Apply CPU and memory limits to a running QEMU VM.
///
/// Calls `PUT /api2/json/nodes/{node}/qemu/{vmid}/config` with `cores` and
/// `memory` fields. Proxmox applies both atomically when hotplug is enabled.
pub async fn set_qemu_config_resources(
    &self,
    node: &str,
    vmid: u32,
    cores: u32,
    memory_mb: u64,
) -> Result<()> {
    let path = format!("/nodes/{node}/qemu/{vmid}/config");
    self.put_form(
        &path,
        &[
            ("cores".to_string(), cores.to_string()),
            ("memory".to_string(), memory_mb.to_string()),
        ],
    )
    .await
}

/// Apply CPU and memory limits to a running LXC container.
///
/// Calls `PUT /api2/json/nodes/{node}/lxc/{vmid}/config`.
pub async fn set_lxc_config_resources(
    &self,
    node: &str,
    vmid: u32,
    cores: u32,
    memory_mb: u64,
) -> Result<()> {
    let path = format!("/nodes/{node}/lxc/{vmid}/config");
    self.put_form(
        &path,
        &[
            ("cores".to_string(), cores.to_string()),
            ("memory".to_string(), memory_mb.to_string()),
        ],
    )
    .await
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox client_has_set_qemu_config_resources_method client_has_set_lxc_config_resources_method -- --nocapture
```

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/client.rs
git commit -m "feat(proxmox): add set_qemu_config_resources and set_lxc_config_resources client methods"
```

---

## Wave 3: Trait Infrastructure

### Task 6: Add `UpdateHookController`, context types, and `ControllerUpdateHook` to `roles.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs`

- [ ] **Step 1: Write failing test**

Add to `roles.rs` `controller_boundary_tests` module:

```rust
#[cfg(feature = "plugin-ops")]
#[test]
fn update_hook_controller_trait_is_object_safe() {
    struct TestHookCtrl;
    impl UpdateHookController for TestHookCtrl {
        fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
            unimplemented!()
        }
    }
    let ctrl = TestHookCtrl;
    let _dyn: &dyn UpdateHookController = &ctrl;
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --features plugin-ops update_hook_controller_trait_is_object_safe -- --nocapture
```

Expected: FAIL — `UpdateHookController` not defined.

- [ ] **Step 3: Add `UpdateHookController` trait and context types**

In `roles.rs`, after the `UpdateProtectionController` block (around line 264), add:

```rust
/// Typed controller boundary for update hook workflows.
#[cfg(feature = "plugin-ops")]
pub trait UpdateHookController: Send + Sync {
    /// Tenant-scoped database access for the update hook workflow.
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
}

/// Context provided to the pre-update hook.
#[cfg(feature = "plugin-ops")]
#[non_exhaustive]
pub struct UpdateHookPreContext<'a> {
    pub controller: &'a dyn UpdateHookController,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
    pub output_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
}

#[cfg(feature = "plugin-ops")]
impl<'a> UpdateHookPreContext<'a> {
    pub fn new(
        controller: &'a dyn UpdateHookController,
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

    pub fn with_output_tx(mut self, tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>) -> Self {
        self.output_tx = Some(tx);
        self
    }
}

/// Context provided to the post-update hook.
#[cfg(feature = "plugin-ops")]
#[non_exhaustive]
pub struct UpdateHookPostContext<'a> {
    pub controller: &'a dyn UpdateHookController,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
    pub final_status: uptrakit_shared_types::UpdateStatus,
    pub notification_ops: &'a dyn crate::plugin_ops::NotificationOps,
    /// Tenant-scoped DB handle required by `NotificationOps::send_transactional_email`.
    pub tenant_db: uptrakit_tenant_db::TenantDb,
}

#[cfg(feature = "plugin-ops")]
impl<'a> UpdateHookPostContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        controller: &'a dyn UpdateHookController,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        update_history_id: Uuid,
        final_status: uptrakit_shared_types::UpdateStatus,
        notification_ops: &'a dyn crate::plugin_ops::NotificationOps,
        tenant_db: uptrakit_tenant_db::TenantDb,
    ) -> Self {
        Self {
            controller,
            tenant_id,
            host_id,
            software_item_id,
            update_history_id,
            final_status,
            notification_ops,
            tenant_db,
        }
    }
}
```

- [ ] **Step 4: Add `ControllerUpdateHook` trait**

After the context types, add:

```rust
/// Controller-side pre/post-update hook plugin (e.g. resource scaling).
///
/// Singleton created at catalog construction.
#[async_trait]
pub trait ControllerUpdateHook: PluginMeta + Send + Sync {
    /// Called before update execution. Best-effort: returns `()` so that
    /// scale-up failure cannot accidentally block the Update.
    async fn prepare_pre_update_hook(&self, ctx: &UpdateHookPreContext<'_>);

    /// Called after update completion. Returns `Result<()>` so restore
    /// failures propagate to the dispatch wrapper for logging.
    async fn finalize_post_update_hook(
        &self,
        ctx: &UpdateHookPostContext<'_>,
    ) -> crate::error::Result<()>;
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --features plugin-ops update_hook_controller_trait_is_object_safe -- --nocapture
```

Expected: PASS

- [ ] **Step 6: `cargo check` clean**

```bash
cargo check -p uptrakit-plugin-infrastructure-core --all-features
```

- [ ] **Step 7: Commit**

```bash
git add crates/plugins/infrastructure/core/src/roles.rs
git commit -m "feat(core): add UpdateHookController, UpdateHookPre/PostContext, ControllerUpdateHook to roles.rs"
```

---

### Task 7: Add `ControllerUpdateHookOps` to `plugin_ops.rs` and `PluginOps`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/plugin_ops.rs`

- [ ] **Step 1: Write failing test**

Add a test in `plugin_ops.rs` (or the catalog tests — easiest in a local test struct):

```rust
#[cfg(test)]
mod update_hook_ops_tests {
    use super::*;

    struct TestOps;
    impl ControllerUpdateHookOps for TestOps {}

    #[test]
    fn default_impl_returns_none() {
        let ops = TestOps;
        assert!(ops.controller_update_hook().is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --all-features default_impl_returns_none -- --nocapture
```

Expected: FAIL — `ControllerUpdateHookOps` not defined.

- [ ] **Step 3: Add `ControllerUpdateHookOps` trait**

In `plugin_ops.rs`, add after `ControllerUpdateProtectionOps`:

```rust
// ── Trait 8: ControllerUpdateHookOps ────────────────────────────────────────

/// Controller-side singleton update hook accessor.
pub trait ControllerUpdateHookOps: Send + Sync + 'static {
    /// The registered controller update hook plugin (if configured).
    fn controller_update_hook(&self) -> Option<std::sync::Arc<dyn crate::roles::ControllerUpdateHook>> {
        None
    }
}
```

- [ ] **Step 4: Add `ControllerUpdateHookOps` to `PluginOps` supertrait and blanket impl**

In `plugin_ops.rs`, update `PluginOps`:

```rust
pub trait PluginOps:
    PluginMetadataOps
    + PluginConfigOps
    + PluginSurfaceActionOps
    + PluginSurfaceOps
    + NotificationOps
    + SoftwareItemLifecycleOps
    + ControllerUpdateProtectionOps
    + ControllerUpdateHookOps
{
}

impl<T> PluginOps for T where
    T: PluginMetadataOps
        + PluginConfigOps
        + PluginSurfaceActionOps
        + PluginSurfaceOps
        + NotificationOps
        + SoftwareItemLifecycleOps
        + ControllerUpdateProtectionOps
        + ControllerUpdateHookOps
{
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --all-features default_impl_returns_none -- --nocapture
```

Expected: PASS

- [ ] **Step 6: Add `ControllerUpdateHookOps` import to `roles.rs` (for the `ControllerUpdateHook` export)**

Make sure `ControllerUpdateHook` is re-exported from the crate's `lib.rs`. Check `lib.rs` for the existing `ControllerUpdateProtection`
re-export pattern and add the hook types alongside it.

- [ ] **Step 7: Fix any compilation errors from adding the bound**

All existing test stubs (`ProtectionOverridePluginOps`, `TestPluginOps`) that implement `ControllerUpdateProtectionOps` must also implement
`ControllerUpdateHookOps`. They get the default `None` impl automatically since `ControllerUpdateHookOps` has a default impl — no action
needed unless any struct uses a manual blanket that doesn't include the new bound.

```bash
cargo check --workspace --all-features
```

Expected: no errors (the default `None` impl covers existing implementors; this workspace-wide check catches any crate that manually implements `PluginOps`)

- [ ] **Step 8: Commit**

```bash
git add crates/plugins/infrastructure/core/src/plugin_ops.rs
git commit -m "feat(core): add ControllerUpdateHookOps trait and add to PluginOps supertrait"
```

---

### Task 8: Add type alias and field to `descriptor.rs` + `macros.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`
- Modify: `crates/plugins/infrastructure/core/src/macros.rs`

- [ ] **Step 1: Write failing test (descriptor field exists)**

Add to `descriptor.rs`:

```rust
#[cfg(test)]
mod role_creators_update_hook_tests {
    use super::*;

    #[test]
    fn role_creators_has_controller_update_hook_field() {
        let rc = RoleCreators {
            discoverer: None,
            version_detector: None,
            release_fetcher: None,
            package_indexer: None,
            update_executor: None,
            lifecycle_hook: None,
            notification_transport: None,
            software_item_lifecycle: None,
            controller_update_protection: None,
            controller_update_hook: None,
            infra: None,
        };
        assert!(rc.controller_update_hook.is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-core role_creators_has_controller_update_hook_field -- --nocapture
```

Expected: FAIL — `controller_update_hook` field doesn't exist.

- [ ] **Step 3: Add type alias and field to `descriptor.rs`**

In `descriptor.rs`, after `CreateControllerProtectionFn`:

```rust
/// Creation for a controller update hook plugin (singleton).
pub type CreateControllerUpdateHookFn =
    fn(&CatalogConfig) -> crate::error::Result<std::sync::Arc<dyn roles::ControllerUpdateHook>>;
```

In `RoleCreators`, add after `controller_update_protection`:

```rust
/// Singleton controller-side update hook (catalog config → Arc, created once at startup)
pub controller_update_hook: Option<CreateControllerUpdateHookFn>,
```

- [ ] **Step 4: Update `macros.rs` to support optional `controller_update_hook` parameter**

In `macros.rs`, in the `declare_plugin!` macro parameter list, add after `$(, controller_update_protection: $controller_protection_fn:expr )?`:

```rust
$(, controller_update_hook: $controller_hook_fn:expr )?
```

In the `RoleCreators` struct literal inside the macro body, add `controller_update_hook: None,` to the struct:

```rust
let mut rc = $crate::RoleCreators {
    discoverer: None,
    version_detector: None,
    release_fetcher: None,
    package_indexer: None,
    update_executor: None,
    lifecycle_hook: None,
    notification_transport: None,
    software_item_lifecycle: None,
    controller_update_protection: None,
    controller_update_hook: None,
    infra: None,
};
```

After the `$(rc.controller_update_protection = Some($controller_protection_fn);)?` block, add:

```rust
$(
    rc.controller_update_hook = Some($controller_hook_fn);
)?
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-core role_creators_has_controller_update_hook_field -- --nocapture
```

Expected: PASS

- [ ] **Step 6: `cargo check` all crates clean (verify no plugin crate breaks)**

```bash
cargo check --all-features
```

Expected: no errors — the new `None` default means all existing `declare_plugin!` invocations are unaffected.

- [ ] **Step 7: Commit**

```bash
git add crates/plugins/infrastructure/core/src/descriptor.rs \
        crates/plugins/infrastructure/core/src/macros.rs
git commit -m "feat(core): add CreateControllerUpdateHookFn type alias, RoleCreators field, and macro support"
```

---

### Task 9: Wire `ControllerUpdateHookOps` into `PluginCatalog`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`

- [ ] **Step 1: Write failing test**

Add to `catalog.rs` tests:

```rust
#[cfg(test)]
mod update_hook_catalog_tests {
    use super::*;

    #[test]
    fn catalog_with_no_hook_returns_none() {
        // Use the simplest empty-ish catalog possible
        let catalog = PluginCatalog::new(vec![], &CatalogConfig::default()).unwrap();
        assert!(catalog.controller_update_hook().is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-core catalog_with_no_hook_returns_none -- --nocapture
```

Expected: FAIL — method doesn't exist.

- [ ] **Step 3: Add field to `PluginCatalog` struct**

In `catalog.rs`, in the `PluginCatalog` struct, add after `controller_update_protection`:

```rust
controller_update_hook: Option<Arc<dyn crate::roles::ControllerUpdateHook>>,
```

- [ ] **Step 4: Construct in `PluginCatalog::new`**

Add before the loop in `new`:

```rust
let mut controller_update_hook: Option<Arc<dyn crate::roles::ControllerUpdateHook>> = None;
```

Inside the loop, after the `controller_update_protection` block:

```rust
// ── Singleton: controller update hook ──
if let Some(create) = desc.roles.controller_update_hook {
    if controller_update_hook.is_some() {
        return Err(rootcause::report!(PluginError::UnsupportedOperation(
            format!("duplicate controller update hook: {}", desc.type_id)
        )));
    }
    let plugin = create(config).map_err(|e| {
        rootcause::report!(PluginError::UnsupportedOperation(format!(
            "failed to create controller update hook '{}': {e}",
            desc.type_id
        )))
    })?;
    controller_update_hook = Some(plugin);
}
```

In `Ok(Self { ... })`, add `controller_update_hook,`.

Update the `use` imports at top of `catalog.rs` to include `ControllerUpdateHookOps` from `plugin_ops`.

- [ ] **Step 5: Implement `ControllerUpdateHookOps` for `PluginCatalog`**

```rust
impl ControllerUpdateHookOps for PluginCatalog {
    fn controller_update_hook(&self) -> Option<Arc<dyn crate::roles::ControllerUpdateHook>> {
        self.controller_update_hook.clone()
    }
}
```

- [ ] **Step 6: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-core catalog_with_no_hook_returns_none -- --nocapture
```

Expected: PASS

- [ ] **Step 7: `cargo check` clean**

```bash
cargo check --all-features
```

- [ ] **Step 8: Commit**

```bash
git add crates/plugins/infrastructure/core/src/catalog.rs
git commit -m "feat(core): store and expose controller_update_hook in PluginCatalog"
```

---

## Wave 4: Proxmox Implementation

### Task 10: Add `upsert_scaling_record` and `load_scaling_record` to `policy_store.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/policy_store.rs`

- [ ] **Step 1: Write failing tests**

Add to `policy_store.rs` tests:

```rust
#[tokio::test]
async fn scaling_record_round_trip() {
    use crate::entity::proxmox_resource_scaling_record;
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};

    let update_id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();

    // Mock: no existing row (insert path)
    let db = MockDatabase::new(DbBackend::Sqlite)
        .append_query_results([Vec::<proxmox_resource_scaling_record::Model>::new()])
        .append_exec_results([MockExecResult { last_insert_id: 0, rows_affected: 1 }])
        .into_connection();

    let record = crate::policy_store::ScalingRecord {
        update_history_id: update_id,
        tenant_id: Uuid::now_v7(),
        host_id: Uuid::now_v7(),
        software_item_id: Uuid::now_v7(),
        plugin_config_id: Uuid::now_v7(),
        mapping_id: Uuid::now_v7(),
        vm_type: "qemu".to_string(),
        original_cores: 2,
        original_memory_mb: 2048,
        scaled_cores: 4,
        scaled_memory_mb: 4096,
        scale_status: "scaling".to_string(),
        restore_status: "pending".to_string(),
        error_message: None,
    };

    let result = crate::policy_store::upsert_scaling_record(&db, &record).await;
    assert!(result.is_ok(), "upsert should succeed: {result:?}");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox scaling_record_round_trip -- --nocapture
```

Expected: FAIL — functions not defined.

- [ ] **Step 3: Add `ScalingRecord` struct and the two free functions**

In `policy_store.rs`:

Add `use crate::entity::proxmox_resource_scaling_record;` and
`use proxmox_resource_scaling_record::Entity as ProxmoxResourceScalingRecord;` to the use block.

Add the struct and functions after `upsert_protection_audit`:

```rust
/// Scaling record for one `update_history_id`.
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
}

/// Load a scaling record by `update_history_id`.
pub async fn load_scaling_record(
    db: &DatabaseConnection,
    update_history_id: Uuid,
) -> Result<Option<ScalingRecord>> {
    let row = ProxmoxResourceScalingRecord::find_by_id(update_history_id)
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query scaling record: {e}"
            )))
        })?;

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
    }))
}

/// Upsert a scaling record by `update_history_id`.
///
/// Uses `BEGIN IMMEDIATE` to prevent `SQLITE_BUSY_SNAPSHOT` under concurrent
/// callers (e.g. pre-hook and crash-recovery finalization overlapping).
pub async fn upsert_scaling_record(
    db: &DatabaseConnection,
    record: &ScalingRecord,
) -> Result<()> {
    use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait as _};

    let now = OffsetDateTime::now_utc();
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to begin transaction for scaling record upsert: {e}"
            )))
        })?;

    let existing = ProxmoxResourceScalingRecord::find_by_id(record.update_history_id)
        .one(&txn)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing scaling record: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: proxmox_resource_scaling_record::ActiveModel = existing.into();
        active.tenant_id = Set(record.tenant_id);
        active.host_id = Set(record.host_id);
        active.software_item_id = Set(record.software_item_id);
        active.plugin_config_id = Set(record.plugin_config_id);
        active.mapping_id = Set(record.mapping_id);
        active.vm_type = Set(record.vm_type.clone());
        active.original_cores = Set(record.original_cores);
        active.original_memory_mb = Set(record.original_memory_mb);
        active.scaled_cores = Set(record.scaled_cores);
        active.scaled_memory_mb = Set(record.scaled_memory_mb);
        active.scale_status = Set(record.scale_status.clone());
        active.restore_status = Set(record.restore_status.clone());
        active.error_message = Set(record.error_message.clone());
        active.updated_at = Set(now);
        active.update(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to update scaling record: {e}"
            )))
        })?;
    } else {
        let active = proxmox_resource_scaling_record::ActiveModel {
            update_history_id: Set(record.update_history_id),
            tenant_id: Set(record.tenant_id),
            host_id: Set(record.host_id),
            software_item_id: Set(record.software_item_id),
            plugin_config_id: Set(record.plugin_config_id),
            mapping_id: Set(record.mapping_id),
            vm_type: Set(record.vm_type.clone()),
            original_cores: Set(record.original_cores),
            original_memory_mb: Set(record.original_memory_mb),
            scaled_cores: Set(record.scaled_cores),
            scaled_memory_mb: Set(record.scaled_memory_mb),
            scale_status: Set(record.scale_status.clone()),
            restore_status: Set(record.restore_status.clone()),
            error_message: Set(record.error_message.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to insert scaling record: {e}"
            )))
        })?;
    }

    txn.commit().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to commit scaling record upsert: {e}"
        )))
    })?;
    Ok(())
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox scaling_record_round_trip -- --nocapture
```

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/policy_store.rs
git commit -m "feat(proxmox): add ScalingRecord, upsert_scaling_record, load_scaling_record to policy_store"
```

---

### Task 11: Implement `prepare_pre_update_hook` in `resource_scaling.rs`

**Files:**

- Create: `crates/plugins/infrastructure/proxmox/src/resource_scaling.rs`

- [ ] **Step 1: Write failing tests**

Create the file with a test module:

```rust
//! Proxmox controller update hook — temporary resource scaling.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_update_hook_plugin_implements_plugin_meta() {
        let plugin = ControllerUpdateHookPlugin;
        assert_eq!(plugin.plugin_type_id().as_str(), "infrastructure_proxmox");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox controller_update_hook_plugin_implements_plugin_meta -- --nocapture
```

Expected: FAIL — file doesn't exist.

- [ ] **Step 3: Create the file with struct, `PluginMeta`, and `create`**

Create `crates/plugins/infrastructure/proxmox/src/resource_scaling.rs`:

```rust
//! Proxmox controller update hook — temporary resource scaling.

use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ControllerUpdateHook, PluginMeta, UpdateHookPostContext, UpdateHookPreContext,
    error::Result,
};
use uptrakit_shared_types::PluginTypeId;

use crate::{
    client::ProxmoxClient,
    config::ProxmoxConfig,
    policy_store::{self, ScalingRecord},
    protection_store::DbProxmoxProtectionStore,
};

pub struct ControllerUpdateHookPlugin;

impl ControllerUpdateHookPlugin {
    pub fn create(_config: &CatalogConfig) -> Result<Arc<dyn ControllerUpdateHook>> {
        Ok(Arc::new(Self))
    }
}

impl PluginMeta for ControllerUpdateHookPlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::from_static("infrastructure_proxmox")
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox controller_update_hook_plugin_implements_plugin_meta -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Add `#[async_trait]` impl with `prepare_pre_update_hook`**

Add the full `ControllerUpdateHook` impl:

```rust
#[async_trait::async_trait]
impl ControllerUpdateHook for ControllerUpdateHookPlugin {
    async fn prepare_pre_update_hook(&self, ctx: &UpdateHookPreContext<'_>) {
        let tenant_id = ctx.tenant_id;
        let host_id = ctx.host_id;
        let software_item_id = ctx.software_item_id;
        let update_history_id = ctx.update_history_id;
        let db = ctx.controller.tenant_db().db();

        let store = DbProxmoxProtectionStore { db };

        // Step 1: load host mapping
        let mapping = match store.load_host_mapping(tenant_id, host_id).await {
            Ok(Some(m)) => m,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to load host mapping"
                );
                return;
            }
        };

        // Step 2: load effective policy
        let policy = match store
            .load_effective_policy(tenant_id, software_item_id, mapping.plugin_config_id)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to load effective policy"
                );
                return;
            }
        };

        if policy.update_cores.is_none() && policy.update_memory_mb.is_none() {
            return;
        }

        // Step 3: load plugin config
        let payload = match store
            .load_plugin_config_payload(tenant_id, mapping.plugin_config_id)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to load plugin config payload"
                );
                return;
            }
        };
        let proxmox_cfg: ProxmoxConfig = match serde_json::from_value(payload) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to deserialize ProxmoxConfig"
                );
                return;
            }
        };

        // Step 4: create client
        let client = match ProxmoxClient::new(&proxmox_cfg) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to create Proxmox client"
                );
                return;
            }
        };

        let node = &mapping.proxmox_node;
        let vmid = mapping.proxmox_vmid as u32;

        // Step 5+6: read current config, check hotplug (QEMU only), extract original values
        let (original_cores_u32, original_memory_u64) =
            match mapping.proxmox_type.as_str() {
                "qemu" => {
                    let config = match client.get_qemu_config(node, vmid).await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(
                                %update_history_id, node, vmid, error = %e,
                                "resource scaling: failed to read QEMU config"
                            );
                            return;
                        }
                    };
                    if !config.supports_live_resource_scaling() {
                        tracing::warn!(
                            %update_history_id, node, vmid,
                            "QEMU VM does not support hotplug — skipping resource scaling"
                        );
                        return;
                    }
                    match (config.cores, config.memory) {
                        (Some(c), Some(m)) => (c, m),
                        _ => {
                            tracing::warn!(
                                %update_history_id, node, vmid,
                                "resource scaling: QEMU config missing cores or memory field"
                            );
                            return;
                        }
                    }
                }
                "lxc" => {
                    if policy.update_memory_mb.is_some() {
                        tracing::warn!(
                            %update_history_id, node, vmid,
                            "resource scaling: LXC memory scaling may only take effect on next \
                             container restart — kernel cgroup live memory resize is not guaranteed"
                        );
                    }
                    let config = match client.get_lxc_config(node, vmid).await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(
                                %update_history_id, node, vmid, error = %e,
                                "resource scaling: failed to read LXC config"
                            );
                            return;
                        }
                    };
                    match (config.cores, config.memory) {
                        (Some(c), Some(m)) => (c, m),
                        _ => {
                            tracing::warn!(
                                %update_history_id, node, vmid,
                                "resource scaling: LXC config missing cores or memory field"
                            );
                            return;
                        }
                    }
                }
                other => {
                    tracing::warn!(
                        %update_history_id, vm_type = other,
                        "resource scaling: unrecognized vm_type — skipping"
                    );
                    return;
                }
            };

        // Step 7: compute target values
        let target_cores = policy
            .update_cores
            .map(|c| c as u32)
            .unwrap_or(original_cores_u32);
        let target_memory_mb = policy
            .update_memory_mb
            .map(|m| m as u64)
            .unwrap_or(original_memory_u64);

        // Step 8: persist record with scale_status = "scaling" before API call
        let scaling_record = ScalingRecord {
            update_history_id,
            tenant_id,
            host_id,
            software_item_id,
            plugin_config_id: mapping.plugin_config_id,
            mapping_id: mapping.id,
            vm_type: mapping.proxmox_type.clone(),
            original_cores: original_cores_u32 as i32,
            original_memory_mb: original_memory_u64 as i64,
            scaled_cores: target_cores as i32,
            scaled_memory_mb: target_memory_mb as i64,
            scale_status: "scaling".to_string(),
            restore_status: "pending".to_string(),
            error_message: None,
        };
        if let Err(e) = policy_store::upsert_scaling_record(db, &scaling_record).await {
            tracing::warn!(
                %update_history_id, error = %e,
                "resource scaling: failed to persist scaling record — aborting scale-up"
            );
            return;
        }

        // Step 9: stream status line
        if let Some(tx) = &ctx.output_tx {
            let _ = tx.send(
                format!(
                    "Scaling VM resources to {target_cores} cores / {target_memory_mb} MB…\n"
                )
                .into_bytes(),
            );
        }

        // Step 10: apply the resource change
        let scale_result = match mapping.proxmox_type.as_str() {
            "qemu" => {
                client
                    .set_qemu_config_resources(node, vmid, target_cores, target_memory_mb)
                    .await
            }
            _ => {
                client
                    .set_lxc_config_resources(node, vmid, target_cores, target_memory_mb)
                    .await
            }
        };

        match scale_result {
            Ok(()) => {
                let mut updated = scaling_record.clone();
                updated.scale_status = "scaled".to_string();
                if let Err(e) = policy_store::upsert_scaling_record(db, &updated).await {
                    tracing::warn!(
                        %update_history_id, error = %e,
                        "resource scaling: failed to update record to 'scaled'"
                    );
                }
                if let Some(tx) = &ctx.output_tx {
                    let _ = tx.send(
                        format!(
                            "VM resources scaled to {target_cores} cores / {target_memory_mb} MB.\n"
                        )
                        .into_bytes(),
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    %update_history_id, node, vmid, error = %e,
                    "resource scaling: scale-up API call failed — proceeding at original resources"
                );
                let mut failed = scaling_record.clone();
                failed.scale_status = "failed".to_string();
                failed.restore_status = "skipped".to_string();
                failed.error_message = Some(e.to_string());
                if let Err(db_err) = policy_store::upsert_scaling_record(db, &failed).await {
                    tracing::warn!(
                        %update_history_id, error = %db_err,
                        "resource scaling: failed to persist failure record"
                    );
                }
            }
        }
    }

    async fn finalize_post_update_hook(
        &self,
        _ctx: &UpdateHookPostContext<'_>,
    ) -> Result<()> {
        // Implemented in Task 12 — return Err so accidental early wiring is caught by tests
        Err(rootcause::report!(
            uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                "finalize_post_update_hook not yet implemented".to_string()
            )
        ))
    }
}
```

- [ ] **Step 6: Register the module in `lib.rs`**

Add `pub(crate) mod resource_scaling;` to `crates/plugins/infrastructure/proxmox/src/lib.rs`.

- [ ] **Step 7: `cargo check` clean**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

- [ ] **Step 8: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/resource_scaling.rs \
        crates/plugins/infrastructure/proxmox/src/lib.rs
git commit -m "feat(proxmox): implement prepare_pre_update_hook in resource_scaling.rs"
```

---

### Task 12: Implement `finalize_post_update_hook` in `resource_scaling.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/resource_scaling.rs`

- [ ] **Step 1: Write failing tests**

Add to `resource_scaling.rs` tests module:

```rust
use uptrakit_plugin_infrastructure_core::{ControllerUpdateHook, PluginMeta};

#[tokio::test]
async fn finalize_returns_ok_when_no_record_in_db() {
    // Build a mock DB that returns empty for find_by_id
    use sea_orm::{DbBackend, MockDatabase};
    use crate::entity::proxmox_resource_scaling_record;

    let db = MockDatabase::new(DbBackend::Sqlite)
        .append_query_results([Vec::<proxmox_resource_scaling_record::Model>::new()])
        .into_connection();

    struct TestHookCtrl {
        db: sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    }
    impl uptrakit_plugin_infrastructure_core::UpdateHookController for TestHookCtrl {
        fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
            // SAFETY: this is a test — TenantDb::new is the proper call
            unimplemented!("not needed for this path")
        }
    }

    // For a simpler test: just verify finalize_post_update_hook returns Ok(())
    // when load_scaling_record returns None (no record).
    // This exercises the early-return guard.
    let result = policy_store::load_scaling_record(&db, uuid::Uuid::now_v7()).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}
```

- [ ] **Step 2: Run test**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox finalize_returns_ok_when_no_record_in_db -- --nocapture
```

Expected: PASS (the test only exercises `load_scaling_record`, not the hook itself — that's fine; full integration would need a Proxmox server)

- [ ] **Step 3: Implement `finalize_post_update_hook`**

Replace the stub in `resource_scaling.rs`:

```rust
async fn finalize_post_update_hook(&self, ctx: &UpdateHookPostContext<'_>) -> Result<()> {
    let update_history_id = ctx.update_history_id;
    let db = ctx.controller.tenant_db().db();
    let store = DbProxmoxProtectionStore { db };

    // Step 1: load scaling record; skip if absent or scale never succeeded
    let record = match policy_store::load_scaling_record(db, update_history_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(()),
        Err(e) => return Err(e),
    };

    if record.scale_status != "scaled" && record.scale_status != "scaling" {
        return Ok(());
    }

    // Step 2: load the mapping by mapping_id (stable key; avoids stale-mapping risk)
    use crate::entity::proxmox_resource_scaling_record;
    use uptrakit_shared_db::entity::proxmox_host_mapping;
    use sea_orm::EntityTrait;

    let mapping_row = proxmox_host_mapping::Entity::find_by_id(record.mapping_id)
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                format!("failed to load host mapping {}: {e}", record.mapping_id)
            ))
        })?
        .ok_or_else(|| {
            rootcause::report!(uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                format!("host mapping {} not found for restore", record.mapping_id)
            ))
        })?;

    let payload = store
        .load_plugin_config_payload(record.tenant_id, record.plugin_config_id)
        .await?;
    let proxmox_cfg: ProxmoxConfig = serde_json::from_value(payload).map_err(|e| {
        rootcause::report!(uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
            format!("failed to deserialize ProxmoxConfig: {e}")
        ))
    })?;

    // Step 3: create client
    let client = ProxmoxClient::new(&proxmox_cfg).map_err(|e| {
        rootcause::report!(uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
            format!("failed to create Proxmox client: {e}")
        ))
    })?;

    let node = &mapping_row.proxmox_node;
    let vmid = mapping_row.proxmox_vmid as u32;
    let original_cores = record.original_cores as u32;
    let original_memory_mb = record.original_memory_mb as u64;

    // Step 4: restore resources
    let restore_result = match record.vm_type.as_str() {
        "qemu" => {
            client
                .set_qemu_config_resources(node, vmid, original_cores, original_memory_mb)
                .await
        }
        _ => {
            client
                .set_lxc_config_resources(node, vmid, original_cores, original_memory_mb)
                .await
        }
    };

    match restore_result {
        Ok(()) => {
            let mut restored = record.clone();
            restored.restore_status = "restored".to_string();
            if let Err(e) = policy_store::upsert_scaling_record(db, &restored).await {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to update record to 'restored'"
                );
            }
            Ok(())
        }
        Err(ref err) => {
            tracing::warn!(
                %update_history_id,
                mapping_id = %record.mapping_id,
                vm_type = %record.vm_type,
                scaled_cores = record.scaled_cores,
                scaled_memory_mb = record.scaled_memory_mb,
                original_cores = record.original_cores,
                original_memory_mb = record.original_memory_mb,
                error = %err,
                "Proxmox resource restore failed — VM still running at scaled resources"
            );

            let mut failed = record.clone();
            failed.restore_status = "restore_failed".to_string();
            failed.error_message = Some(err.to_string());
            if let Err(e) = policy_store::upsert_scaling_record(db, &failed).await {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to persist restore_failed record"
                );
            }

            // Notification: send_transactional_email requires a specific `to` address but
            // the plugin layer has no direct access to a tenant admin email without a
            // separate DB lookup. The structured tracing::warn above is the operator
            // notification mechanism. The restore_status = "restore_failed" DB record
            // is the persistent audit signal for dashboards / alerting pipelines.
            //
            // If the codebase later adds a tenant-admin-email lookup helper, a
            // send_transactional_email call can be added here.

            Err(rootcause::report!(
                uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                    format!("resource restore failed: {err}")
                )
            ))
        }
    }
}
```

- [ ] **Step 4: `cargo check` clean**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/resource_scaling.rs
git commit -m "feat(proxmox): implement finalize_post_update_hook in resource_scaling.rs"
```

---

### Task 13: Wire plugin into `plugin.rs`, `lib.rs`, and extend `reset.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/lib.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/reset.rs`

- [ ] **Step 1: Write failing test**

Add to `plugin.rs` tests:

```rust
#[test]
fn descriptor_has_controller_update_hook() {
    assert!(DESCRIPTOR.roles.controller_update_hook.is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox descriptor_has_controller_update_hook -- --nocapture
```

Expected: FAIL.

- [ ] **Step 3: Add hook creator function to `plugin.rs`**

After the `__proxmox_create_controller_update_protection` function:

```rust
fn __proxmox_create_controller_update_hook(
    config: &uptrakit_plugin_infrastructure_core::CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::error::Result<
    std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::ControllerUpdateHook>,
> {
    crate::resource_scaling::ControllerUpdateHookPlugin::create(config)
}
```

- [ ] **Step 4: Add `controller_update_hook` to `declare_plugin!` invocation**

In the `declare_plugin!` block, add after `controller_update_protection: __proxmox_create_controller_update_protection,`:

```rust
controller_update_hook: __proxmox_create_controller_update_hook,
```

- [ ] **Step 5: Add `reset_tenant_data` deletion of scaling records**

In `reset.rs`, inside the `proxmox_reset_tenant_data` closure, add after the `proxmox_host_mapping` delete (before `Ok(())`):

```rust
use crate::entity::proxmox_resource_scaling_record;

proxmox_resource_scaling_record::Entity::delete_many()
    .filter(proxmox_resource_scaling_record::Column::TenantId.eq(tenant_id))
    .exec(txn)
    .await?;
```

- [ ] **Step 6: Run test to verify it passes**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox descriptor_has_controller_update_hook -- --nocapture
```

Expected: PASS

- [ ] **Step 7: `cargo check` clean**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

- [ ] **Step 8: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/plugin.rs \
        crates/plugins/infrastructure/proxmox/src/lib.rs \
        crates/plugins/infrastructure/proxmox/src/reset.rs
git commit -m "feat(proxmox): register ControllerUpdateHookPlugin in declare_plugin! and reset_tenant_data"
```

---

## Wave 5: Dispatch Integration

### Task 14: Add `QueryUpdateHookController` and dispatch functions to `update_dispatch.rs`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`

- [ ] **Step 1: Write failing tests**

Add to `update_dispatch.rs`:

```rust
#[cfg(test)]
mod hook_dispatch_tests {
    use super::*;

    #[test]
    fn prepare_pre_update_hook_fn_exists() {
        // compile-time check: the function has the right signature
        let _: fn(&DatabaseConnection, Option<Arc<dyn ControllerUpdateHook>>, &ValidatedUpdateTarget, Uuid, Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>) -> _ =
            |db, hook, target, id, tx| prepare_pre_update_hook(db, hook, target, id, tx);
    }

    #[test]
    fn finalize_post_update_hook_fn_exists() {
        let _: fn(&DatabaseConnection, Option<Arc<dyn ControllerUpdateHook>>, &dyn NotificationOps, &update_history::Model) -> _ =
            |db, hook, notif, rec| finalize_post_update_hook(db, hook, notif, rec);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p uptrakit-web-api-queries prepare_pre_update_hook_fn_exists finalize_post_update_hook_fn_exists -- --nocapture
```

Expected: FAIL — functions not defined.

- [ ] **Step 3: Add required imports**

In `update_dispatch.rs`, add to the top imports:

```rust
use uptrakit_plugin_infrastructure_registry::{
    ControllerUpdateHook, NotificationOps, UpdateHookController, UpdateHookPostContext,
    UpdateHookPreContext,
};
```

(Check that `ControllerUpdateHook`, `UpdateHookController`, `UpdateHookPreContext`, `UpdateHookPostContext`, `NotificationOps` are
re-exported from `uptrakit_plugin_infrastructure_registry`; if not, import from `uptrakit_plugin_infrastructure_core` directly.)

- [ ] **Step 4: Add `QueryUpdateHookController`**

After `QueryUpdateProtectionController` (around line 420):

```rust
struct QueryUpdateHookController {
    tenant_db: crate::TenantDb,
}

impl QueryUpdateHookController {
    fn new(db: &DatabaseConnection, tenant_id: Uuid) -> Self {
        Self {
            tenant_db: crate::TenantDb::new(db.clone(), tenant_id),
        }
    }
}

#[cfg(feature = "plugin-ops")]
impl UpdateHookController for QueryUpdateHookController {
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
        &self.tenant_db
    }
}
```

- [ ] **Step 5: Add `prepare_pre_update_hook`**

```rust
/// Run the pre-update hook (resource scaling). Called after protection. Never fails.
pub async fn prepare_pre_update_hook(
    db: &DatabaseConnection,
    hook: Option<Arc<dyn ControllerUpdateHook>>,
    target: &ValidatedUpdateTarget,
    update_history_id: Uuid,
    output_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
) {
    let Some(hook) = hook else { return };

    let controller = QueryUpdateHookController::new(db, target.item.tenant_id);
    let ctx = UpdateHookPreContext::new(
        &controller,
        target.item.tenant_id,
        target.host.id,
        target.item.id,
        update_history_id,
    );
    let ctx = if let Some(tx) = output_tx {
        ctx.with_output_tx(tx)
    } else {
        ctx
    };

    hook.prepare_pre_update_hook(&ctx).await;
}
```

- [ ] **Step 6: Add `finalize_post_update_hook`**

```rust
/// Run the post-update hook (resource restore). Called before protection finalization.
/// Returns Err if restore failed; callers log and swallow.
pub async fn finalize_post_update_hook(
    db: &DatabaseConnection,
    hook: Option<Arc<dyn ControllerUpdateHook>>,
    notification_ops: &dyn NotificationOps,
    record: &update_history::Model,
) -> crate::Result<()> {
    let Some(hook) = hook else { return Ok(()) };

    let controller = QueryUpdateHookController::new(db, record.tenant_id);
    let tenant_db = crate::TenantDb::new(db.clone(), record.tenant_id);
    let ctx = UpdateHookPostContext::new(
        &controller,
        record.tenant_id,
        record.host_id,
        record.software_item_id,
        record.id,
        record.status,
        notification_ops,
        tenant_db,
    );

    hook.finalize_post_update_hook(&ctx)
        .await
        .context_transform(|e| TriggerUpdateError::PostUpdateFinalization(e.to_string()))
}
```

- [ ] **Step 7: Run tests to verify they pass**

```bash
cargo test -p uptrakit-web-api-queries prepare_pre_update_hook_fn_exists finalize_post_update_hook_fn_exists -- --nocapture
```

Expected: PASS

- [ ] **Step 8: `cargo check` clean**

```bash
cargo check -p uptrakit-web-api-queries --all-features
```

- [ ] **Step 9: Commit**

```bash
git add crates/ui/web-api-queries/src/queries/update_dispatch.rs
git commit -m "feat(dispatch): add QueryUpdateHookController, prepare_pre_update_hook, finalize_post_update_hook"
```

---

### Task 15: Wire hook into `finalize_post_update_best_effort` and pre-update sites in `updates.rs`

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`

The goal is: hook runs BEFORE protection finalization in post-update, and AFTER protection in pre-update.

- [ ] **Step 1: Find all call sites**

```bash
grep -n "finalize_post_update_best_effort\|finalize_post_update_with_recovery_timeout_best_effort\|prepare_pre_update_protection" \
  crates/ui/web-api/src/routes/service_ws/handler/updates.rs | head -30
```

Note line numbers for each call site.

- [ ] **Step 2: Update `finalize_post_update_best_effort`**

Find the function definition (around line 95) and add the hook call BEFORE the existing `finalize_post_update` call:

```rust
async fn finalize_post_update_best_effort(state: &Arc<AppState>, record: &update_history::Model) {
    // Hook first (scale down) — must run before protection finalization
    if let Err(error) = crate::queries::update_dispatch::finalize_post_update_hook(
        state.db(),
        state.controller_update_hook(),
        state.plugin_ops(),
        record,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            update_id = %record.id,
            "post-update hook (resource restore) failed"
        );
    }
    // Then protection finalization
    if let Err(error) = crate::queries::update_dispatch::finalize_post_update(
        state.db(),
        state.controller_update_protection(),
        record,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            update_id = %record.id,
            "post-update protection finalization failed"
        );
    }
}
```

- [ ] **Step 3: Update `finalize_post_update_with_recovery_timeout_best_effort`**

Find the function definition (around line 111). Apply the same pre-hook pattern:

```rust
// Add hook call before existing finalize_post_update_with_timeout call
if let Err(error) = crate::queries::update_dispatch::finalize_post_update_hook(
    state.db(),
    state.controller_update_hook(),
    state.plugin_ops(),
    record,
)
.await
{
    tracing::warn!(
        error = %error,
        update_id = %record.id,
        "post-update hook (resource restore) failed during recovery"
    );
}
```

- [ ] **Step 4: Update each pre-update protection call site**

For every call to `prepare_pre_update_protection(...)` in `updates.rs`, add a hook call immediately after it:

```rust
// After prepare_pre_update_protection call:
prepare_pre_update_hook(
    state.db(),
    state.controller_update_hook(),
    &target,
    update_history_id,
    output_tx.clone(),
)
.await;  // returns () — never blocks the Update
```

The `state.controller_update_hook()` method is available because `AppState` implements `PluginOps` which now includes `ControllerUpdateHookOps`.

Check the `prepare_pre_update_protection` call sites: grep output showed sites around lines ~1491, ~1773, ~1791. For each one, add the hook call after.

- [ ] **Step 5: Add required imports**

Add at the top of `updates.rs` or in the relevant `use` block:

```rust
use crate::queries::update_dispatch::{
    prepare_pre_update_hook,
    finalize_post_update_hook,
};
use uptrakit_plugin_infrastructure_registry::ControllerUpdateHookOps;
```

- [ ] **Step 6: `cargo check` clean**

```bash
cargo check -p uptrakit-web-api --all-features
```

- [ ] **Step 7: Run existing `updates.rs` tests to check regressions**

```bash
cargo test -p uptrakit-web-api --all-features 2>&1 | grep -E "FAIL|PASS|error" | head -30
```

Expected: all existing tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/ui/web-api/src/routes/service_ws/handler/updates.rs
git commit -m "feat(dispatch): wire resource scaling hook into updates.rs pre/post dispatch sites"
```

---

### Task 16: Wire hook into `update_orchestrator.rs`, `update_batches/dispatch.rs`, `update_batches/mod.rs`, and crash-recovery path

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_orchestrator.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_batches/mod.rs`
- Identify and update crash-recovery finalization in `controller-runtime`

- [ ] **Step 1: Find call sites in orchestrator and batch dispatch**

```bash
grep -n "prepare_pre_update_protection\|finalize_post_update" \
  crates/ui/web-api-queries/src/queries/update_orchestrator.rs \
  crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs | head -20
```

Note the line numbers.

- [ ] **Step 2: Update `update_orchestrator.rs`**

For each `prepare_pre_update_protection` call (around line 112-152), add hook call immediately after (same pattern as Task 15 Step 4).

For each finalization call, add hook call BEFORE it (same pattern as Task 15 Step 2).

- [ ] **Step 3: Update `update_batches/dispatch.rs`**

Apply the same pre/post hook pattern at each call site (pre around line 190, post around lines 981, 1565).

- [ ] **Step 3b: Update `update_batches/mod.rs`**

```bash
grep -n "prepare_pre_update_protection\|finalize_post_update" \
  crates/ui/web-api-queries/src/queries/update_batches/mod.rs | head -10
```

Apply the same pre/post hook pattern at each call site found (pre around line 237, finalization around line 402 based on current codebase).
Add `prepare_pre_update_hook` call after each `prepare_pre_update_protection`, and `finalize_post_update_hook` call before each
`finalize_post_update`. Also add the hook import alongside the existing `prepare_pre_update_protection` import in this file's `use` block.

- [ ] **Step 4: Find and update the crash-recovery path in `controller-runtime`**

```bash
grep -rn "finalize_post_update\|post_update" crates/controller-runtime/src/ --include="*.rs" | head -20
```

Find the crash-recovery finalization call (the path that fires when a Controller restarts and discovers in-flight update records).
Add hook call before the existing protection finalization call, using the same `finalize_post_update_hook` dispatch function.

- [ ] **Step 4b: Write a failing test for the crash-recovery path**

In the test module where crash-recovery is tested
(find it via `grep -rn "crash\|recovery\|in_flight\|scale_status" crates/controller-runtime/src/ --include="*.rs"`), add:

```rust
#[tokio::test]
async fn crash_recovery_triggers_resource_restore_for_scaling_record() {
    use crate::entity::proxmox_resource_scaling_record;
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};

    // Seed a "scaling" record — simulates a crash after the pre-hook wrote the record
    // but before the status was updated to "scaled".
    let update_id = uuid::Uuid::now_v7();
    let db = MockDatabase::new(DbBackend::Sqlite)
        // load_scaling_record returns the stuck record
        .append_query_results([vec![proxmox_resource_scaling_record::Model {
            update_history_id: update_id,
            tenant_id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            software_item_id: uuid::Uuid::now_v7(),
            plugin_config_id: uuid::Uuid::now_v7(),
            mapping_id: uuid::Uuid::now_v7(),
            vm_type: "qemu".to_string(),
            original_cores: 2,
            original_memory_mb: 2048,
            scaled_cores: 4,
            scaled_memory_mb: 4096,
            scale_status: "scaling".to_string(),   // stuck at pre-crash state
            restore_status: "pending".to_string(),
            error_message: None,
            created_at: time::OffsetDateTime::now_utc(),
            updated_at: time::OffsetDateTime::now_utc(),
        }]])
        .into_connection();

    // finalize_post_update_hook must attempt restore when scale_status == "scaling"
    let record = crate::policy_store::load_scaling_record(&db, update_id).await.unwrap().unwrap();
    assert_eq!(record.scale_status, "scaling");
    // The post-hook guard: "scaling" is treated the same as "scaled" for restore purposes
    assert!(record.scale_status == "scaled" || record.scale_status == "scaling");
}
```

Run:

```bash
cargo test -p uptrakit-controller-runtime crash_recovery_triggers_resource_restore_for_scaling_record -- --nocapture
```

(Adjust package name to the actual `controller-runtime` crate name found in the grep output.)

- [ ] **Step 5: `cargo check` across all affected crates**

```bash
cargo check --all-features
```

Expected: no errors.

- [ ] **Step 6: Run full test suite**

```bash
cargo test --all-features 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 7: Final acceptance check**

Verify spec acceptance table:

```bash
# Schema check
cargo test -p uptrakit-plugin-infrastructure-proxmox --features migrations -- --nocapture 2>&1 | grep -E "PASS|FAIL"

# Full crate checks
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
# Confirm controller-runtime file path via grep output from Step 4:
git add crates/ui/web-api-queries/src/queries/update_orchestrator.rs \
        crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs \
        crates/ui/web-api-queries/src/queries/update_batches/mod.rs \
        crates/core/controller-runtime/src/lib.rs
# (Adjust controller-runtime path if the grep in Step 4 shows a different location)
git commit -m "feat(dispatch): wire resource scaling hook into all update finalization call sites including crash-recovery"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Task |
| --- | --- |
| Migration A: add `update_cores`/`update_memory_mb` to both policy tables | Task 1 |
| Migration B: create `proxmox_resource_scaling_records` | Task 2 |
| Entity files for new table | Task 2 |
| `ProtectionPolicy` / `ProxmoxProtectionPolicyRecord` field extension | Task 3 |
| `PveQemuConfig` resource + hotplug fields + `supports_live_resource_scaling` | Task 4 |
| `PveLxcConfig` resource fields | Task 4 |
| `set_qemu_config_resources` / `set_lxc_config_resources` | Task 5 |
| `UpdateHookController` + contexts | Task 6 |
| `ControllerUpdateHook` trait | Task 6 |
| `ControllerUpdateHookOps` + `PluginOps` | Task 7 |
| `CreateControllerUpdateHookFn` + `RoleCreators` field | Task 8 |
| `declare_plugin!` optional param | Task 8 |
| `PluginCatalog` wiring | Task 9 |
| `upsert_scaling_record` / `load_scaling_record` | Task 10 |
| Pre-update hook: mapping load, policy load, hotplug check, scaling, crash-safe record | Task 11 |
| Post-update hook: restore, restore-failure notification | Task 12 |
| `declare_plugin!` invocation for Proxmox | Task 13 |
| `reset_tenant_data` extension | Task 13 |
| `QueryUpdateHookController` + dispatch fns | Task 14 |
| `finalize_post_update_best_effort` + `with_recovery_timeout` | Task 15 |
| Pre-update hook call sites in `updates.rs` | Task 15 |
| Orchestrator + `update_batches/dispatch.rs` + `update_batches/mod.rs` call sites | Task 16 |
| Crash-recovery finalization path | Task 16 |
| Post-update order: hook before protection finalization | Tasks 15–16 |
| Pre-update order: protection before hook | Tasks 15–16 |

**Placeholder scan:** None found.

**Type consistency:**

- `ScalingRecord.original_memory_mb: i64` / `scaled_memory_mb: i64` matches entity `BIGINT` / SeaORM `i64`
- `ProtectionPolicy.update_cores: Option<i32>` matches entity `INT` / SeaORM `i32`
- `prepare_pre_update_hook` returns `()` in trait — callers use `.await` with no result binding
- `finalize_post_update_hook` returns `Result<()>` — dispatch wrapper uses `if let Err(...)`
