# Proxmox Resource Scaling Hook

**Date:** 2026-05-03
**Status:** Spec

## Background

The Proxmox plugin already implements `ControllerUpdateProtection` to create snapshots or
backups before an Update executes. That contract has a clear semantic: produce a recovery
artifact. A separate orthogonal need exists: temporarily give the target VM more CPU cores
and RAM while the Update runs, then restore original values afterward. Scaling and
protection are independent — an Operator should be able to enable either without the other.
Mixing scaling into `ControllerUpdateProtection` would violate its single-responsibility
contract and prevent future non-Proxmox plugins from implementing one without the other.

This spec introduces a new generic `ControllerUpdateHook` trait for pre/post-update side
effects that do not produce recovery artifacts, wires it through the dispatch layer, and
implements it in the Proxmox plugin for temporary CPU/RAM scaling.

## Goals

1. Operators can configure `update_cores` and `update_memory_mb` per Software Item (with
   a plugin-config-level default), causing the mapped VM to be scaled up before the
   Update and restored after.
2. Scaling is best-effort: failure to scale up logs a warning and allows the Update to
   proceed at original resources. The Update is never blocked by scaling failures.
3. For QEMU VMs, hotplug capability is checked before attempting live resource changes;
   if hotplug is absent the VM is silently skipped with a log warning.
4. Original resource values are persisted to DB before scaling so that a Controller
   restart between pre- and post-update does not prevent restoration.
5. Restore failures log a warning and attempt a best-effort notification through
   whatever notification channel is available; the Update outcome is not changed.
6. All new code uses `tenant_db()` directly. No plugin-named store traits are introduced,
   keeping the feature boundary-hardening-compatible from day one.
7. Pre-update execution order: protection (snapshot/backup) runs first, then scaling.
   Post-update execution order: scaling restore runs first, then protection finalization.

## Non-Goals

- Delta scaling (e.g., "+2 cores") — future extension; column types are chosen not to
  preclude it, but no delta logic is implemented here.
- UI surfaces for configuring `update_cores`/`update_memory_mb` — values are set via the
  existing policy surface that already exposes protection mode and backup target key.
- Hotplug detection for LXC containers — LXC supports live resource changes natively;
  no hotplug field needs checking.
- A dedicated notification surface for the restore failure — `tracing::warn!` plus
  `send_transactional_email` via `NotificationOps` are the two channels used.
- Cleanup of stale scaling records — records are retained indefinitely alongside the
  update history row they are keyed on.

## Dependency

This spec is independent of boundary-hardening wave ordering, but the new code uses
`tenant_db()` directly (following the post-Wave-3 pattern). `send_transactional_email` on
`NotificationOps` is available — boundary-hardening has merged.

---

## Wave 1: DB schema — policy columns + scaling record table

**Goal:** Extend existing policy tables with optional scaling config and create a new
scaling record table to persist original/target resource values across Controller restarts.

### Migration A: `AddProxmoxResourceScalingPolicyColumns`

Migration name: `"m20260503_000001_proxmox_resource_scaling_policy"`.

Add two nullable integer columns to both `proxmox_protection_defaults` and
`proxmox_protection_item_overrides`:

| Column | Type | Constraint |
| --- | --- | --- |
| `update_cores` | `INT` | `NULL` |
| `update_memory_mb` | `INT` | `NULL` |

`NULL` means no scaling configured. Both columns are `NULL` on all existing rows after
the migration (no backfill needed).

### Migration B: `CreateProxmoxResourceScalingRecord`

Migration name: `"m20260503_000002_proxmox_resource_scaling_record"`.

Create table `proxmox_resource_scaling_records`:

| Column | Type | Notes |
| --- | --- | --- |
| `update_history_id` | `UUID` | Primary key |
| `tenant_id` | `UUID` | Not null |
| `host_id` | `UUID` | Not null |
| `software_item_id` | `UUID` | Not null |
| `plugin_config_id` | `UUID` | Not null |
| `mapping_id` | `UUID` | Not null |
| `vm_type` | `VARCHAR(16)` | Not null — `"qemu"` or `"lxc"` (from `mapping.proxmox_type`) |
| `original_cores` | `INT` | Not null — value read from Proxmox before scaling |
| `original_memory_mb` | `BIGINT` | Not null — use BIGINT to match `u64` Proxmox API type |
| `scaled_cores` | `INT` | Not null — target value applied by pre-update hook |
| `scaled_memory_mb` | `BIGINT` | Not null |
| `scale_status` | `VARCHAR(32)` | Not null — `scaling` / `scaled` / `skipped` / `failed` |
| `restore_status` | `VARCHAR(32)` | Not null — `pending` / `restored` / `restore_failed` / `skipped` (used when `scale_status = "failed"`) |
| `error_message` | `TEXT` | Nullable |
| `created_at` | `TIMESTAMP` | Not null (use `.timestamp()` in SeaORM migration, matching all other Proxmox tables) |
| `updated_at` | `TIMESTAMP` | Not null |

No FK to `update_history` — that table lives in `shared-db`; cross-crate FK constraints
are not used elsewhere in this plugin (matching the `proxmox_protection_audit` precedent).

Both migrations go into `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`
and are appended to the `migrations()` vec.

### SeaORM entity updates

**`proxmox_protection_default.rs`** — already in
`crates/plugins/infrastructure/proxmox/src/entity/` (moved by boundary-hardening Wave 4).
Add:

```rust
pub update_cores: Option<i32>,
pub update_memory_mb: Option<i32>,
```

**`proxmox_protection_item_override.rs`** — same two fields, same location.

**New entity** `proxmox_resource_scaling_record.rs` placed directly in
`crates/plugins/infrastructure/proxmox/src/entity/proxmox_resource_scaling_record.rs`.
Add `pub mod proxmox_resource_scaling_record;` to `src/entity/mod.rs`. Entity field types
follow SeaORM conventions: `INT` → `i32`, `BIGINT` → `i64`, `UUID` → `Uuid`, `TEXT` →
`String`. `original_memory_mb` and `scaled_memory_mb` are `i64` (matching their `BIGINT`
column type).

### Policy struct extensions

The merge introduced two separate policy structs that serve different consumers. Both
need the new fields.

**`policy_store.rs` — `ProtectionPolicy`** (pub; used by surface action handlers):

```rust
pub update_cores: Option<i32>,
pub update_memory_mb: Option<i32>,
```

SeaORM maps SQLite `INT` columns to `i32`. `ProtectionPolicy::do_nothing()` sets both to
`None`. Update `load_global_default`, `load_item_override`, `resolve_effective_policy`,
`upsert_global_default`, and `upsert_item_override` to cascade and persist the new fields —
same pattern as `backup_target_key`.

**`protection_store.rs` — `ProxmoxProtectionPolicyRecord`** (pub(crate); used by
`DbProxmoxProtectionStore.load_effective_policy()` and, in this feature, by
`resource_scaling.rs`):

```rust
pub update_cores: Option<i32>,
pub update_memory_mb: Option<i32>,
```

Update `DbProxmoxProtectionStore`'s `load_effective_policy` implementation to query and
cascade these columns from the entity — same override logic as `backup_target_key`.

### Acceptance

- `cargo check --all-features` clean.
- Migration integration test (SQLite) verifies both new columns appear in
  `proxmox_protection_defaults` and `proxmox_protection_item_overrides`.
- Migration integration test verifies `proxmox_resource_scaling_records` is created with
  all expected columns.

---

## Wave 2: API type and client extensions

**Goal:** Extend the Proxmox API types to expose resource and hotplug fields, and add
client methods to apply resource changes.

### `api_types.rs` changes

Extend `PveQemuConfig`:

```rust
pub struct PveQemuConfig {
    #[serde(default)]
    pub name: Option<String>,
    /// Number of CPU cores (matches Proxmox `cores` field).
    #[serde(default)]
    pub cores: Option<u32>,
    /// Memory in MB (matches Proxmox `memory` field).
    #[serde(default)]
    pub memory: Option<u64>,
    /// Comma-separated hotplug device list, e.g. `"disk,network,usb,memory,cpu"`.
    /// Absent means hotplug is disabled.
    #[serde(default)]
    pub hotplug: Option<String>,
}
```

A QEMU VM supports live CPU+RAM scaling when `hotplug` contains both `"cpu"` and
`"memory"`. Helper:

```rust
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

Extend `PveLxcConfig`:

```rust
pub struct PveLxcConfig {
    #[serde(default)]
    pub hostname: Option<String>,
    /// Number of CPU cores.
    #[serde(default)]
    pub cores: Option<u32>,
    /// Memory limit in MB.
    #[serde(default)]
    pub memory: Option<u64>,
}
```

LXC does not use the hotplug field; live resource updates are always supported.

### `client.rs` new methods

```rust
/// Apply CPU and memory limits to a running QEMU VM via the Proxmox API.
///
/// Calls `PUT /api2/json/nodes/{node}/qemu/{vmid}/config` with `cores` and
/// `memory` fields. Proxmox applies the change live when hotplug is enabled.
pub async fn set_qemu_config_resources(
    &self,
    node: &str,
    vmid: u32,
    cores: u32,
    memory_mb: u64,
) -> Result<()>

/// Apply CPU and memory limits to a running LXC container.
///
/// Calls `PUT /api2/json/nodes/{node}/lxc/{vmid}/config`.
pub async fn set_lxc_config_resources(
    &self,
    node: &str,
    vmid: u32,
    cores: u32,
    memory_mb: u64,
) -> Result<()>
```

Both methods issue a `PUT` to the respective config endpoint with
`{"cores": cores, "memory": memory_mb}` as a `application/x-www-form-urlencoded` body
(matching how Proxmox accepts config changes for non-task fields). The response is
ignored beyond HTTP status — Proxmox returns `null` data for synchronous config changes.

### Acceptance

- `PveQemuConfig::supports_live_resource_scaling` unit tests:
  - `hotplug = None` → `false`
  - `hotplug = Some("disk,network")` → `false` (missing cpu and memory)
  - `hotplug = Some("disk,network,usb,memory,cpu")` → `true`
- New client methods compile; no integration test required (Proxmox API is external).

---

## Wave 3: New trait infrastructure in `plugin-infrastructure-core`

**Goal:** Establish the `ControllerUpdateHook` extension point, its context types,
the `UpdateHookController` DB-access seam, and the catalog accessor, following the
exact patterns used for `ControllerUpdateProtection`.

### `roles.rs` additions

`uptrakit-tenant-db` is already a dep of `plugin-infrastructure-core` gated behind
`plugin-ops` (added by boundary-hardening Wave 1). No new dep needed.

**`UpdateHookController` trait** (gated behind `plugin-ops` feature, same as
`UpdateProtectionController` after boundary-hardening Wave 2):

```rust
#[cfg(feature = "plugin-ops")]
pub trait UpdateHookController: Send + Sync {
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
}
```

**Context types** (both gated behind `plugin-ops` because they reference `UpdateHookController`):

```rust
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
    ) -> Self { ... }

    pub fn with_output_tx(mut self, tx: UnboundedSender<Vec<u8>>) -> Self { ... }
}

#[cfg(feature = "plugin-ops")]
#[non_exhaustive]
pub struct UpdateHookPostContext<'a> {
    pub controller: &'a dyn UpdateHookController,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
    pub final_status: uptrakit_shared_types::UpdateStatus,
    pub notification_ops: &'a dyn NotificationOps,
    /// Tenant-scoped DB handle required by `NotificationOps::send_transactional_email`.
    pub tenant_db: uptrakit_tenant_db::TenantDb,
}

#[cfg(feature = "plugin-ops")]
impl<'a> UpdateHookPostContext<'a> {
    pub fn new(
        controller: &'a dyn UpdateHookController,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        update_history_id: Uuid,
        final_status: uptrakit_shared_types::UpdateStatus,
        notification_ops: &'a dyn NotificationOps,
        tenant_db: uptrakit_tenant_db::TenantDb,
    ) -> Self { ... }
}
```

`NotificationOps` is already re-exported from `plugin-infrastructure-core/lib.rs`; no new
dep needed.

**`ControllerUpdateHook` trait:**

```rust
#[async_trait]
pub trait ControllerUpdateHook: PluginMeta + Send + Sync {
    /// Called before the update executes. Always best-effort: returns `()` so that
    /// callers cannot accidentally treat a scale-up failure as update-blocking.
    async fn prepare_pre_update_hook(&self, ctx: &UpdateHookPreContext<'_>);

    /// Called after the update completes. Returns `Result<()>` so restore failures
    /// propagate to the dispatch wrapper, which logs them and then swallows.
    async fn finalize_post_update_hook(
        &self,
        ctx: &UpdateHookPostContext<'_>,
    ) -> Result<()>;
}
```

`prepare_pre_update_hook` returns `()` — making the best-effort contract explicit at the
type level and preventing future implementors from accidentally treating errors as
blocking. `finalize_post_update_hook` returns `Result<()>` to surface restore failures
for logging; the dispatch wrapper always swallows the error.

### `plugin_ops.rs` additions

```rust
pub trait ControllerUpdateHookOps: Send + Sync + 'static {
    fn controller_update_hook(&self) -> Option<Arc<dyn ControllerUpdateHook>> {
        None
    }
}
```

Add `ControllerUpdateHookOps` to the `PluginOps` supertrait list AND to the `where T:`
clause of the blanket `impl<T> PluginOps for T where T: ...` block — both must be updated.
Test stubs (`ProtectionOverridePluginOps`, `TestPluginOps`) inherit the default `None`
impl — only `PluginCatalog` overrides it.

### `descriptor.rs` additions

New type alias:

```rust
pub type CreateControllerUpdateHookFn =
    fn(&CatalogConfig) -> crate::error::Result<Arc<dyn roles::ControllerUpdateHook>>;
```

Add to `RoleCreators`:

```rust
pub controller_update_hook: Option<CreateControllerUpdateHookFn>,
```

Update `declare_plugin!` macro to accept an optional `controller_update_hook: $hook_fn:expr`
parameter (following the `migrations:` optional-parameter pattern already in the macro)
and emit `controller_update_hook: $crate::__option_expr!( $( $hook_fn )? )` in the
`RoleCreators` literal. This sweeps all plugin crates atomically and defaults to `None`.

### `catalog.rs` additions

Store `Option<Arc<dyn ControllerUpdateHook>>` alongside
`Option<Arc<dyn ControllerUpdateProtection>>`. Implement `ControllerUpdateHookOps` on
`PluginCatalog`, overriding the default to return the stored reference. Populate at
catalog construction from `descriptor.roles.controller_update_hook` — same pattern
as `controller_update_protection`.

### Acceptance

- All existing plugin crates compile with the new `controller_update_hook: None` default.
- `PluginOps` bound satisfied by `PluginCatalog`.
- `cargo check --all-features` clean.

---

## Wave 4: Proxmox plugin implementation

**Goal:** Implement `ControllerUpdateHook` for the Proxmox plugin, performing hotplug
detection, resource scaling, record persistence, restore, and restore-failure notification.

### New file `crates/plugins/infrastructure/proxmox/src/resource_scaling.rs`

**`ControllerUpdateHookPlugin`** struct (unit struct, same pattern as
`ControllerUpdateProtectionPlugin`):

```rust
pub struct ControllerUpdateHookPlugin;

impl ControllerUpdateHookPlugin {
    pub fn create(_config: &CatalogConfig) -> Result<Arc<dyn ControllerUpdateHook>> {
        Ok(Arc::new(Self))
    }
}
```

**`prepare_pre_update_hook` logic:**

`resource_scaling.rs` uses `DbProxmoxProtectionStore` for all DB access — same pattern
as `update_protection.rs`:

```rust
let store = DbProxmoxProtectionStore { db: ctx.controller.tenant_db().db() };
```

1. Call `store.load_host_mapping(tenant_id, host_id)`.
   If no mapping → return early (no-op).
2. Call `store.load_effective_policy(tenant_id, software_item_id, mapping.plugin_config_id)`.
   If `update_cores.is_none() && update_memory_mb.is_none()` → return early (no-op).
3. Call `store.load_plugin_config_payload(tenant_id, mapping.plugin_config_id)` then
   `serde_json::from_value(payload)` into `crate::config::ProxmoxConfig`.
   On error → `tracing::warn!(...)`, return early.
4. `ProxmoxClient::new(&proxmox_cfg)` — on error → `tracing::warn!(...)`, return early.
5. Read current VM config using `mapping.proxmox_type` to branch:
   - QEMU (`mapping.proxmox_type == "qemu"`): `client.get_qemu_config(node, vmid).await`
     Check `config.supports_live_resource_scaling()`. If `false` →
     `tracing::warn!("QEMU VM {vmid} on {node} does not support hotplug — skipping resource scaling")` → return early.
   - LXC (`mapping.proxmox_type == "lxc"`): `client.get_lxc_config(node, vmid).await` (no hotplug check).
   On API error → `tracing::warn!(...)`, return early.
6. Extract `original_cores` and `original_memory_mb` from the config response.
   If either is absent (Proxmox did not return the field) → `tracing::warn!(...)`, return early.
   **Type notes:**
   - `PveQemuConfig.cores: Option<u32>`, `memory: Option<u64>` — API types.
   - DB `original_cores`/`scaled_cores` are `INT` → SeaORM `i32`.
   - DB `original_memory_mb`/`scaled_memory_mb` are `BIGINT` → SeaORM `i64`.
   - Policy fields `update_cores: Option<i32>` / `update_memory_mb: Option<i32>` require
     cast to `u32`/`u64` before passing to `set_*_config_resources`.
7. Determine final target values: use `update_cores.unwrap_or(original_cores)` and
   `update_memory_mb.unwrap_or(original_memory_mb)` (all in their native API types at this point).
8. Persist `proxmox_resource_scaling_record` with `scale_status = "scaling"`,
   `restore_status = "pending"`, and `vm_type = mapping.proxmox_type` via `tenant_db()`.
   Writing before the API call captures `original_cores`/`original_memory_mb` so that a
   Controller crash mid-scaling leaves a record with `scale_status = "scaling"` — the
   post-update hook treats this identically to `"scaled"` and attempts restore.
9. Stream "Scaling VM resources to {cores} cores / {memory_mb} MB…\n" to `output_tx`.
10. Call `client.set_qemu_config_resources()` or `client.set_lxc_config_resources()`.
    - On success: update record `scale_status = "scaled"`. Stream success line.
    - On error: update record `scale_status = "failed"`, `restore_status = "skipped"`,
      `error_message = err.to_string()`. `tracing::warn!(...)`. Return early — best-effort.
      (`"skipped"` because scaling never succeeded; the post-update hook already
      gates restore on `scale_status` being `"scaled"` or `"scaling"`.)

**`finalize_post_update_hook` logic:**

```rust
let store = DbProxmoxProtectionStore { db: ctx.controller.tenant_db().db() };
```

1. Call `policy_store::load_scaling_record(ctx.controller.tenant_db().db(), update_history_id)`.
   If no record, or `scale_status` is not `"scaled"` or `"scaling"` → return `Ok(())`.
   (`"scaling"` means a crash occurred mid-scale; attempt restore anyway — a redundant
   restore is harmless.)
2. Look up the mapping directly by `record.mapping_id` via
   `ProxmoxHostMapping::find_by_id(record.mapping_id).one(db)` (not via
   `store.load_host_mapping(tenant_id, host_id)` — keying on the mapping's primary key
   is safer because the host mapping may have been re-pointed since the update started).
   Call `store.load_plugin_config_payload(record.tenant_id, record.plugin_config_id)` +
   deserialize `ProxmoxConfig`. If either lookup fails, return `Err`.
3. `ProxmoxClient::new(&proxmox_cfg)?`
4. Use `record.vm_type` to determine the restore call:
   - `"qemu"`: `client.set_qemu_config_resources(node, vmid, original_cores, original_memory_mb)`
   - `"lxc"`: `client.set_lxc_config_resources(node, vmid, original_cores, original_memory_mb)`
   - Proxmox applies `cores` and `memory` config fields atomically within a single PUT
     request; there is no partial-apply risk between the two fields.
   - On success: update record `restore_status = "restored"`. Return `Ok(())`.
   - On error: update record `restore_status = "restore_failed"`,
     `error_message = err.to_string()`. Then:

     ```rust
     tracing::warn!(
         update_history_id = %ctx.update_history_id,
         mapping_id = %record.mapping_id,
         vm_type = %record.vm_type,
         scaled_cores = record.scaled_cores,
         scaled_memory_mb = record.scaled_memory_mb,
         original_cores = record.original_cores,
         original_memory_mb = record.original_memory_mb,
         error = %err,
         "Proxmox resource restore failed — VM still running at scaled resources"
     );
     ```

     Attempt notification via
     `ctx.notification_ops.send_transactional_email(&ctx.tenant_db, ...)`.
     (`send_transactional_email` requires `&TenantDb` as its first argument;
     `ctx.tenant_db` is constructed by the Wave 5 dispatch wrapper from
     `db` + `record.tenant_id` and stored in the context.)
     Notification failure is silently ignored. Return `Err(err)` so the dispatch
     wrapper logs the failure at the update-id level.

### `PluginMeta` impl

```rust
impl PluginMeta for ControllerUpdateHookPlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::from_static("infrastructure_proxmox")
    }
}
```

### `plugin.rs` changes

```rust
fn __proxmox_create_controller_update_hook(
    config: &CatalogConfig,
) -> Result<Arc<dyn ControllerUpdateHook>> {
    crate::resource_scaling::ControllerUpdateHookPlugin::create(config)
}
```

Add to `declare_plugin!` invocation:

```rust
controller_update_hook: __proxmox_create_controller_update_hook,
```

Add `pub(crate) mod resource_scaling;` to `lib.rs`.

### `policy_store.rs` additions

Add `upsert_scaling_record` and `load_scaling_record` free fns accepting
`&DatabaseConnection` for the new `proxmox_resource_scaling_record` entity. These follow
the same pattern as `upsert_protection_audit` / `load_protection_audit` already in
`policy_store.rs`.

No new `load_host_mapping` free fn is needed — `resource_scaling.rs` uses
`DbProxmoxProtectionStore.load_host_mapping()` directly (same as `update_protection.rs`).

### `reset_tenant_data` extension

The `reset_tenant_data` callback (registered via `declare_plugin!`) must delete all rows
from `proxmox_resource_scaling_records` where `tenant_id = $tenant_id`, alongside the
existing deletion of protection/audit/override rows. This ensures tenant data cleanup
remains complete after this wave ships.

### Acceptance

- `cargo check --all-features` clean.
- Unit tests in `resource_scaling.rs`:
  - Pre-update hook returns `Ok(())` when no `update_cores`/`update_memory_mb` configured.
  - Pre-update hook returns `Ok(())` when no host mapping found.
  - Pre-update hook returns `Ok(())` when QEMU hotplug absent; no scaling record written.
  - Pre-update hook returns `Ok(())` on Proxmox API failure (scale error path), with
    record written as `scale_status = "failed"`.
  - Post-update hook returns `Ok(())` when no record exists.
  - Post-update hook sets `restore_status = "restored"` on success.

---

## Wave 5: Dispatch integration

**Goal:** Wire the hook into the update execution path, maintaining the specified ordering
(protection first on pre; hook first on post).

### `update_dispatch.rs` changes

**New `QueryUpdateHookController`:**

```rust
struct QueryUpdateHookController {
    tenant_db: TenantDb,
}

#[cfg(feature = "plugin-ops")]
impl UpdateHookController for QueryUpdateHookController {
    fn tenant_db(&self) -> &TenantDb {
        &self.tenant_db
    }
}
```

Construction follows the same pattern as `QueryUpdateProtectionController` after
boundary-hardening Wave 2 (clone pool handle, pass tenant id).

**New public functions:**

```rust
/// Run the pre-update hook (resource scaling). Called after protection. Never fails.
pub async fn prepare_pre_update_hook(
    db: &DatabaseConnection,
    hook: Option<Arc<dyn ControllerUpdateHook>>,
    target: &ValidatedUpdateTarget,
    output_tx: Option<UnboundedSender<Vec<u8>>>,
)

/// Run the post-update hook (resource restore). Called before protection finalization.
/// Returns Err if restore failed; callers log and swallow.
pub async fn finalize_post_update_hook(
    db: &DatabaseConnection,
    hook: Option<Arc<dyn ControllerUpdateHook>>,
    notification_ops: &dyn NotificationOps,
    record: &update_history::Model,
) -> Result<()>
```

### Call site changes

`AppState` already implements `PluginOps`; adding `ControllerUpdateHookOps` to `PluginOps`
makes `state.controller_update_hook()` available without any `AppState`-specific changes.

**Pre-update hook call sites** — add `prepare_pre_update_hook` after every existing
`prepare_pre_update_protection` call. Locations:

- `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` (WebSocket dispatch)
- `crates/ui/web-api-queries/src/queries/update_orchestrator.rs` (orchestrator dispatch)
- `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs` (batch dispatch)

Pattern (same at all three sites):

```rust
// After prepare_pre_update_protection ...
prepare_pre_update_hook(
    state.db(),
    state.controller_update_hook(),
    &target,
    output_tx.clone(),
).await; // returns () — never blocks the Update
```

**Post-update hook — `finalize_post_update_best_effort`** (in `updates.rs`):

```rust
async fn finalize_post_update_best_effort(state: &Arc<AppState>, record: &update_history::Model) {
    // Hook first (scale down) — finalize_post_update_hook returns Err on restore failure
    if let Err(error) = crate::queries::update_dispatch::finalize_post_update_hook(
        state.db(),
        state.controller_update_hook(),
        state.plugin_ops(),  // &dyn NotificationOps via PluginOps supertrait
        record,
    ).await {
        tracing::warn!(error = %error, update_id = %record.id, "post-update hook failed");
    }
    // Then protection finalization
    if let Err(error) = crate::queries::update_dispatch::finalize_post_update(
        state.db(),
        state.controller_update_protection(),
        record,
    ).await { ... }
}
```

The `finalize_post_update_hook` dispatch function constructs a
`TenantDb::new(db.clone(), record.tenant_id)` and passes it into
`UpdateHookPostContext::new(...)` as the `tenant_db` field, so the hook
implementation can call `ctx.notification_ops.send_transactional_email(&ctx.tenant_db, ...)`.

Apply the same pre/post-hook pattern to every update-finalization path — this is the
set of paths that must call `finalize_post_update_hook` so that crash-recovered updates
are also restored:

- `finalize_post_update_with_recovery_timeout_best_effort` in `updates.rs`
- The corresponding finalization path in `update_orchestrator.rs`
- `update_batches/dispatch.rs` and `update_batches/mod.rs`
- `controller-runtime/src/lib.rs` — the crash-recovery finalization path that runs when
  a Controller restarts and discovers in-flight updates. This is the path that closes
  the crash-safety loop: without it, `scale_status = "scaling"` / `restore_status =
  "pending"` records written before a crash will never be acted on.

The implementation of Wave 5 must audit all call sites of `finalize_post_update` (or
equivalent) in the codebase and confirm each one also calls `finalize_post_update_hook`.

Note: `prepare_pre_update_hook` in `update_dispatch.rs` is a best-effort wrapper that
never propagates errors (the hook always returns `Ok(())` for pre-update failures).
`finalize_post_update_hook` in `update_dispatch.rs` propagates `Err` from the hook so the
caller can log the failure at the update-id level, then swallows it.

### Acceptance

- `cargo check --all-features` clean.
- Existing `finalize_post_update_best_effort` tests unbroken.
- Pre-update order verified by integration test: protection called before hook when both
  registered; hook called alone when only hook registered.
- Post-update order verified: hook called before protection finalization.
- `cargo test --all-features` passes.

---

## Full Acceptance Criteria

| Check | Expected |
| --- | --- |
| `proxmox_protection_defaults` schema | `update_cores`, `update_memory_mb` columns present, nullable |
| `proxmox_protection_item_overrides` schema | same |
| `proxmox_resource_scaling_records` table | all columns present |
| `ControllerUpdateHook` trait | in `roles.rs`, no plugin-named types |
| `UpdateHookController` trait | in `roles.rs`, only `tenant_db()` |
| `ControllerUpdateHookOps` | in `plugin_ops.rs`, default `None` impl |
| `declare_plugin!` all existing plugin crates | compile with new `controller_update_hook: None` default |
| Pre-update order | protection before hook |
| Post-update order | hook before protection finalization |
| Scale-up failure | Update proceeds; record written with `scale_status = "failed"`; `restore_status = "skipped"` |
| Restore failure | `restore_status = "restore_failed"`; notification attempted; `PostUpdateOutcome` unchanged |
| QEMU hotplug absent | scaling skipped; no scaling record written |
| LXC | no hotplug check; scaling attempted |
| `cargo check --all-features` | clean |
| `cargo clippy --all-targets --all-features` | clean |
| `cargo test --all-features` | passes |
