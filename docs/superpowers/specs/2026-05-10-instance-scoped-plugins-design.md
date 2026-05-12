# Instance-Scoped Plugins — Design

**Status:** Draft (spec) — 2026-05-10 **Owner:** TBD **Related:** [CONTEXT.md](../../../CONTEXT.md) (Plugin, Plugin Scope, Instance-Scoped Plugin),
planned ADR `docs/adr/0006-instance-scoped-plugins.md`

## 1. Goal

Introduce a new flavor of plugin — **Instance-Scoped Plugin** — whose enable/disable state and instance-wide configuration are managed exclusively by
Operators with the `ManageGlobalSettings` permission ("instance owners"). When a plugin is instance-scoped and disabled, tenant Operators see no
evidence of its existence (no entry in plugin-type listings, no surfaces, no SSE events, no admin UI rows). When enabled, tenant Operators interact
with it through the existing `plugin_type_settings` mechanism for per-tenant behavior switches.

The first (and initially only) instance-scoped plugin is **`enhancement_dashboard_icons`**, converted from its current tenant-scoped form. It is
**disabled by default** for both fresh and upgraded installs.

### Success criteria

- A `PluginScope` enum exists on `PluginDescriptor`; existing plugins implicitly default to `Tenant`.
- `dashboard-icons` is `Instance`-scoped, disabled by default, invisible to tenant Operators until an instance owner enables it from the Plugin
  Configs settings tab.
- Toggling enable/disable persists in a new `instance_plugin_setting` table; the controller reads the table at boot to decide whether to construct
  each instance-scoped plugin's singleton (and spawn its background tasks).
- Hot-reload is **deferred** but not precluded — runtime reads of the snapshot remain a viable future change.
- Single visibility predicate centralizes the "is this plugin visible to this user" check; route handlers, surface registry, and SSE filter all
  consult it.
- Quality gates (`cargo fmt`, `cargo clippy --all-features -- -D warnings`, `cargo test --all-features`, `cargo deny check`, frontend
  `lint`/`format:check`/`check`/`build`/`test`, `markdownlint`) pass.

## 2. Domain model changes

### `PluginScope` enum (new)

In `crates/plugins/infrastructure/core/src/descriptor.rs`:

```rust
/// Who manages a plugin's enable state and instance-wide configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PluginScope {
    /// Default — managed per tenant via plugin_configs / plugin_type_settings.
    Tenant,
    /// Managed by instance owners (ManageGlobalSettings); when disabled,
    /// tenant Operators see no evidence the plugin exists.
    Instance,
}

impl Default for PluginScope {
    fn default() -> Self { Self::Tenant }
}

impl std::fmt::Display for PluginScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tenant => write!(f, "tenant"),
            Self::Instance => write!(f, "instance"),
        }
    }
}
```

`#[non_exhaustive]` per the project's wire-safe-enum coding standard. (Not a wire enum — no `Other(String)` catch-all needed; this is a
build-time-only descriptor field.)

### `InstanceConfigOps` slot (new)

```rust
/// Operations for the instance-wide configuration blob owned by an
/// Instance-Scoped Plugin. Optional — instance-scoped plugins may have no
/// configurable knobs beyond the enable toggle.
#[non_exhaustive]
pub struct InstanceConfigOps {
    pub form_schema: fn() -> Vec<FormFieldDescriptor>,
    pub sample: fn() -> serde_json::Value,
    pub validate: fn(&serde_json::Value) -> Result<(), PluginConfigValidationError>,
}
```

`#[non_exhaustive]` because the struct is constructed by individual plugin crates as `static` literals; future field additions must not be breaking
changes. (Sibling `ConfigOps` and `TypeSettingsOps` are macro-internal and not similarly constrained today.)

### `PluginDescriptor` extension

```rust
pub struct PluginDescriptor {
    // ── existing fields ──
    pub type_id: &'static str,
    pub display_name: &'static str,
    pub family: PluginFamily,
    pub config_model: ConfigModel,
    // ...

    // ── new ──
    pub scope: PluginScope,
    pub instance_config: Option<&'static InstanceConfigOps>,
}
```

### Invariants (enforced at catalog build)

- `scope == Tenant` ⇒ `instance_config.is_none()`. If a tenant-scoped descriptor declares `instance_config`, the catalog build fails with a
  `PluginError::UnsupportedOperation` describing the contradiction.
- `scope == Instance` allows any combination: kill-switch only (both `instance_config` and `type_settings` absent), instance-config only,
  type-settings only, or both.

These invariants cannot be enforced at compile time because `PluginDescriptor` carries function pointers (no const generics over fn-pointer presence).
Runtime enforcement at `CatalogBuild` is the chosen point — failures abort controller boot, surfaced via `tracing::error!` and a non-recoverable
`Result`.

### Existing `PluginDescriptor` struct literals (compatibility)

Every direct `PluginDescriptor { ... }` struct literal must add `scope: PluginScope::Tenant, instance_config: None` (or appropriate values). Affected
sites confirmed today:

- `crates/plugins/infrastructure/core/src/catalog.rs` (test fixtures around lines 600–940)
- `crates/plugins/infrastructure/registry/src/test_support.rs` (helper fixtures around lines 110–170)
- Any other plugin crate constructing `PluginDescriptor` directly (not through `declare_plugin!`)

Workspace lints (`warnings = "deny"`) make missing fields a hard build error — caught immediately on the first `cargo check`.

### `declare_plugin!` macro

Extend `crates/plugins/infrastructure/core/src/macros.rs` to accept two new optional fields:

```rust
declare_plugin!(MyPlugin, MyConfig, "my_type_id", {
    display_name: "My Plugin",
    family: PluginFamily::Enhancement,
    config_model: ConfigModel::None,
    scope: PluginScope::Instance,                 // optional, defaults Tenant
    instance_config: &MY_INSTANCE_CONFIG_OPS,     // optional
    type_settings: true,
    roles: [SoftwareItemLifecycle],
    software_item_lifecycle: create_my_lifecycle,
});
```

#### Macro grammar diff

The macro currently uses a token-tree pattern with `$(, name: $value:expr )?` arms (see e.g. `type_settings: $ts_marker` handling at `macros.rs:266`).
Add two analogous optional arms in the macro's `@parse` grammar:

```rust
$(, scope: $scope:expr )?
$(, instance_config: $instance_cfg:expr )?
```

The expansion of the static `DESCRIPTOR` initializer (around `macros.rs:176-280`) gains two new fields populated either from the user-supplied value
or from the defaults. Mirror the existing `type_settings` initializer pattern: a default-initialized field gets unconditionally set first, then
optionally overwritten when the user supplied a value:

```rust
let mut rc = PluginDescriptor {
    // ... existing fields ...
    scope: PluginScope::Tenant,
    instance_config: None,
    // ... existing fields ...
};
$( rc.scope = $scope; )?
$( rc.instance_config = Some($instance_cfg); )?
```

Defaults: `scope: PluginScope::Tenant`, `instance_config: None`. All existing `declare_plugin!` invocations remain untouched.

## 3. Data model

### New table: `instance_plugin_setting`

Stores enable state and instance-wide configuration for instance-scoped plugins. One row per `plugin_type_id`. Row absence ⇒ plugin defaults to
disabled and config = `{}`.

```sql
CREATE TABLE instance_plugin_setting (
    plugin_type_id TEXT        PRIMARY KEY,
    enabled        BOOLEAN     NOT NULL DEFAULT FALSE,
    config         JSON        NOT NULL DEFAULT '{}',
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

No FK to a plugin catalog (the catalog is a static Rust registry, not a table — same constraint as the existing `plugin_type_setting` table).

### SeaORM entity

In `crates/shared/db/src/entity/instance_plugin_setting.rs`:

```rust
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "instance_plugin_setting")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub plugin_type_id: String,
    pub enabled: bool,
    #[sea_orm(column_type = "Json")]
    pub config: serde_json::Value,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

Notes (mirroring `crates/shared/db/src/entity/plugin_type_setting.rs`):

- **No `Eq` derive.** `serde_json::Value` does not implement `Eq`; only `PartialEq`. Existing entities with `Value` columns (`plugin_type_setting`,
  `plugin_config`, `notification_log`, `global_setting`) all omit `Eq` for this reason.
- **`#[sea_orm(column_type = "Json")]`** is required on the `config` field — without it SeaORM stores the value as TEXT, which fails on SQLite type
  checks.

Wired into `entity/mod.rs` re-exports.

### Migration

File: `crates/shared/db/src/migration/mYYYYMMDD_NNNNNN_create_instance_plugin_setting.rs`. The date prefix **must be ≥ the latest existing migration
prefix on `main`** at implementation time (today's tip is `m20260430_000003`). Using an earlier prefix breaks migration ordering — SeaORM applies
migrations strictly by `mod.rs` declaration order, and the lexicographic naming is the only mnemonic for chronology.

Two registration sites both required, otherwise the migration silently never runs:

1. `mod mYYYYMMDD_NNNNNN_create_instance_plugin_setting;` declaration in `crates/shared/db/src/migration/mod.rs`.
2. `Box::new(mYYYYMMDD_NNNNNN_create_instance_plugin_setting::Migration)` appended to the `Migrator::migrations()` `vec!` in the same file.

Standard `MigrationTrait` with `up`/`down`. No seed row. `down` drops the table.

### `RawSettings` / `global_settings` — no extension

Per the locked decision, the existing `global_settings` table is **not** extended. `SettingKey` remains closed and typed. Instance-scoped plugin state
lives in its own table.

## 4. Backend changes

### Crate impact summary

| Crate                                         | Change                                                                                                                                                                                                     |
| --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `uptrakit-plugin-infrastructure-core`         | New `PluginScope`, `InstanceConfigOps`; descriptor extension; macro extension; catalog build invariant + gating                                                                                            |
| `uptrakit-plugin-infrastructure-registry`     | No structural change; tests update                                                                                                                                                                         |
| `uptrakit-shared-db`                          | New `instance_plugin_setting` entity + migration                                                                                                                                                           |
| `uptrakit-web-api-queries`                    | New `instance_plugin_settings` query module + `InstancePluginSnapshot::load_at_boot(db)` helper (lives here, not in `web-api-auth` — keeps DB query operations in the queries crate)                       |
| `uptrakit-web-api-types`                      | New DTOs (`InstancePluginSummary`, `InstancePluginDetail`, `SetInstancePluginEnabledRequest`, `UpsertInstancePluginConfigRequest`) with `Validate` impls                                                   |
| `uptrakit-web-api-auth`                       | No change (no new `SettingKey` variants, no DB queries)                                                                                                                                                    |
| `uptrakit-web-api`                            | New `routes/instance_plugins.rs` (4 handlers); update `routes/plugin_configs.rs::list_plugin_types` and `routes/plugin_type_settings.rs` to call visibility predicate; surface registry filter; SSE filter |
| `uptrakit-audit-log`                          | Two new `AuditActionType` constants: `INSTANCE_PLUGIN_TOGGLED`, `INSTANCE_PLUGIN_CONFIG_UPSERTED`                                                                                                          |
| `uptrakit-plugin-enhancement-dashboard-icons` | Descriptor diff (see §7)                                                                                                                                                                                   |
| `uptrakit-controller` (boot path)             | Read snapshot before catalog build; pass into `CatalogConfig`                                                                                                                                              |

### Catalog build gating

`CatalogConfig` is a shared boundary type passed into singleton constructors (`CreateEnhancementFn`, `CreateTransportFn`) and into per-agent runtime
contexts (`agent-ssh-runtime`, `agent_infra`). Widening it with a DB-loaded runtime map would couple it to controller-only state.

Instead, pass the instance-plugin gating map as a **separate argument** to the catalog build entry point. Three concrete code changes (load-bearing —
not optional):

**1. New typed wrapper in `crates/plugins/infrastructure/core/src/catalog.rs`:**

```rust
#[derive(Default, Debug, Clone)]
pub struct InstancePluginStates(BTreeMap<&'static str, bool>);

impl InstancePluginStates {
    pub fn from_pairs<I>(pairs: I) -> Self
        where I: IntoIterator<Item = (&'static str, bool)>
    {
        Self(pairs.into_iter().collect())
    }
    pub fn enabled(&self, type_id: &str) -> bool { self.0.get(type_id).copied().unwrap_or(false) }
    pub fn all_disabled() -> Self { Self::default() }
}
```

**2. Extend `PluginCatalog::new` (current signature at `catalog.rs:79`) to accept a third argument:**

```rust
impl PluginCatalog {
    pub fn new(
        descriptors: Vec<&'static PluginDescriptor>,
        config: CatalogConfig,
        instance_states: InstancePluginStates,    // NEW
    ) -> Result<Self> { ... }
}
```

`PluginCatalog` gains a private `instance_states: InstancePluginStates` field stored alongside its existing
`descriptors`/`transports`/`lifecycle_plugins`/etc. fields. Every existing call site of `PluginCatalog::new` is updated to pass
`InstancePluginStates::all_disabled()` (tests) or the loaded boot snapshot (controller).

**3. Skip singleton construction inside the per-descriptor loop (currently at `catalog.rs:97-222`):**

For each descriptor with `scope == Instance`, check `instance_states.enabled(descriptor.type_id)` **before** entering any of the role-creation blocks
(transport, software*item_lifecycle, notification, controller_update*\*). If disabled:

- Skip provider availability check.
- Skip every `if let Some(create) = desc.roles.<slot>` block — do not call any `create()` factory.
- Log once at `info!` with `plugin = type_id, scope = "instance", enabled = false, "skipping construction"`.
- Still record the descriptor in `descriptors` (so `descriptor_for(type_id)` works for the visibility predicate at admin boundaries — instance owners
  must be able to see disabled plugins).

For `scope == Tenant` descriptors: behavior unchanged.

**4. New method on `PluginMetadataOps` trait** (`plugin_ops.rs:83`):

```rust
pub trait PluginMetadataOps: Send + Sync + 'static {
    // ... existing methods ...
    fn instance_enabled(&self, id: &PluginTypeId) -> bool;
}
```

Default implementation in the `PluginCatalog`-implementing block: returns `instance_states.enabled(id.as_str())` for `Instance`-scoped plugins;
returns `true` for `Tenant`-scoped plugins (semantically: "tenant plugins are always 'instance-enabled' because there's no instance-level kill
switch"). Test fixture impls (`registry/src/test_support.rs`) override accordingly.

The controller boot path populates `InstancePluginStates` from `instance_plugin_setting` rows **before** the catalog is constructed, which is
**before** the web-api starts. Order: open DB → run migrations → load `InstancePluginStates` → build catalog → start web-api.

For each descriptor with `scope == Instance`:

1. Look up `instance_plugin_states[descriptor.type_id]`. Absent ⇒ disabled.
2. If disabled: skip singleton construction, skip background-task spawn, skip global-provider availability check. Log once at `info!` with
   `plugin = type_id, scope = "instance", enabled = false, "skipping construction"`.
3. If enabled: proceed exactly as today (provider check, singleton construction, background spawn).

For descriptors with `scope == Tenant`: behavior unchanged.

The `PluginOps` registry exposes `descriptor.scope` and a new method `instance_enabled(type_id) -> bool` (returns `true` for tenant-scoped plugins;
for instance-scoped, returns the snapshot value at boot time).

### Web-API routes

New module `crates/ui/web-api/src/routes/instance_plugins.rs`. All four endpoints gated by `CanManageGlobalSettings`.

```text
GET    /api/v1/instance-plugins
       → 200 Vec<InstancePluginSummary>
       → For every descriptor with scope == Instance: returns
         { plugin_type, display_name, enabled, has_instance_config,
           instance_config_form_fields, type_settings_form_fields,
           current_config, updated_at }

GET    /api/v1/instance-plugins/{plugin_type}
       → 200 InstancePluginDetail
       → 404 if plugin_type unknown OR scope != Instance

PUT    /api/v1/instance-plugins/{plugin_type}/enabled
       Body: SetInstancePluginEnabledRequest { enabled: bool }
       → 200 InstancePluginSummary
       → Emits INSTANCE_PLUGIN_TOGGLED audit event
       → Note in response body: "restart_required": true

PUT    /api/v1/instance-plugins/{plugin_type}/config
       Body: UpsertInstancePluginConfigRequest { config: serde_json::Value }
       → 200 InstancePluginSummary
       → 400 if descriptor's instance_config = None
       → 400 if config fails InstanceConfigOps::validate
       → Emits INSTANCE_PLUGIN_CONFIG_UPSERTED audit event
```

OpenAPI annotations follow the existing `plugin_type_settings.rs` pattern (`extensions(("x-required-permission" = json!("manage_global_settings")))`).

### DTOs (uptrakit-web-api-types)

```rust
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InstancePluginSummary {
    pub plugin_type: PluginTypeId,
    pub display_name: String,
    pub enabled: bool,
    pub has_instance_config: bool,
    pub instance_config_form_fields: Vec<FormField>,
    pub type_settings_form_fields: Vec<FormField>,
    pub current_config: serde_json::Value,
    pub updated_at: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InstancePluginDetail { /* superset of Summary */ }

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetInstancePluginEnabledRequest { pub enabled: bool }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertInstancePluginConfigRequest { pub config: serde_json::Value }
```

Both request types implement `Validate`. `SetInstancePluginEnabledRequest` validation is trivial (always Ok). `UpsertInstancePluginConfigRequest`
validates that `config` is a JSON object (not null, not array, not scalar) — descriptor-level schema validation happens in the route handler against
`InstanceConfigOps::validate`.

### Snapshot loading

`InstancePluginSnapshot` is a `HashMap<String, InstancePluginRow>` (or dedicated typed wrapper) loaded by
`crates/ui/web-api-queries/src/instance_plugin_settings.rs::load_at_boot(db)`. Returns the entire table in one query. Stored in `AppState` and shared
with handlers.

**Concurrency.** The snapshot is mostly-read, occasionally-written. Two acceptable storage choices:

- **`arc_swap::ArcSwap<InstancePluginSnapshot>`** (already used in `crates/ui/web-api/src/global_providers/github.rs`) — read-optimized atomic swap,
  no lock contention.
- **`parking_lot::RwLock<Arc<InstancePluginSnapshot>>`** — short critical section, guard always dropped before any `.await` point.

**Forbidden:** `tokio::sync::RwLock` and `tokio::sync::Mutex`. Snapshot rule: "Use `parking_lot::Mutex` (workspace dependency) everywhere in async
code. Never use `std::sync::Mutex` or `tokio::sync::Mutex`."

Web-api routes read from `AppState` for the current state when responding to `GET` requests; writes go directly to the DB and update the in-memory
snapshot **for read-back consistency on the same request only** — they do **not** affect the running catalog (catalog is constructed once at boot —
see Out of Scope §11 for hot-reload deferral).

The catalog build receives a separate `InstancePluginStates` argument loaded once at controller startup. It is **decoupled** from the web-api
`AppState` snapshot (catalog is constructed before web-api starts).

### Audit emission

In `crates/shared/audit-log/src/action_type.rs`:

```rust
pub const INSTANCE_PLUGIN_TOGGLED:          RegisteredAuditAction = ...;
pub const INSTANCE_PLUGIN_CONFIG_UPSERTED:  RegisteredAuditAction = ...;
```

Emission shape mirrors `PLUGIN_TYPE_SETTINGS_*`: tenant scope is `None` (instance-level event), actor from `authenticated_user_audit_actor`, target
`("instance_plugin", plugin_type_id, plugin_type_id)`, details
details object:

```json
{
  "plugin_type": "...",
  "operation": "toggle" | "config_upsert",
  "previous_enabled": "bool (toggle only)",
  "new_enabled": "bool (toggle only)",
  "config_field_count": "usize (config_upsert only)"
}
```

Raw config never written to audit details.

## 5. Frontend changes

### Section in `frontend/src/routes/settings/PluginConfigsTab.svelte`

New `<SectionCard>` titled **"Instance Plugins"**, conditionally rendered when `canManageGlobalSettings`. Section ordering:

1. **Instance Plugins** (new — visible only to instance owners)
2. Configurations (existing)
3. Discovery Allowlist (existing)
4. Type Defaults (existing)

Section description: "Plugins managed at the instance level. Disabled plugins are invisible to tenant Operators. Changes take effect after the
controller restarts."

Row template:

| Column  | Source                                                                                                  |
| ------- | ------------------------------------------------------------------------------------------------------- |
| Plugin  | `display_name` (`StatusBadge` for the type id below)                                                    |
| State   | `StatusBadge tone="success" label="Enabled"` or `tone="neutral" label="Disabled"`                       |
| Actions | `Toggle` button (`Enable`/`Disable`); `Edit Settings` button (only when `has_instance_config === true`) |

Toggle action opens `<ConfirmDialog>` with copy: "Enable Dashboard Icons? Restart the controller to apply." Confirm calls
`setInstancePluginEnabled(plugin_type, true)`. Success shows `<Toast>`: "Dashboard Icons enabled. Restart the controller to apply."

**Persistent restart-pending indicator (v1 UX mitigation for restart drift).** Each `InstancePluginSummary` row carries `enabled: bool` (the
**stored** state). The catalog's *running* state is reflected by whether the descriptor appears in `/api/v1/plugin-types` for the instance owner — the
predicate at §6 returns `true` for instance owners regardless of `enabled`, so the running state must be conveyed separately. Add a sibling boolean
`running_enabled: bool` on `InstancePluginSummary`, populated from `PluginOps::instance_enabled(type_id)` at request time (this reflects the catalog
snapshot from controller boot — not the live DB row). When `summary.enabled !== summary.running_enabled`, render a
`<StatusBadge tone="warning" label="Pending restart">` next to the State badge. Tenant-facing endpoints do not expose `running_enabled` —
instance-owner UI only.

Edit Settings opens `<ModalShell>` reusing the existing `flattenConfig`/`unflattenConfig`/`requiredFieldErrors` helpers and the same
`<FormFieldRow>`/`<Input>`/`<Textarea>`/`<Checkbox>`/`<Select>` rendering loop already present for Type Defaults (lines ~1387–1457 of the existing
file). Save calls `upsertInstancePluginConfig(plugin_type, config)`.

State variables (Svelte 5 runes):

```ts
let instancePlugins: InstancePluginSummary[] = $state([]);
let instancePluginsLoading: boolean = $state(true);
let instancePluginsError: string | null = $state(null);
let editingInstancePluginType: string | null = $state(null);
let showInstancePluginConfigModal: boolean = $state(false);
let instancePluginToggleConfirm: {
  plugin_type: string;
  display_name: string;
  next_enabled: boolean;
} | null = $state(null);
let instancePluginFormValues: Record<string, string> = $state({});
let instancePluginFieldErrors: Record<string, string> = $state({});
```

Multi-select state would use `SvelteSet<string>` per the frontend AGENTS rule, but no batch operations are planned for v1 — single-row toggles only.

### `src/lib/api.ts` additions

```ts
export async function listInstancePlugins(): Promise<InstancePluginSummary[]>;
export async function setInstancePluginEnabled(
  pluginType: string,
  enabled: boolean,
): Promise<InstancePluginSummary>;
export async function upsertInstancePluginConfig(
  pluginType: string,
  config: Record<string, unknown>,
): Promise<InstancePluginSummary>;
```

All three follow the existing `fetch` wrapper convention; no direct `fetch()` calls.

### `src/lib/types.ts` additions

```ts
export interface InstancePluginSummary {
  plugin_type: string;
  display_name: string;
  enabled: boolean;
  has_instance_config: boolean;
  instance_config_form_fields: FormField[];
  type_settings_form_fields: FormField[];
  current_config: Record<string, unknown>;
  updated_at: string | null;
}
```

### Cross-section interaction

When `dashboard-icons` is **enabled** at the instance level, it appears in **both** sections:

- **Instance Plugins**: instance owner edits the kill switch + (future) instance config.
- **Type Defaults**: tenant Operators edit per-tenant type_settings (the existing `enabled: bool` opt-out).

This is intentional. Section descriptions disambiguate. For instance owners viewing the same plugin in both sections, no warning or cross-link is
shown in v1 — out of scope.

## 6. Permission and visibility model

### Visibility predicate (single helper)

```rust
// crates/ui/web-api/src/visibility.rs
pub fn is_plugin_visible_to_user(
    descriptor: &PluginDescriptor,
    snapshot: &InstancePluginSnapshot,
    user: &AuthenticatedUser,
) -> bool {
    match descriptor.scope {
        PluginScope::Tenant => true,
        PluginScope::Instance => {
            let enabled = snapshot.enabled(descriptor.type_id);
            enabled || user.has_permission(Permission::ManageGlobalSettings)
        }
        // PluginScope is #[non_exhaustive]; per project rule, external match
        // sites must include a wildcard arm with a tracing::warn! and a
        // documented safe fallback. Defaulting to visible avoids accidentally
        // hiding plugins from instance owners when a future scope variant is
        // introduced before this predicate is updated.
        _ => {
            tracing::warn!(
                plugin = descriptor.type_id,
                scope = %descriptor.scope,
                "unknown PluginScope variant; defaulting to visible",
            );
            true
        }
    }
}
```

Call sites:

| Call site                                                     | What it filters                                                                                     |
| ------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `routes/plugin_configs.rs::list_plugin_types`                 | Filter `Vec<PluginTypeInfo>`                                                                        |
| `routes/plugin_type_settings.rs::list_plugin_type_settings`   | Filter `Vec<PluginTypeSettingsResponse>`                                                            |
| `routes/plugin_type_settings.rs::get_plugin_type_settings`    | 404 when filter rejects                                                                             |
| `routes/plugin_type_settings.rs::upsert_plugin_type_settings` | 404 when filter rejects (no audit emission for filtered case — same shape as `unknown_plugin_type`) |
| `routes/plugin_type_settings.rs::delete_plugin_type_settings` | 404 when filter rejects (closes the existence-leak via 404-vs-204 differential)                     |
| Surfaces registry read path                                   | Filter `Vec<SurfaceResponse>` per request                                                           |

**SSE / `AdminEvent` filter — out of scope for v1.** `AdminEvent` variants today carry no plugin-origin field. `enhancement_dashboard_icons` does not
emit any `AdminEvent` itself (it's a `SoftwareItemLifecycle` hook that mutates `software_item.icon_url`; downstream events like `SoftwareItemUpdated`
are emitted by the central handler, not by the plugin, and are tenant-scoped already). When a future instance-scoped plugin emits its own admin
events, this section must be revisited — likely by tagging events with origin plugin id and applying the same predicate.

### Leakage vectors checklist (every future instance-scoped plugin must verify)

The visibility predicate covers HTTP routes and surfaces today. A plugin author promoting a plugin to `PluginScope::Instance` must walk every channel
where its existence could leak to tenants and confirm the plugin does not surface there, OR extend the predicate to cover that channel:

- [ ] **HTTP plugin-type/type-settings routes** — covered by predicate (this spec).
- [ ] **Surfaces registry** — covered by predicate (this spec).
- [ ] **`AdminEvent` SSE stream** — uncovered. Plugin must not emit `AdminEvent` directly. If it must, add origin-plugin tagging + predicate filter
      (out of scope until needed).
- [ ] **Agent-side runtime / wire protocol** — uncovered. Plugin must be controller-only (no agent role declarations, no wire events). Dashboard-icons
      satisfies this — `SoftwareItemLifecycle` is controller-side.
- [ ] **MQTT topics** — uncovered. Plugin must not publish to MQTT or appear in MQTT topic structure.
- [ ] **Audit log target rows** — instance-disabled plugins should never have produced audit rows (their singleton was never constructed).
      Pre-existing rows from before conversion may persist; tenant audit-log views may include them. Acceptable for v1; tenants seeing a stale
      `enhancement_dashboard_icons` audit row from before the conversion is a known limitation, not a leak of the *current* enabled state.
- [ ] **OpenAPI schema** — structural; plugin type ids do not appear in `utoipa`-generated route schemas as enum members. No leak.
- [ ] **Database tables a tenant can read** — `plugin_type_setting` rows are filtered by the list/get/delete handlers via the predicate; no other
      tenant-readable table references plugin type ids by string today.
- [ ] **Persisted side effects on tenant-readable rows** — the plugin's prior writes may remain visible after disable, even with no plugin-id
      reference. For `enhancement_dashboard_icons`: `software_item.icon_url` may hold `https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/...`
      URLs from previous enrichment. These URLs identify the upstream source unmistakably. **Known limitation, accepted for v1** — matches the locked
      decision in §7 ("existing icons untouched — no provenance to safely identify which icons came from this plugin"). Document in the
      dashboard-icons end-user doc and in the disable-confirmation copy. A future provenance column on `software_item.icon_url` would enable a clean
      wipe-on-disable path; out of scope here.

The plan-writing step must run this checklist for `enhancement_dashboard_icons` and document the result.

For consistency: 404 for "exists but invisible" matches today's "unknown plugin type" response — no information leak about existence.

### Permission

All four `instance_plugins` endpoints require `ManageGlobalSettings`. No new `Permission` variant. Future split (`ManageInstancePlugins`) is additive
— kept open but explicitly out of scope for this spec.

## 7. `dashboard-icons` conversion

### Descriptor diff

In `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`:

```diff
 declare_plugin!(DashboardIconsPlugin, DashboardIconsConfig, "enhancement_dashboard_icons", {
     display_name: "Dashboard Icons",
     family: PluginFamily::Enhancement,
     config_model: ConfigModel::None,
+    scope: PluginScope::Instance,
     type_settings: true,
     roles: [SoftwareItemLifecycle],
     software_item_lifecycle: create_dashboard_icons_lifecycle,
     global_provider_consumers: ["github"],
 });
```

`instance_config` is **not** declared (kill switch only for v1). Future work may add a `DashboardIconsInstanceConfig` (e.g. custom GitHub repo,
refresh interval).

### Behavior preserved

- `DashboardIconsConfig { enabled: bool }` (default `true`) **stays** in `config.rs` — it remains the per-tenant `type_settings` schema.
- `on_software_item_created` keeps its existing tenant `enabled` check (line ~48 in `plugin.rs`). When the instance is enabled and a tenant has not
  overridden, enrichment proceeds as today.
- Cache + background refresh loop continue to be created in `create_dashboard_icons_lifecycle` — but only when the catalog construction phase decides
  to call it (i.e., instance enabled).

### Behavior changed

- On fresh install: no `instance_plugin_setting` row exists → runtime treats as disabled → singleton not constructed → no background refresh → no
  enrichment.
- On upgrade: same — migration creates an empty table; no row is seeded; behavior matches fresh install.
- Existing tenant `plugin_type_setting` rows for `enhancement_dashboard_icons` are **left untouched**. They become moot until an instance owner
  enables the plugin, at which point they resume their previous role (per-tenant opt-out).
- Existing `software_item.icon_url` values set by previous enrichment remain in place. No provenance tracking exists, so a wipe is impossible without
  collateral damage. Note: these URLs (e.g. `https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/...`) are identifiable as dashboard-icons-origin
  by their host/path, so disabling the plugin does not retroactively erase tenant-visible evidence of past enrichment. Listed as a known limitation in
  the leakage vectors checklist (§6).

## 8. Migrations

Single migration file (date placeholder: replace with the actual date the migration is added to `main`):

```text
crates/shared/db/src/migration/m20260520_000001_create_instance_plugin_setting.rs
```

Standard SeaORM `MigrationTrait`:

- `up`: `CREATE TABLE instance_plugin_setting (...)` matching §3.
- `down`: `DROP TABLE instance_plugin_setting`.

No seed row. No data backfill. No touching of `plugin_type_setting`, `software_item`, or any other existing table.

## 9. Tests

### Backend

**`crates/plugins/infrastructure/core` — descriptor invariants**

- `tenant_scope_with_instance_config_fails_catalog_build` — synthesizes a descriptor with `scope = Tenant` and `instance_config = Some(...)`, expects
  `CatalogBuild` to return `PluginError::UnsupportedOperation`.
- `instance_scope_kill_switch_only_builds_successfully` — `scope = Instance`, `instance_config = None`, `type_settings = None`. Passes.
- `instance_scope_with_both_surfaces_builds_successfully` — `scope = Instance`, `instance_config = Some`, `type_settings = Some`. Passes.

**`crates/plugins/infrastructure/core::catalog` — gating**

- `instance_disabled_skips_singleton_construction` — boot the catalog with snapshot saying `enhancement_dashboard_icons: enabled = false`; assert no
  `SoftwareItemLifecycle` plugin is registered for that type id; no provider lookup performed.
- `instance_enabled_constructs_singleton` — same but `enabled = true`; assert the singleton appears in `software_item_lifecycle_plugins()`.

**`crates/ui/web-api/routes/instance_plugins`**

- `list_requires_manage_global_settings` — non-instance-owner gets 403.
- `list_returns_all_instance_scoped_plugins_with_state` — seeds two rows (one enabled, one disabled), expects both in response.
- `set_enabled_persists_and_audits` — toggles, asserts row updated and `INSTANCE_PLUGIN_TOGGLED` audit row written with correct
  `previous_enabled`/`new_enabled`.
- `set_enabled_for_unknown_plugin_returns_404` — unknown id.
- `set_enabled_for_tenant_scoped_plugin_returns_404` — known id but `scope = Tenant`.
- `upsert_config_for_kill_switch_only_plugin_returns_400` — descriptor's `instance_config = None`.
- `upsert_config_validates_against_instance_config_schema` — invalid payload returns 400 with validation reason; valid payload persists + audits.

**`crates/ui/web-api/routes/plugin_configs::list_plugin_types` and `plugin_type_settings::*`**

- `tenant_user_does_not_see_disabled_instance_plugin_in_plugin_types_list` — assert `enhancement_dashboard_icons` absent.
- `tenant_user_get_plugin_type_settings_for_disabled_instance_plugin_returns_404` — explicit GET returns 404.
- `tenant_user_put_plugin_type_settings_for_disabled_instance_plugin_returns_404` — PUT returns 404, no audit.
- `tenant_user_delete_plugin_type_settings_for_disabled_instance_plugin_returns_404` — DELETE returns 404 (closes existence-leak via 404-vs-204
  differential).
- `instance_owner_sees_disabled_instance_plugin_everywhere` — same calls as above, performed by instance owner, return 200.

**`crates/plugins/enhancements/dashboard-icons` — conversion**

- `descriptor_has_instance_scope` — assert `DESCRIPTOR.scope == PluginScope::Instance`.
- `type_settings_still_present_with_enabled_field` — assert `DESCRIPTOR.type_settings.is_some()` and the form schema still contains `enabled`.
- Existing tests in `plugin.rs::tests` continue to pass unchanged (the runtime hook behavior is unchanged when the lifecycle is constructed).

### Frontend

`frontend/src/routes/settings/PluginConfigsTab.test.ts`:

- `renders_instance_plugins_section_when_user_has_manage_global_settings`
- `does_not_render_instance_plugins_section_when_user_lacks_permission`
- `toggle_button_opens_confirm_dialog_with_restart_required_copy`
- `edit_button_disabled_when_has_instance_config_false`

Use existing test harness (Svelte Testing Library, `MockApi`).

### Quality gates

All locked from `docs/development/pr-process.md`:

- `cargo fmt --all`
- `cargo check --no-default-features --features db-sqlite`
- `cargo check --all-features`
- `cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`
- `cargo deny check`
- `markdownlint --config .markdownlint.json '**/*.md'`
- `cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build`

## 10. Documentation deliverables

Every doc affected by externally observable behavior, surface area, config, or architecture is listed; none are optional.

| Doc                                                                             | Status                   | Change                                                                                                                                                                                                                                                                                  |
| ------------------------------------------------------------------------------- | ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CONTEXT.md`                                                                    | **Done in this session** | Added Plugin Scope and Instance-Scoped Plugin entries; clarified Plugin entry to reference scopes.                                                                                                                                                                                      |
| `docs/adr/0006-instance-scoped-plugins.md`                                      | **New**                  | ADR covering: introduction of `PluginScope`, dedicated `instance_plugin_setting` table (vs reusing `global_settings`), restart-required toggle (vs hot-reload), `ManageGlobalSettings` reuse (vs new permission). Three architectural alternatives weighed; locked decisions justified. |
| `docs/development/plugin-guidelines.md`                                         | **Update**               | New section "Choosing a plugin scope": when to use `Tenant` vs `Instance`; the two-surface model (instance config vs type_settings); descriptor invariants; the restart-required toggle constraint.                                                                                     |
| `ARCHITECTURE.md`                                                               | **Update**               | One-paragraph mention in the Plugin section enumerating scopes and their lifecycle implications.                                                                                                                                                                                        |
| `website/public/docs/end-user/dashboard-icons`                                  | **Update**               | Note the plugin is now disabled by default and must be enabled by an instance owner from Settings → Plugin Configs → Instance Plugins. Document tenant-side opt-out (Type Defaults) is unchanged.                                                                                       |
| `docs/admin/instance-plugins.md` (or equivalent path under existing admin docs) | **New**                  | One short page for instance owners: how to find the section, how toggling works, the restart-required caveat, audit log entries to expect.                                                                                                                                              |
| OpenAPI schema                                                                  | **Auto-generated**       | New routes annotated with `utoipa` macros — no manual JSON edits. Confirmed via `cargo run -p uptrakit-controller -- openapi` (or whichever exporter exists) producing the four new paths.                                                                                              |

## 11. Out of scope

Explicitly **not** part of this spec:

- **Hot-reload of instance-plugin enable state.** Toggling requires controller restart in v1. The design decouples the catalog snapshot from the
  web-api snapshot precisely to keep this option open later — a follow-up can add a broadcast invalidation channel without re-architecting storage.
- **Multi-controller coordination.** Single-controller deployments only (matches today's testing).
- **Additional instance-scoped plugins.** Only `dashboard-icons` is converted. Other candidates (proxmox-helper-scripts? notification plugins? GitHub
  global provider?) are evaluated separately.
- **Splitting `ManageGlobalSettings` into a finer permission.** Reuse for v1; additive split later.
- **Dynamic plugin descriptors.** Plugins remain compile-time-static. The instance-plugin enable flag does not turn into a "plugin loader."
- **Provenance tracking on `software_item.icon_url`.** No column tracks which plugin set an icon. Existing icons are not wiped on `dashboard-icons`
  conversion.
- **UI restart-required indicator** beyond the per-action confirmation copy. No banner, no badge comparing "stored state" vs "running state." Out of
  scope for v1.
- **Batch operations on the Instance Plugins UI section.** Single-row toggles only.
- **`AdminEvent` SSE filtering by plugin origin.** Today `AdminEvent` variants carry no plugin-origin field. Dashboard-icons doesn't emit
  `AdminEvent`s directly — none of its enrichment work generates events that would leak its existence. When a future instance-scoped plugin produces
  its own admin events, this surface must be reconsidered (likely an `origin_plugin_type_id: Option<&str>` on `AdminEvent` variants + per-subscriber
  predicate filter at the SSE handler).

## 12. Open questions and risks

| Item                                                                                                                                                                                                                                                                                                                      | Mitigation                                                                                                                                                                                                                                               |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Restart UX is a sharp edge.** Operators may toggle and not realize a restart is needed; toggled state can drift silently from running state forever.                                                                                                                                                                    | Mitigated in v1 by: (1) confirm-dialog copy "Restart the controller to apply", (2) `running_enabled` field on `InstancePluginSummary` + persistent "Pending restart" badge in the UI when stored ≠ running. Hot-reload remains the proper fix; deferred. |
| **Descriptor invariant enforcement is runtime, not compile-time.** A misconfigured descriptor only fails at catalog build (controller boot).                                                                                                                                                                              | A test in the registry asserts every shipped descriptor passes the invariant check. CI catches it.                                                                                                                                                       |
| **No FK from `instance_plugin_setting.plugin_type_id` to a catalog table.** Stale rows can accumulate if a plugin is removed from the registry.                                                                                                                                                                           | Same constraint as `plugin_type_setting`. Acceptable. A future cleanup job can prune orphans (out of scope).                                                                                                                                             |
| **Two snapshots in flight (catalog snapshot at boot, web-api snapshot per request).** Read-after-write inside one request is consistent (route updates the in-memory snapshot before returning); read-after-write across requests on a different controller would not be (single-controller scope makes this moot today). | Documented in the ADR.                                                                                                                                                                                                                                   |
| **Writes to `instance_plugin_setting` while controller is mid-boot** could race with catalog build.                                                                                                                                                                                                                       | Catalog build occurs before the web-api accepts requests; no race window in practice.                                                                                                                                                                    |
| **Tenant Operator confusion** when an instance owner disables a plugin they were using — disappearance with no notification.                                                                                                                                                                                              | Audit log is the breadcrumb. UX improvement (e.g., admin-pushed notification) deferred.                                                                                                                                                                  |

## Snapshot conformance check

| Binding rule                                                       | Satisfied                                                                                                                                                                                              |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Extensible public enums `#[non_exhaustive]`                        | Yes (`PluginScope`, audit constants follow existing patterns).                                                                                                                                         |
| No `unwrap()`/`expect()`/`panic!()` in production code             | Spec'd uses `Result` everywhere; tests may use the project's standard test exemptions.                                                                                                                 |
| `rootcause::Report` for errors, `report!`/`bail!` macros           | All new error returns follow the pattern.                                                                                                                                                              |
| `tracing` macros with structured fields                            | All new log sites use structured `tracing` calls; no `log` crate.                                                                                                                                      |
| `Validate` trait on HTTP request types in `uptrakit-web-api-types` | Both new request DTOs implement `Validate`.                                                                                                                                                            |
| `parking_lot::Mutex` for sync locks                                | Snapshot held via `arc_swap::ArcSwap` (already in workspace, used by `web-api/global_providers/github.rs`) OR `parking_lot::RwLock<Arc<...>>`. `tokio::sync::*` locks **forbidden** per snapshot rule. |
| BEGIN IMMEDIATE for read-then-write SQLite transactions            | `set_enabled` reads previous value for audit `previous_enabled`, then writes — uses `BEGIN IMMEDIATE`.                                                                                                 |
| Conventional Commits for PR titles                                 | Implementation PRs to follow.                                                                                                                                                                          |
| Frontend rules (api.ts, shared components, TS strict, SvelteSet)   | Followed; new API calls in `src/lib/api.ts`, new section reuses existing components, no `Set` (no multi-select needed in v1).                                                                          |
| Quality gates all green                                            | Required for merge.                                                                                                                                                                                    |

No deviations from the snapshot are necessary.
