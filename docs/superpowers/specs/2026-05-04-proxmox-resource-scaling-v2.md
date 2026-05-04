# Proxmox Resource Scaling v2: Delta Mode + UI Surfaces

**Date:** 2026-05-04
**Status:** Spec
**Supersedes:** `2026-05-03-proxmox-resource-scaling-hook.md` (v1, fully implemented)

## Background

The v1 spec added absolute resource scaling: a pre-update hook temporarily sets a VM to a fixed
number of CPU cores and MB of RAM, then restores the original values after. UI configuration was
a Non-Goal in v1.

This spec adds three things:

1. **Delta scaling mode** — add +X cores / +Y MB RAM to current values rather than setting absolute
   targets. The restore path is identical: always revert to original values captured at pre-hook time.
2. **Explicit opt-in semantics** — a `scaling_mode` discriminant replaces the implicit
   `update_cores IS NOT NULL` heuristic. Both layers (global defaults + per-item overrides) default
   to `none`.
3. **UI surfaces** — the existing `proxmox.settings.update-protection` and
   `proxmox.software-item.update-protection` surfaces are **renamed** to `proxmox.settings.update-hooks`
   and `proxmox.software-item.update-hooks`, re-labelled "Proxmox Update Hooks", and extended with
   a "Resource Scaling" section alongside the existing protection section.

The v1 code also stored `update_cores` / `update_memory_mb` directly in the protection policy
tables (`proxmox_protection_defaults` / `proxmox_protection_item_overrides`). This spec moves
scaling config to **dedicated scaling tables** to keep protection and scaling concerns independent.
Old columns are migrated and dropped.

## Goals

1. Operators can choose `scaling_mode = delta` and supply `+delta_cores` / `+delta_memory_mb`,
   with the actual scale target computed as `current + delta` at hook time.
2. Both `absolute` and `delta` are opt-in per Software Item via the new UI surfaces; the default
   at both layers is `none`.
3. Mode is an exclusive discriminant: an item carries exactly one of `none` / `absolute` / `delta`.
4. Dimension values (cores / memory) are independently optional: `null` means "don't touch this
   dimension." At least one dimension must be set when mode ≠ `none`.
5. Per-item overrides support three states: **inherit** (no override row — defer to global),
   **disable** (row with `scaling_mode = 'none'` — opt out regardless of global), and
   **configure** (row with a real mode + dimension values).
6. Cross-mode field inheritance is gated by mode: if the resolved effective mode is `delta`, only
   `delta_cores` / `delta_memory_mb` cascade from global to item; `absolute_cores` /
   `absolute_memory_mb` are ignored.
7. The `proxmox_resource_scaling_records` table stores `scaling_mode_used` so restore logic and
   auditing never re-read policy at post-hook time.
8. All v1 crash-safety guarantees (persist record before API call; `scale_status = "scaling"` treated
   identically to `"scaled"` at restore time) are preserved.
9. The unmatch race is handled: if the host mapping no longer exists at post-hook time, write
   `restore_status = "skipped_mapping_deleted"` and log a warning rather than returning a hard error.

## Non-Goals

- UI display of current live VM resource values (no Proxmox round-trip at preload time).
- Soft cap / ceiling validation in uptrakit; Proxmox rejects out-of-range values at the API.
- Exposing `scale_status` in the Update History UI (separate feature).
- Hotplug compatibility indicator on the host mapping list (separate feature).
- Cleanup of stale scaling records (retained alongside update history, same as v1).

## Dependency

Boundary-hardening Wave 4 must be merged first (entity module paths referenced below assume
post-Wave-4 locations). This spec is otherwise independent.

---

## Wave 1: DB schema

### Migration A — `CreateProxmoxScalingDefaults`

Migration name: `"m20260504_000001_proxmox_scaling_defaults"`.

Create table `proxmox_scaling_defaults`:

| Column               | Type          | Constraint                 |
| -------------------- | ------------- | -------------------------- |
| `id`                 | `UUID`        | Primary key                |
| `tenant_id`          | `UUID`        | Not null                   |
| `plugin_config_id`   | `UUID`        | Not null                   |
| `scaling_mode`       | `VARCHAR(16)` | Not null, default `'none'` |
| `absolute_cores`     | `INT`         | Nullable                   |
| `absolute_memory_mb` | `INT`         | Nullable                   |
| `delta_cores`        | `INT`         | Nullable                   |
| `delta_memory_mb`    | `INT`         | Nullable                   |
| `created_at`         | `TIMESTAMP`   | Not null                   |
| `updated_at`         | `TIMESTAMP`   | Not null                   |

Unique constraint: `(tenant_id, plugin_config_id)`.

CHECK constraints (enforced at the DB layer; no-op on rows with NULL values):

- `absolute_cores >= 1 OR absolute_cores IS NULL`
- `absolute_memory_mb >= 1 OR absolute_memory_mb IS NULL`
- `delta_cores >= 1 OR delta_cores IS NULL`
- `delta_memory_mb >= 1 OR delta_memory_mb IS NULL`

### Migration B — `CreateProxmoxScalingItemOverrides`

Migration name: `"m20260504_000002_proxmox_scaling_item_overrides"`.

Create table `proxmox_scaling_item_overrides`:

| Column               | Type          | Constraint                 |
| -------------------- | ------------- | -------------------------- |
| `id`                 | `UUID`        | Primary key                |
| `tenant_id`          | `UUID`        | Not null                   |
| `software_item_id`   | `UUID`        | Not null                   |
| `plugin_config_id`   | `UUID`        | Not null                   |
| `scaling_mode`       | `VARCHAR(16)` | Not null, default `'none'` |
| `absolute_cores`     | `INT`         | Nullable                   |
| `absolute_memory_mb` | `INT`         | Nullable                   |
| `delta_cores`        | `INT`         | Nullable                   |
| `delta_memory_mb`    | `INT`         | Nullable                   |
| `created_at`         | `TIMESTAMP`   | Not null                   |
| `updated_at`         | `TIMESTAMP`   | Not null                   |

Unique constraint: `(software_item_id, plugin_config_id)`.

Same CHECK constraints as `proxmox_scaling_defaults`.

### Migration C — `MigrateProxmoxScalingFromProtectionTables`

Migration name: `"m20260504_000003_migrate_scaling_from_protection_tables"`.

Steps C.1–C.3 run within Migration C's own transaction. Migration D runs as a separate subsequent migration.
The two are not jointly atomic — if D fails on first boot it will retry on the next startup;
C's null-out step leaves the source columns in a coherent (all-null) state in the interim.

1. For each row in `proxmox_protection_defaults` where `update_cores IS NOT NULL OR update_memory_mb IS NOT NULL`:
   - INSERT into `proxmox_scaling_defaults` (`id`, `tenant_id`, `plugin_config_id`,
     `scaling_mode`, `absolute_cores`, `absolute_memory_mb`, `delta_cores`, `delta_memory_mb`,
     `created_at`, `updated_at`) VALUES (generate new UUID, `tenant_id` from source row,
     `plugin_config_id` from source row, `'absolute'`, `update_cores`, `update_memory_mb`,
     `NULL`, `NULL`, source `created_at`, source `updated_at`).
2. For each row in `proxmox_protection_item_overrides` where `update_cores IS NOT NULL OR update_memory_mb IS NOT NULL`:
   - INSERT into `proxmox_scaling_item_overrides` (`id`, `tenant_id`, `software_item_id`,
     `plugin_config_id`, `scaling_mode`, `absolute_cores`, `absolute_memory_mb`, `delta_cores`,
     `delta_memory_mb`, `created_at`, `updated_at`) VALUES (generate new UUID, all corresponding
     fields from source row, `'absolute'`, `update_cores`, `update_memory_mb`, `NULL`, `NULL`,
     source `created_at`, source `updated_at`).
3. Set `update_cores = NULL` and `update_memory_mb = NULL` on all rows in both source tables
   (SQLite: update to null rather than drop; columns will be dropped in Migration D).

Note on SQLite `DROP COLUMN`: SQLite 3.35.0+ supports `ALTER TABLE DROP COLUMN`. The project
targets SQLite ≥ 3.37 (JSON support requirement). Use `SchemaManager::drop_column` within the
migration. If the runtime SQLite is older, the migration will produce a schema error at startup
rather than silently missing the drop — this is the desired failure mode.

### Migration D — `DropProxmoxScalingColumnsFromProtectionTables`

Migration name: `"m20260504_000004_drop_scaling_columns_from_protection_tables"`.

Drop `update_cores` and `update_memory_mb` from both `proxmox_protection_defaults` and
`proxmox_protection_item_overrides`. These columns are now unused following Migration C.

### Migration E — `AddScalingModeUsedToScalingRecord`

Migration name: `"m20260504_000005_add_scaling_mode_used_to_scaling_record"`.

Add to `proxmox_resource_scaling_records`:

| Column              | Type          | Constraint                     |
| ------------------- | ------------- | ------------------------------ |
| `scaling_mode_used` | `VARCHAR(16)` | Not null, default `'absolute'` |

Default `'absolute'` preserves the semantic meaning of all existing v1 records (which were written
without a mode discriminant and used absolute values).

### SeaORM entity changes

**New entities** (in `crates/plugins/infrastructure/proxmox/src/entity/`):

- `proxmox_scaling_default.rs` — maps `proxmox_scaling_defaults`
- `proxmox_scaling_item_override.rs` — maps `proxmox_scaling_item_overrides`

Add both to `entity/mod.rs`.

**Updated entity**: `proxmox_resource_scaling_record.rs` — add `pub scaling_mode_used: String`.

**Updated entities**: `proxmox_protection_default.rs` and `proxmox_protection_item_override.rs` —
remove `pub update_cores: Option<i32>` and `pub update_memory_mb: Option<i32>`.

All migrations go into `controller_migration.rs` and are appended to the `migrations()` vec.

**Commit-ordering constraint**: the `proxmox_resource_scaling_record` entity change (adding
`scaling_mode_used: String`, Wave 1) and the `ScalingRecord` struct change (adding
`scaling_mode_used: ScalingMode`, Wave 2) must land in the **same commit**. The entity field is
read by `load_scaling_record` which constructs `ScalingRecord` — if the entity gains the column
before the struct does, the code fails to compile. Do not split these two changes across separate
commits.

### Acceptance

- `cargo check --all-features` clean.
- Migration integration tests (SQLite):
  - `proxmox_scaling_defaults` created with all expected columns and unique constraint.
  - `proxmox_scaling_item_overrides` created with all expected columns and unique constraint.
  - Migration C transfers rows correctly: protection table rows with non-null scaling values
    appear in scaling tables as `absolute` mode; null-only rows produce no scaling rows.
  - Migration D removes both columns from both protection tables.
  - Migration E adds `scaling_mode_used` to scaling records; existing rows read as `'absolute'`.

---

## Wave 2: `ScalingMode` type, policy structs, scaling store, and `resource_scaling.rs`

### `ScalingMode` enum

New file `crates/plugins/infrastructure/proxmox/src/scaling_mode.rs` (or inline in `policy_store.rs`
if small):

```rust
/// Effective scaling mode for a pre-update resource adjustment.
///
/// Not `#[non_exhaustive]` — this is an internal enum, not wire-protocol.
/// Not sent over any network boundary; no `Other(String)` needed.
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
            "absolute" => Ok(Self::Absolute),
            "delta" => Ok(Self::Delta),
            "none" => Ok(Self::None),
            _ => Err(()),
        }
    }
}
```

Call sites that load `scaling_mode_used` from the DB entity use:

```rust
let mode = entity.scaling_mode_used
    .parse::<ScalingMode>()
    .unwrap_or_else(|_| {
        tracing::warn!(value = %entity.scaling_mode_used, "unrecognised scaling_mode_used in DB; treating as None");
        ScalingMode::None
    });
```

`ScalingMode` is `Copy`. It does NOT use `#[non_exhaustive]` — it is internal-only and must be
exhaustively matched everywhere.

### New `ScalingPolicy` struct

In `policy_store.rs` (or a new `scaling_store.rs` — see below):

```rust
/// Effective scaling policy resolved for a software item update.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScalingPolicy {
    pub mode: ScalingMode,
    /// Absolute target CPU cores. Used when `mode == Absolute`. `None` = don't touch.
    pub absolute_cores: Option<i32>,
    /// Absolute target memory MB. Used when `mode == Absolute`. `None` = don't touch.
    pub absolute_memory_mb: Option<i32>,
    /// Delta CPU cores to add. Used when `mode == Delta`. `None` = don't touch.
    pub delta_cores: Option<i32>,
    /// Delta memory MB to add. Used when `mode == Delta`. `None` = don't touch.
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

    /// True when the policy will result in at least one API call.
    pub fn is_active(&self) -> bool {
        if self.mode == ScalingMode::None {
            return false;
        }
        // At least one dimension must be set for the policy to have any effect.
        match self.mode {
            ScalingMode::Absolute => self.absolute_cores.is_some() || self.absolute_memory_mb.is_some(),
            ScalingMode::Delta => self.delta_cores.is_some() || self.delta_memory_mb.is_some(),
            ScalingMode::None => false,
        }
    }
}
```

### `ProtectionPolicy` cleanup

Remove `update_cores: Option<i32>` and `update_memory_mb: Option<i32>` from `ProtectionPolicy`
(in `policy_store.rs`) and from `ProxmoxProtectionPolicyRecord` (in `protection_store.rs`). Remove
all cascade logic that read those fields from entities. Update `ProtectionPolicy::do_nothing()` and
all call sites.

### New scaling store functions

Add (to `policy_store.rs` or a dedicated `scaling_store.rs`):

```rust
/// Load the global scaling default for `(tenant_id, plugin_config_id)`.
/// Returns `ScalingPolicy::none()` if no row exists.
pub async fn load_scaling_global_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ScalingPolicy>

/// Load the per-item scaling override for `(software_item_id, plugin_config_id)`.
/// Returns `None` if no row exists (meaning: inherit global).
pub async fn load_scaling_item_override(
    db: &DatabaseConnection,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<Option<ScalingPolicy>>

/// Resolve the effective scaling policy for a software item.
///
/// Mode resolution: item override wins if present; otherwise global default.
/// Dimension cascade rule: only fields belonging to the resolved effective mode
/// participate in the item→global fallback. Cross-mode inheritance is forbidden.
///
/// Example:
///   global = { mode: Delta, delta_cores: Some(2), delta_memory_mb: None }
///   item   = { mode: Delta, delta_cores: None,    delta_memory_mb: Some(1024) }
///   result = { mode: Delta, delta_cores: Some(2), delta_memory_mb: Some(1024) }
///
///   global = { mode: Absolute, absolute_cores: Some(8), absolute_memory_mb: None }
///   item   = { mode: Delta, delta_cores: Some(2), delta_memory_mb: None }
///   result = { mode: Delta, delta_cores: Some(2), delta_memory_mb: None }
///   // global's absolute_* values are NOT inherited — different mode.
pub async fn resolve_effective_scaling_policy(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ScalingPolicy>

/// Upsert (insert-or-update) the global scaling default.
///
/// Uses `BEGIN IMMEDIATE` (SQLite read-then-write rule): reads the existing row
/// to determine insert vs update, then writes. No-op on Postgres.
pub async fn upsert_scaling_global_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    policy: &ScalingPolicy,
) -> Result<()>

/// Upsert the per-item scaling override.
///
/// Uses `BEGIN IMMEDIATE` for the same reason as `upsert_scaling_global_default`.
pub async fn upsert_scaling_item_override(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
    policy: &ScalingPolicy,
) -> Result<()>

/// Delete the per-item scaling override, reverting the item to global inheritance.
pub async fn delete_scaling_item_override(
    db: &DatabaseConnection,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<()>
```

All three functions must be modelled on `upsert_scaling_record` in the existing `policy_store.rs`
(which already uses `BEGIN IMMEDIATE`) rather than on `upsert_global_default` (which does not —
that is a pre-existing omission, not a pattern to copy).

`resolve_effective_scaling_policy` applies the cross-mode gate described in Goal 6: after
determining the effective mode (item wins over global), dimension values are cascaded only if
they belong to that mode. The function calls `load_scaling_item_override` then
`load_scaling_global_default` and merges field-by-field within the resolved mode.

Implementation note: a single LEFT JOIN query fetching both the item-override row and the
global-default row in one round-trip is preferred over two sequential selects, given this
function runs on the critical pre-update path. Use `QuerySelect::join` with a LEFT OUTER join
keyed on `plugin_config_id`.

### `resource_scaling.rs` changes

Replace all uses of `policy.update_cores` / `policy.update_memory_mb` with the new
`ScalingPolicy` fields. Key logic changes:

**Pre-update hook (`prepare_pre_update_hook`):**

1. Replace `DbProxmoxProtectionStore.load_effective_policy()` scaling field access with
   `resolve_effective_scaling_policy(tenant_db, tenant_id, software_item_id, plugin_config_id).await`.
2. Early-return gate: `if !policy.is_active() { return; }`.
3. Delta mode: after reading `original_cores` / `original_memory_mb` from the Proxmox API:

   ```rust
   let target_cores: Option<u32> = match policy.mode {
       ScalingMode::Absolute => policy.absolute_cores.map(|v| v as u32),
       ScalingMode::Delta => policy.delta_cores.map(|v| (original_cores as i64 + v as i64).max(1) as u32),
       ScalingMode::None => unreachable!("is_active() returned true"),
   };
   let target_memory_mb: Option<u64> = match policy.mode {
       ScalingMode::Absolute => policy.absolute_memory_mb.map(|v| v as u64),
       ScalingMode::Delta => policy.delta_memory_mb.map(|v| (original_memory_mb as i64 + v as i64).max(1) as u64),
       ScalingMode::None => unreachable!("is_active() returned true"),
   };
   ```

   Delta fields are validated ≥ 1 at the API layer and enforced by DB CHECK constraints
   (Migration A/B). Delta scaling is scale-up only. Remove the `max(1)` clamp — it is dead code
   given the constraints above and its presence implies negative input is possible when it is not.
   If a value < 1 somehow reaches the hook (indicating a DB integrity violation), log
   `tracing::error!` and return early rather than silently clamping.

4. When writing `scaled_cores` / `scaled_memory_mb` to `ScalingRecord` (which stores `i32` /
   `i64`), cast with a saturating guard:

   ```rust
   scaled_cores: i32::try_from(target_cores_u32).unwrap_or(i32::MAX),
   scaled_memory_mb: i64::try_from(target_memory_mb_u64).unwrap_or(i64::MAX),
   ```

   `i32::MAX` (~2B cores) and `i64::MAX` will never be reached by any real Proxmox host; the
   Proxmox API will reject the call first. The guard prevents arithmetic overflow only.
5. The `ScalingRecord` struct (in `policy_store.rs`) gains a `scaling_mode_used: ScalingMode` field.
   On persist, write `scaling_mode_used.as_str()` to the DB column.

**Post-update hook (`finalize_post_update_hook`):**

Restore logic is unchanged — it reads `original_cores` / `original_memory_mb` from the record
only, never from policy.

Add the unmatch-race guard: if `ProxmoxHostMapping::find_by_id(record.mapping_id).one(db).await`
returns `None`, update the record to `restore_status = "skipped_mapping_deleted"`, log a warning,
and return `Ok(())`.

### Acceptance

- `cargo check --all-features` clean.
- Unit tests in `resource_scaling.rs`:
  - Pre-hook returns immediately when `is_active()` is false.
  - Delta mode: `delta_cores = +2`, `original_cores = 4` → `target = 6`.
  - Delta mode: `delta_cores = None` → only memory is scaled; cores PUT param absent.
  - Delta DB integrity violation guard: `delta_cores = 0` (bypassed CHECK constraint) reaching
    hook → `tracing::error!` + early return, no API call made.
  - `scaling_mode_used` written to record.
  - Post-hook with missing mapping writes `skipped_mapping_deleted`, returns `Ok`.
- All existing `resource_scaling.rs` unit tests pass unchanged.
- `ProtectionPolicy` has no `update_cores` / `update_memory_mb` fields; all callers compile.

---

## Wave 3: Surface backend

### Renamed surfaces

Rename surface IDs and labels in `plugin.rs`:

| Old surface ID                            | New surface ID                       | Old label                 | New label            |
| ----------------------------------------- | ------------------------------------ | ------------------------- | -------------------- |
| `proxmox.settings.update-protection`      | `proxmox.settings.update-hooks`      | Proxmox Update Protection | Proxmox Update Hooks |
| `proxmox.software-item.update-protection` | `proxmox.software-item.update-hooks` | Proxmox Update Protection | Proxmox Update Hooks |

All `const` string literals and tests in `surfaces.rs` that reference the old IDs must be updated.

### New action constants

In `surfaces.rs`:

```rust
const SURFACE_SETTINGS_UPDATE_HOOKS: &str = "proxmox.settings.update-hooks";
const SURFACE_SOFTWARE_ITEM_UPDATE_HOOKS: &str = "proxmox.software-item.update-hooks";

const ACTION_PRELOAD_SCALING_GLOBAL_DEFAULTS: &str = "preload-scaling-global-defaults";
const ACTION_SAVE_SCALING_GLOBAL_DEFAULTS: &str = "save-scaling-global-defaults";
const ACTION_PRELOAD_SCALING_ITEM_OVERRIDES: &str = "preload-scaling-item-overrides";
const ACTION_SAVE_SCALING_ITEM_OVERRIDES: &str = "save-scaling-item-overrides";
```

Add four new entries to `surface_actions()` and four new arms to `resolve_controller_surface_action`.

### New action handlers

**`handle_preload_scaling_global_defaults`**

Signature:

```rust
async fn handle_preload_scaling_global_defaults(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxScopeSelectionRequest,
) -> std::result::Result<serde_json::Value, String>
```

Response JSON:

```json
{
  "plugin_config_id": "<uuid>",
  "scaling_mode": "none" | "absolute" | "delta",
  "absolute_cores": null | <number>,
  "absolute_memory_mb": null | <number>,
  "delta_cores": null | <number>,
  "delta_memory_mb": null | <number>
}
```

When no plugin config is selected or found, return `scaling_mode: "none"` and all dimensions null.

**`handle_save_scaling_global_defaults`**

Request struct `ProxmoxScalingGlobalDefaultsSaveRequest`:

```rust
pub struct ProxmoxScalingGlobalDefaultsSaveRequest {
    pub plugin_config_id: Uuid,
    pub scaling_mode: String,
    pub absolute_cores: Option<i32>,
    pub absolute_memory_mb: Option<i32>,
    pub delta_cores: Option<i32>,
    pub delta_memory_mb: Option<i32>,
}
```

Implement `Validate` on this type. Validation rules:

- `scaling_mode` must be one of `"none"` / `"absolute"` / `"delta"`.
- When mode = `"absolute"`: each non-null dimension ≥ 1; at least one dimension must be non-null.
- When mode = `"delta"`: each non-null dimension ≥ 1; at least one dimension must be non-null.
- When mode = `"none"`: all dimension fields ignored.
- Cross-mode field constraint: when mode = `"absolute"`, `delta_cores` / `delta_memory_mb` must be
  null (rejected if present). When mode = `"delta"`, `absolute_cores` / `absolute_memory_mb` must
  be null. This prevents ambiguous saves.

Calls `upsert_scaling_global_default(...)`.

**`handle_preload_scaling_item_overrides`**

Response JSON:

```json
{
  "software_item_id": "<uuid>",
  "plugin_config_id": "<uuid>",
  "scaling_mode": "inherit" | "none" | "absolute" | "delta",
  "absolute_cores": null | <number>,
  "absolute_memory_mb": null | <number>,
  "delta_cores": null | <number>,
  "delta_memory_mb": null | <number>
}
```

`scaling_mode` here is a 4-state field (note: `"inherit"` is only used on the per-item surface,
not in `ScalingMode` enum or global surface):

- No override row → `"inherit"`, all dimensions null.
- Row with `scaling_mode = "none"` → `"none"` (item explicitly opts out).
- Row with real mode → `"absolute"` or `"delta"`, populate dimension fields.

**`handle_save_scaling_item_overrides`**

Request struct `ProxmoxScalingItemOverridesSaveRequest`:

```rust
pub struct ProxmoxScalingItemOverridesSaveRequest {
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    /// "inherit" | "none" | "absolute" | "delta"
    pub scaling_mode: String,
    pub absolute_cores: Option<i32>,
    pub absolute_memory_mb: Option<i32>,
    pub delta_cores: Option<i32>,
    pub delta_memory_mb: Option<i32>,
}
```

Logic:

- `scaling_mode = "inherit"` → call `delete_scaling_item_override(...)`.
- `scaling_mode = "none"` → call `upsert_scaling_item_override(...)` with `ScalingPolicy::none()`.
- `scaling_mode = "absolute"` or `"delta"` → validate dimensions (same rules as global save),
  call `upsert_scaling_item_override(...)`.

Validation (via `Validate`): `scaling_mode` must be one of `"inherit"` / `"none"` /
`"absolute"` / `"delta"`. When `"absolute"` or `"delta"`, dimension validation rules from the
global save handler apply.

### Surface manifest changes in `plugin.rs`

**`proxmox_settings_update_hooks_surface()`** (renamed from `proxmox_settings_update_protection_surface`):

Rename the function and update surface ID / label. Extend the `root_node` with a second
`SurfaceNode::Form` node for the scaling section, with title "Resource Scaling":

```rust
SurfaceNode::Section {
    title: None,
    children: vec![
        SurfaceNode::Callout { ... },          // existing backup-target callout
        SurfaceNode::Section {
            title: Some("Update Protection".to_string()),
            children: vec![
                SurfaceNode::Form {
                    interaction_id: InteractionId::new("save-global-defaults")...,
                },
            ],
        },
        SurfaceNode::Section {
            title: Some("Resource Scaling".to_string()),
            children: vec![
                SurfaceNode::Form {
                    interaction_id: InteractionId::new("save-scaling-global-defaults")...,
                },
            ],
        },
    ],
}
```

Add four new `InteractionDescriptor` entries for the scaling actions, following the exact same
field layout as the existing protection interactions. Permissions:

- `preload-scaling-global-defaults`: `Permission::ManageGlobalSettings`
- `save-scaling-global-defaults`: `Permission::ManageGlobalSettings`
- `preload-scaling-item-overrides`: `Permission::ViewSoftware`
- `save-scaling-item-overrides`: `Permission::UpdateSoftware`

The `save-scaling-global-defaults` interaction's `form_ui` declares the fields:

- `plugin_config_id` — select, RestApi source (same as protection form)
- `scaling_mode` — select: `none` / `absolute` / `delta`; default `none`
- `absolute_cores` — number, min 1, visible when `scaling_mode = absolute`
- `absolute_memory_mb` — number, min 1, visible when `scaling_mode = absolute`
- `delta_cores` — number, min 1, visible when `scaling_mode = delta`
- `delta_memory_mb` — number, min 1, visible when `scaling_mode = delta`

The `save-scaling-item-overrides` interaction's `form_ui` declares:

- `software_item_id` — hidden, required
- `plugin_config_id` — hidden, required (or select if multi-config)
- `scaling_mode` — select: `inherit` / `none` / `absolute` / `delta`; default `inherit`
- `absolute_cores` — number, min 1; visible when `scaling_mode = absolute`
- `absolute_memory_mb` — number, min 1; visible when `scaling_mode = absolute`
- `delta_cores` — number, min 1; visible when `scaling_mode = delta`
- `delta_memory_mb` — number, min 1; visible when `scaling_mode = delta`

This 4-state `scaling_mode` field collapses the former `scaling_override_state` + `scaling_mode`
pair into a single selector. It maps directly to `FormVisibleWhen`'s single-field condition with
no compound logic required — each dimension field's visibility is fully determined by one field.

`proxmox_software_item_update_hooks_surface()` (renamed from `proxmox_software_item_update_protection_surface`) follows the same pattern.

### `surfaces.rs` request/response structs

New request types (add to the request structs section in `surfaces.rs`):

- `ProxmoxScalingGlobalDefaultsSaveRequest` (see above)
- `ProxmoxScalingItemOverridesSaveRequest` (see above)

Both implement `Validate` and are deserialized from surface action params.

### `reset_tenant_data` extension

The `reset_tenant_data` callback must delete all rows from `proxmox_scaling_defaults` and
`proxmox_scaling_item_overrides` where `tenant_id = $tenant_id`.

`proxmox_resource_scaling_records` deletion was already added by the v1 spec; confirm it is
present in the existing `reset_tenant_data` implementation before considering this complete.

### Acceptance

- `cargo check --all-features` clean.
- Surface action test: `surface_actions_include_host_and_policy_actions_with_permissions` updated to
  include four new scaling actions with correct permissions.
- Handler unit tests:
  - `preload_scaling_global_defaults` returns `scaling_mode: "none"` when no row exists.
  - `save_scaling_global_defaults` validates mode + at least one dimension; rejects cross-mode fields.
  - `preload_scaling_item_overrides` returns `scaling_mode: "inherit"` when no row exists.
  - `preload_scaling_item_overrides` returns `scaling_mode: "none"` for a row with `scaling_mode = none`.
  - `save_scaling_item_overrides` with `scaling_mode = "inherit"` deletes the override row.
  - `save_scaling_item_overrides` with `scaling_mode = "none"` writes a `none` row.
  - `save_scaling_item_overrides` with `scaling_mode = "absolute"` validates dimensions.
- `cargo test --all-features` passes.

---

## Wave 4: Surface frontend (form UI verification)

The surface framework renders forms generically via `SchemaForm` / `SurfaceWorkflow`. No new
Svelte components are required. This wave verifies that the `FormVisibleWhen` conditions declared
in Wave 3 surface descriptors produce the correct conditional field behaviour in the UI.

### Verification checklist

Run the app locally (`cargo run` + `cd frontend && npm run dev`) and exercise both surfaces:

**Settings → Proxmox Update Hooks → Resource Scaling section:**

- Default state: `scaling_mode = none`, no dimension fields visible.
- Switching to `absolute`: `absolute_cores` and `absolute_memory_mb` fields appear; delta fields hidden.
- Switching to `delta`: `delta_cores` and `delta_memory_mb` fields appear; absolute fields hidden.
- Saving `absolute` with no dimensions filled: server returns validation error, form shows it.
- Saving `delta` with `delta_cores = 0`: server returns validation error (must be ≥ 1).
- Saving `delta` with `delta_cores = 2`, `delta_memory_mb = null`: succeeds; partial config preserved.
- Round-trip: reload page, verify saved values appear pre-populated.

**Software Item → Proxmox Update Hooks → Resource Scaling section:**

- Default state: `scaling_mode = inherit`, no dimension fields visible.
- Switching to `none`: no dimension fields visible; save succeeds; reload shows `none`.
- Switching to `absolute`: `absolute_cores` and `absolute_memory_mb` appear; delta fields hidden.
- Switching to `delta`: `delta_cores` and `delta_memory_mb` appear; absolute fields hidden.
- Saving `absolute` with no dimensions filled: server returns validation error.
- Saving `absolute` with `absolute_cores = 4`: succeeds; reload shows `absolute` + pre-populated value.
- `inherit` save deletes override row; subsequent reload shows `inherit` again (not global values).

### Frontend change: surface ID reference updates

Search all frontend TypeScript and Svelte files for any hardcoded references to
`proxmox.settings.update-protection` or `proxmox.software-item.update-protection` and update to
the new IDs. Based on pre-spec exploration, no such hardcoded references exist in the frontend —
surfaces are discovered dynamically. Confirm this during implementation.

### Acceptance

- `cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build` passes.
- All visual verification steps above pass.
- PR includes screenshots of both surfaces showing the scaling section with each mode selected.

---

## Full Acceptance Criteria

| Check                                       | Expected                                                                                      |
| ------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `proxmox_scaling_defaults` table            | All columns present; `(tenant_id, plugin_config_id)` unique                                   |
| `proxmox_scaling_item_overrides` table      | All columns present; `(software_item_id, plugin_config_id)` unique                            |
| v1 data migration                           | Existing `update_cores`/`update_memory_mb` values appear in scaling tables as `absolute` mode |
| Old columns dropped                         | `update_cores`, `update_memory_mb` absent from both protection tables                         |
| `scaling_mode_used` in records              | Existing records default to `'absolute'`; new records write actual mode                       |
| `ScalingMode` enum                          | `Copy`, exhaustive, no `#[non_exhaustive]`, no `Other(String)`                                |
| Cross-mode inheritance                      | Dimension cascade gated by resolved effective mode                                            |
| Per-item four-state `scaling_mode`          | `inherit` → delete row; `none` → write none row; `absolute`/`delta` → write mode row          |
| Unmatch race                                | `mapping_id` not found → `restore_status = "skipped_mapping_deleted"`, `Ok(())`               |
| Delta integrity guard                       | DB CHECK enforces ≥ 1; hook logs error and bails if violated value reaches it                 |
| Surface IDs                                 | Renamed to `proxmox.settings.update-hooks` / `proxmox.software-item.update-hooks`             |
| Surface labels                              | "Proxmox Update Hooks"                                                                        |
| Form conditional fields                     | Mode-gated fields visible/hidden correctly                                                    |
| Validation: cross-mode fields               | Sending absolute fields with delta mode (or vice versa) rejected                              |
| `reset_tenant_data`                         | Deletes rows from both new scaling tables                                                     |
| `cargo check --all-features`                | Clean                                                                                         |
| `cargo clippy --all-targets --all-features` | Clean                                                                                         |
| `cargo test --all-features`                 | Passes                                                                                        |
| Frontend build                              | `npm run build` passes                                                                        |
| PR screenshots                              | Both surfaces shown with each scaling mode selected                                           |

## Documentation Deliverables

- **`docs/development/coding-standards.md`**: Note the three-state override pattern
  (`inherit`/`disable`/`configure`) as the canonical model for per-item policy overrides — it
  replaces any implicit "null = inherit" anti-pattern in future surfaces.
- **No `CONTEXT.md` update required** — no new domain terms; "Resource Scaling" is already
  described in the v1 spec and is implied by existing "Update" / "Software Item" glossary entries.
- **No `plugin-guidelines.md` update required** — `ControllerUpdateHook` trait introduced in v1
  is unchanged.
