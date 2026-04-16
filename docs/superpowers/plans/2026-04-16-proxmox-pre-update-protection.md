# Proxmox Pre-Update Protection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add controller-side Proxmox snapshot/backup protection before
updates, surface global and per-software-item policy through shared surfaces,
and expose generic recovery guidance in update history without leaking
Proxmox details outside the plugin.

**Architecture:** Add one controller-owned singleton protection role to the
plugin catalog, thread it through immediate dispatch, batch initial dispatch,
queued promotion, WebSocket completion, reconnect recovery, and controller
startup rollout cleanup, and keep all Proxmox policy resolution, target
caching, execution, and audit persistence inside the Proxmox plugin. Add one
new shared-surface slot for software-item tabs, then implement Proxmox-owned
settings and software-item surfaces on top of the existing shared-surface
runtime.

**Tech Stack:** Rust, SeaORM migrations, shared surfaces, Svelte, uptrakit-plugin-infrastructure-core, uptrakit-plugin-infrastructure-registry

---

## File Map

| File | Change |
| --- | --- |
| `crates/plugins/infrastructure/core/src/roles.rs` | Add `ControllerUpdateProtection` trait and generic context/outcome types |
| `crates/plugins/infrastructure/core/src/descriptor.rs` | Add singleton creator slot for controller protection |
| `crates/plugins/infrastructure/core/src/catalog.rs` | Construct/store controller protection singleton |
| `crates/plugins/infrastructure/core/src/plugin_ops.rs` | Add accessor trait for controller protection singleton |
| `crates/plugins/infrastructure/core/src/macros.rs` | Support `controller_update_protection:` in `declare_plugin!` |
| `crates/plugins/infrastructure/core/src/lib.rs` | Re-export new role/types |
| `crates/plugins/infrastructure/registry/src/lib.rs` | Re-export new role/accessor types for controller code |
| `crates/plugins/infrastructure/registry/src/test_support.rs` | Update test `RoleCreators` literals for the new singleton field |
| `crates/core/controller/src/main.rs` | Thread dispatch context through startup rollout cleanup promotion |
| `crates/ui/web-api/src/app_state.rs` | Store singleton access path in `AppState` |
| `crates/ui/web-api-queries/src/queries/update_dispatch.rs` | Add `DispatchContext`, pre-protection invocation, generic history field persistence |
| `crates/ui/web-api-queries/src/queries/update_triggers.rs` | Thread `DispatchContext` into immediate update trigger path |
| `crates/ui/web-api-queries/src/queries/update_batches/mod.rs` | Run protection in initial batch dispatch path and pass context into later promotion |
| `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs` | Run protection in queued promotion path, loop FIFO after controller-side failure |
| `crates/ui/web-api/src/actions/software_items.rs` | Pass dispatch context into single-item trigger action |
| `crates/ui/web-api/src/actions/update_batches.rs` | Pass dispatch context into batch trigger action |
| `crates/ui/web-api/src/test_harness/mod.rs` | Update `AppState` test harness construction for new accessor/storage wiring |
| `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs` | Pass dispatch context into service-triggered update entrypoints |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs` | Update test `update_history::ActiveModel` literals for new shared fields |
| `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` | Update `PluginOps` test doubles for the new accessor trait |
| `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` | Invoke post-update finalization on completion and reconnect/rollout cleanup |
| `crates/ui/web-api/src/surface_registry.rs` | Map `software_item.tabs` into the software page slot registry |
| `crates/shared/db/src/entity/update_history.rs` | Add generic protection/recovery columns |
| `crates/shared/db/src/migration/mod.rs` | Register shared update-history migration |
| `crates/shared/db/src/migration/m20260416_000001_update_history_protection.rs` | Add shared protection/recovery columns |
| `crates/shared/web-api-types/src/update_history.rs` | Add generic response fields |
| `crates/ui/web-api-queries/src/queries/update_history.rs` | Populate response fields |
| `crates/ui/cli/src/commands/history.rs` | Update response/test literals for new history fields |
| `frontend/src/lib/types.ts` | Add generic protection/recovery fields |
| `frontend/src/routes/history/+page.svelte` | Render generic protection summary and recovery hint |
| `crates/plugins/infrastructure/proxmox/src/client.rs` | Add storage listing, snapshot, backup, and task polling calls |
| `crates/plugins/infrastructure/proxmox/src/discovery.rs` | Persist backup-target cache during sync |
| `crates/plugins/infrastructure/proxmox/src/controller_migration.rs` | Add Proxmox-owned policy/cache/audit tables |
| `crates/plugins/infrastructure/proxmox/src/matching.rs` | Preserve deterministic host-to-Proxmox mapping for protection policy |
| `crates/plugins/infrastructure/proxmox/src/plugin.rs` | Register controller protection singleton and new surfaces |
| `crates/plugins/infrastructure/proxmox/src/surfaces.rs` | Add settings/software-item surfaces and interactions |
| `crates/plugins/infrastructure/proxmox/src/lib.rs` | Export new controller modules |
| `crates/plugins/infrastructure/proxmox/src/update_protection.rs` | New controller-side Proxmox protection role implementation |
| `crates/plugins/infrastructure/proxmox/src/policy_store.rs` | New Proxmox-owned policy/cache persistence helpers |
| `crates/shared/surfaces/src/slot.rs` | Add `SLOT_SOFTWARE_ITEM_TABS` |
| `crates/shared/surfaces/tests/ids.rs` | Cover new slot shape |
| `frontend/src/routes/software/[id]/+page.svelte` | Mount `software_item.tabs` with `software_item_id` base params |
| `frontend/src/routes/software/[id]/software-detail.test.ts` | New route test for software-item surface tabs |
| `frontend/src/routes/hosts/[id]/host-detail.test.ts` | Reuse slot-rendering pattern as reference; no production changes expected |

## Task 1: Add the controller-side protection singleton role to plugin core

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs`
- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/plugins/infrastructure/core/src/plugin_ops.rs`
- Modify: `crates/plugins/infrastructure/core/src/macros.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`
- Modify: `crates/plugins/infrastructure/registry/src/lib.rs`
- Modify: `crates/plugins/infrastructure/registry/src/test_support.rs`

- [ ] **Step 1: Add the role and generic payload types**

Add a new singleton role in `roles.rs` with a shape equivalent to:

```rust
#[async_trait]
pub trait ControllerUpdateProtection: PluginMeta {
    async fn prepare_pre_update_protection(
        &self,
        ctx: &ControllerProtectionContext,
    ) -> Result<ControllerProtectionDecision>;

    async fn finalize_post_update(
        &self,
        ctx: &ControllerPostUpdateContext,
    ) -> Result<PostUpdateOutcome>;
}

#[non_exhaustive]
#[derive(Clone)]
pub struct ControllerProtectionContext<'a> {
    pub db: &'a (dyn std::any::Any + Send + Sync),
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
}

#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ControllerProtectionDecision {
    pub attempted: bool,
    pub succeeded: bool,
    pub protection_status: Option<String>,
    pub protection_summary: Option<String>,
}

#[non_exhaustive]
#[derive(Clone)]
pub struct ControllerPostUpdateContext<'a> {
    pub db: &'a (dyn std::any::Any + Send + Sync),
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
    pub final_status: uptrakit_shared_types::UpdateStatus,
}

#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct PostUpdateOutcome {
    pub recovery_hint: Option<String>,
}
```

Keep the DB handle in the context as `dyn Any`, matching the existing
`SurfaceActionContext` pattern, so the role can downcast to
`DatabaseConnection` without forcing an unconditional `sea-orm` dependency
through every consumer of the core crate.

- [ ] **Step 2: Add the descriptor/catalog plumbing**

Extend descriptor/catalog wiring so this role behaves like existing singletons:

```rust
pub type CreateControllerProtectionFn =
    fn(&CatalogConfig) -> crate::error::Result<Arc<dyn roles::ControllerUpdateProtection>>;

pub struct RoleCreators {
    // existing fields...
    pub controller_update_protection: Option<CreateControllerProtectionFn>,
}
```

In `catalog.rs`, construct and store:

```rust
controller_update_protection: Option<Arc<dyn ControllerUpdateProtection>>,
```

and expose an accessor trait method in `plugin_ops.rs`, then re-export it in
`lib.rs` and the registry crate.

Be explicit about the compile blockers here:

- update every `RoleCreators { ... }` struct literal, including the
  `declare_plugin!` initializer in `macros.rs` and the static test descriptors
  in `catalog.rs`
- reject duplicate singleton registration in `PluginCatalog::new(...)` rather
  than silently overwriting the first instance
- extend the `PluginOps` supertrait and blanket impl so `Arc<dyn PluginOps>` in
  `AppState` can expose the new accessor without downcasting

- [ ] **Step 3: Extend `declare_plugin!` for the new singleton**

Add a new singleton key in `macros.rs`:

```rust
controller_update_protection: $controller_protection_fn:expr
```

and ensure `RoleCreators` initialization sets it exactly once.

Do **not** add a new `PluginCapability` variant in this change. This role is a
controller-internal catalog singleton accessed through the new accessor, not a
capability-filtered REST/API surface, so `__accumulate_role_caps!`,
`__expand_role_caps!`, and `PluginCapability` should stay unchanged unless a
real capability query appears later.

- [ ] **Step 4: Verify the core crate still compiles**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-registry
```

Expected: both crates compile with the new singleton role available through the registry re-exports.

Also add one focused catalog test in this task for duplicate
`controller_update_protection` registration so the singleton rejection behavior
is locked in where it is introduced, not only in the final verification sweep.

## Task 2: Add shared update-history fields for generic protection and recovery data

**Files:**

- Modify: `crates/shared/db/src/entity/update_history.rs`
- Create: `crates/shared/db/src/migration/m20260416_000001_update_history_protection.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`
- Modify: `crates/shared/web-api-types/src/update_history.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_history.rs`
- Modify: `crates/ui/cli/src/commands/history.rs` (tests/response fixtures)
- Modify: `frontend/src/lib/types.ts`

- [ ] **Step 1: Add nullable entity fields**

Extend the entity model with shared generic columns:

```rust
pub pre_update_protection_status: Option<String>,
pub pre_update_protection_summary: Option<String>,
pub recovery_hint: Option<String>,
```

- [ ] **Step 2: Add the migration**

Create a new shared DB migration that adds the three nullable columns to `update_history` in both SQLite and PostgreSQL-compatible form.

Minimum schema contract:

```rust
ColumnDef::new(UpdateHistory::PreUpdateProtectionStatus).text().null()
ColumnDef::new(UpdateHistory::PreUpdateProtectionSummary).text().null()
ColumnDef::new(UpdateHistory::RecoveryHint).text().null()
```

Register it in `migration/mod.rs` immediately after the latest shared migration.

- [ ] **Step 3: Extend API and query mapping**

Add matching response fields in `uptrakit_web_api_types::update_history::UpdateHistoryResponse` and map them in `build_response(...)` inside `queries/update_history.rs`.

Use:

```rust
pub pre_update_protection_status: Option<String>,
pub pre_update_protection_summary: Option<String>,
pub recovery_hint: Option<String>,
```

- [ ] **Step 4: Extend frontend wire types**

Add the same optional fields in `frontend/src/lib/types.ts` so the history page can render them without any plugin-specific typing.

Also update exhaustive `UpdateHistoryResponse` and `update_history::ActiveModel`
test literals touched by these new fields so all-target builds do not fail on
missing struct members. Prefer `..Default::default()` where that keeps future
field additions cheaper.

- [ ] **Step 5: Verify schema/API compilation**

Run:

```bash
cargo check -p uptrakit-shared-db --features "migration db-sqlite"
cargo check -p uptrakit-web-api-types -p uptrakit-web-api-queries
```

Expected: entity, migration, and response mapping changes compile together.

## Task 3: Thread `DispatchContext` through immediate, queued, and cleanup update flows

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_triggers.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_batches/mod.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`
- Modify: `crates/ui/web-api/src/actions/software_items.rs`
- Modify: `crates/ui/web-api/src/actions/update_batches.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/ui/web-api/src/app_state.rs`

- [ ] **Step 1: Introduce `DispatchContext`**

Add a shared dispatch helper type in `update_dispatch.rs`:

```rust
pub struct DispatchContext<'a> {
    pub notifier: &'a dyn ServiceNotifier,
    pub protection: Option<Arc<dyn ControllerUpdateProtection>>,
}
```

Add helpers that:

- build `ControllerProtectionContext { db, ... }` and call
  `prepare_pre_update_protection(...)`
- write `pre_update_protection_status` and `pre_update_protection_summary`
- fail the current row before dispatch when protection fails

Be explicit about that failure helper: when controller-side protection fails
before any agent message, update the current row to `Failed`, set
`completed_at`, set `output` / `output_bytes` to the generic failure message,
and persist the protection status/summary in the same write so the row never
remains stuck in `Pending`.

Define the generic status contract here:

- policy `do_nothing` writes `pre_update_protection_status = "skipped"` and
  creates no protection side effects
- `DispatchContext.protection = None` leaves the protection fields `NULL`
  because no controller-side provider is registered for the tenant/runtime
- actual snapshot/backup success writes a plugin-chosen success status plus
  optional summary
- controller-side protection failure writes a plugin-chosen failure status plus
  generic summary/output text

- [ ] **Step 2: Thread the context through immediate dispatch**

Update `trigger_update_for_host(...)` in `update_triggers.rs` to accept
`DispatchContext` instead of bare `notifier`.

The new call shape should look like:

```rust
pub async fn trigger_update_for_host(
    db: &DatabaseConnection,
    dispatch: DispatchContext<'_>,
    params: TriggerUpdateParams<'_>,
) -> Result<TriggerUpdateResult>
```

and invoke protection after history-row creation, before
`dispatch_update_to_agent(...)`.

Protection must run only when dispatch is imminent. If the host is already busy
and `trigger_update_for_host(...)` inserts a `Queued` row, skip protection at
insert time and let the later promotion path run it immediately before the real
dispatch.

Make the “imminent dispatch” rule concrete for offline/replay cases too: split
or wrap the current dispatch helper so the code can distinguish replay-only
preparation from a real new dispatch attempt before invoking protection.

- normal notifier path: `dispatch_update_to_agent(...) == false` still means the
  command was persisted to the outbox for cross-controller delivery, so keep the
  row `Pending` and treat it as a real dispatch attempt
- replay-preparation notifier path: do not run pre-update protection at all,
  because those rows are already pending/replaying and no new protection side
  effect should be created during replay setup

Update every immediate-dispatch caller in the same task:

- `actions/software_items.rs`
- `routes/service_ws/handler/update_tracking.rs`

- [ ] **Step 3: Thread the context through queued/batch dispatch**

Update `create_batch(...)`, `dispatch_next_queued_for_host(...)`, and
`dispatch_next_in_batch(...)` to accept `DispatchContext`.

The initial batch dispatch path in `update_batches/mod.rs` must invoke
controller-side protection before the first `dispatch_update_to_agent(...)`
call for each host, otherwise the first `Pending` batch item bypasses the new
pre-update protection seam.

Implement the protection-failure queue rule with an explicit loop:

```rust
loop {
    let Some(next_record) = find_next_queued(...)? else { break Ok(()); };
    // CAS queued -> pending
    // attempt protection
    // on controller-side failure: mark failed and continue;
    // on success: dispatch and break;
}
```

This is the part that prevents host FIFO queues from stalling when controller-side protection fails before any agent message is produced.

Apply the same rule to the initial `create_batch(...)` per-host dispatch path:
if protection fails for the first `Pending` item before any agent message is
sent, mark that row failed immediately and advance to the next queued sibling in
the same batch/host instead of waiting for a completion event that will never
arrive.

If every remaining item in that batch/host fails controller-side protection
before any agent message is sent, explicitly run the existing batch-completion
check (`maybe_complete_batch(...)` or equivalent) so the parent batch does not
remain stuck `InProgress`.

Update the batch/action callers in the same task:

- `actions/update_batches.rs`
- `core/controller/src/main.rs` startup promotion after rollout cleanup

- [ ] **Step 4: Wire post-update finalization into WS completion and cleanup**

In `service_ws/handler/updates.rs`, invoke `finalize_post_update(...)`:

- after normal owned completion in the same layer that currently handles `finalize_update_result_if_owned(...)`
- after batch-item completion in the same layer that currently handles `finalize_batch_item_if_owned(...)`
- for rows returned by `mark_owned_in_progress_as_failed_on_reconnect(...)`
- for rows returned by `mark_all_in_progress_as_failed_for_rollout(...)`
  from controller startup cleanup in `core/controller/src/main.rs`

Persist `recovery_hint` from `PostUpdateOutcome` in the shared `update_history` row. Keep this idempotent.

Treat finalization as best-effort in reconnect/startup recovery paths: use a
short bounded **per-row** timeout and log on timeout/failure rather than
blocking the service reconnect or controller startup flow on live Proxmox API
latency. A
missed finalization should leave the row failed with `recovery_hint = NULL`
rather than stall queue progression.

Also cover the existing failure branches that can produce terminal failed rows
outside the owned-completion happy path:

- `fail_pending_unowned_update(...)`
- `fail_unreplayable_pending_update(...)`

`fail_pending_unowned_update(...)` currently lacks the tenant/host/software
identifiers needed for `ControllerPostUpdateContext`, so this task must add the
required `update_history` row lookup before calling `finalize_post_update(...)`.
`fail_unreplayable_pending_update(...)` already receives the row model and
should reuse it directly.

For reconnect/startup cleanup helpers that return pre-update `update_history`
models, build `ControllerPostUpdateContext.final_status` from the persisted
write result (`Failed`) or reload the row first; do not trust the stale
in-memory `InProgress` model shape.

- [ ] **Step 5: Expose the singleton through `AppState`**

Add one controller-owned accessor in `AppState`, sourced from existing plugin
registry/catalog wiring, so REST/WS handlers can build `DispatchContext`.

Keep this as an accessor over existing plugin-ops storage rather than a brand
new stored field if possible; that minimizes fallout across direct `AppState`
test literals.

- [ ] **Step 6: Verify dispatch behavior**

Run:

```bash
cargo test -p uptrakit-web-api-queries update_triggers
cargo test -p uptrakit-web-api-queries create_batch
cargo test -p uptrakit-web-api-queries dispatch_next_in_batch
cargo test -p uptrakit-web-api-queries dispatch_next_queued_for_host
cargo check -p uptrakit-web-api
cargo check -p uptrakit-controller
```

Expected: immediate trigger path, initial batch dispatch, queued promotion,
WS handler paths, and controller startup cleanup all compile with one shared
dispatch context.

Add or extend focused tests in this task for:

- protection failure marks the current row `Failed` instead of leaving it
  `Pending`
- queued promotion continues to the next sibling after controller-side
  protection failure
- initial batch dispatch continues to the next sibling after controller-side
  protection failure
- reconnect/startup finalization timeout does not block queue progression

## Task 4: Implement Proxmox controller-side protection, policy storage, and sync-time target cache

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/client.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/discovery.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/controller_migration.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/matching.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/lib.rs`
- Create: `crates/plugins/infrastructure/proxmox/src/update_protection.rs`
- Create: `crates/plugins/infrastructure/proxmox/src/policy_store.rs`

- [ ] **Step 1: Add Proxmox-owned tables**

Extend `controller_migration.rs` with tables for:

- cached backup targets
- global protection defaults keyed by `(tenant_id, plugin_config_id)`
- per-item overrides keyed by `(software_item_id, plugin_config_id)`
- protection audit rows keyed by `update_history_id`

Use Proxmox-owned tables here, not shared DB tables, because these rows contain provider-specific identifiers and execution detail.

- [ ] **Step 2: Add client operations**

Extend `client.rs` with controller-side helpers for:

```rust
pub async fn list_backup_targets(&self, node: &str) -> Result<Vec<PveBackupTarget>>;
pub async fn create_qemu_snapshot(&self, node: &str, vmid: u32, name: &str) -> Result<String>;
pub async fn create_lxc_snapshot(&self, node: &str, vmid: u32, name: &str) -> Result<String>;
pub async fn start_backup(&self, node: &str, vmid: u32, storage: &str, mode: &str) -> Result<String>;
pub async fn wait_for_task(&self, node: &str, upid: &str) -> Result<PveTaskResult>;
```

The exact types can vary, but the plan requires node-aware storage enumeration plus snapshot/backup task submission and polling.

Bound the pre-update `wait_for_task(...)` path with a real timeout ceiling
(per protection attempt, not unbounded) so controller dispatch cannot hang
forever on a slow or wedged Proxmox task.

- [ ] **Step 3: Cache backup targets during sync**

Extend `discovery.rs` so Proxmox sync persists node-aware backup targets alongside guest discovery.

Target identity must be keyed strongly enough to survive same-name storage collisions across nodes/configs.

- [ ] **Step 4: Implement the singleton role**

In `update_protection.rs`, implement the new `ControllerUpdateProtection` trait for the Proxmox plugin singleton.

The role should:

- resolve host -> Proxmox mapping -> `plugin_config_id`
- load effective policy for `(software_item_id, plugin_config_id)`
- enforce `do_nothing` / `snapshot` / `backup`
- write Proxmox-owned audit rows keyed by `update_history_id`
- return only generic shared fields to the caller

Do not rely on `.one()` by `host_id`. The current mapping table does not
guarantee a unique matched row per host. Make the invariant explicit in this
task:

- query all matched mappings for the host within the tenant
- if zero mappings exist, treat protection as `skipped`
- if exactly one mapping exists, continue normally
- if multiple mappings exist, fail protection with a generic configuration
  error rather than guessing a `plugin_config_id`

Also update `matching.rs` so future match operations preserve that invariant
for this feature, either by rejecting conflicting matches or by clearing the
previous matched row before assigning a new one.

Make the protection action idempotent per `update_history_id`:

- use a deterministic audit-row lookup keyed by `update_history_id` before
  creating a new snapshot/backup
- derive snapshot names from `update_history_id` in a Proxmox-safe form that
  stays within the 40-character snapshot-name limit
- if a prior successful protection artifact already exists for the same
  `update_history_id`, reuse/report it instead of creating a second one on a
  later dispatch retry
- if the `db` downcast from `dyn Any` fails, return a normal plugin error with
  clear context instead of panicking

- [ ] **Step 5: Register the role in the plugin descriptor**

Update `plugin.rs` so `declare_plugin!` exports the new singleton and keeps
migrations/surfaces registered in one place.

The controller-side protection module and descriptor field must stay out of
agent builds. Gate `update_protection.rs` and the
`controller_update_protection:` registration behind `#[cfg(not(feature =
"agent-infra"))]` (or the equivalent helper indirection) so agent-targeted
builds do not try to compile controller-only SeaORM/downcast code.

- [ ] **Step 6: Verify Proxmox crate compilation**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --features migrations
```

Expected: controller migrations, client changes, and singleton role compile together.

## Task 5: Add the new `software_item.tabs` shared-surface slot and mount it on the software detail route

**Files:**

- Modify: `crates/shared/surfaces/src/slot.rs`
- Modify: `crates/ui/web-api/src/surface_registry.rs`
- Modify: `crates/shared/surfaces/tests/ids.rs`
- Modify: `frontend/src/routes/software/[id]/+page.svelte`
- Create: `frontend/src/routes/software/[id]/software-detail.test.ts`

- [ ] **Step 1: Add the slot**

Add:

```rust
pub const SLOT_SOFTWARE_ITEM_TABS: &str = "software_item.tabs";
```

and register it as:

```rust
SurfaceSlotDef::multi_entry(SLOT_SOFTWARE_ITEM_TABS, 100, 999)
```

in `SURFACE_SLOT_DEFS`.

Also update the fixed-size `SURFACE_SLOT_DEFS` array length from `6` to `7`.

- [ ] **Step 2: Extend slot tests**

Update `ids.rs` to assert:

```rust
let software_item_tabs = slot_def(SLOT_SOFTWARE_ITEM_TABS).expect("known slot");
assert!(software_item_tabs.multi_entry);
```

Also extend the slot-to-page mapping so `software_item.tabs` is included in the
software page surface registry/filtering path.

- [ ] **Step 3: Mount the slot on software detail**

Follow the host-detail pattern already present in the app. Add derived state for:

- slot surfaces from `software_item.tabs`
- read models
- `baseParams = { software_item_id: id }`
- surface reload handling after item refresh

Then render those surfaces in a dedicated detail section before or alongside the host table.

- [ ] **Step 4: Add a route test**

Create `software-detail.test.ts` to assert:

- software-item slot surfaces are requested when runtime is active
- the route passes `software_item_id`
- the route still preserves existing host-context surface behavior

- [ ] **Step 5: Verify**

Run:

```bash
cd frontend
npm run check
npm run test -- software-detail.test.ts
```

Expected: the software detail route compiles with the new slot wiring.

## Task 6: Add Proxmox-owned settings and software-item surfaces

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/surfaces.rs`
- Create: `crates/plugins/infrastructure/proxmox/src/policy_store.rs` (if not already created in Task 4)
- Modify: `frontend/src/routes/settings/+page.svelte` only if additional route tab defaults are needed

- [ ] **Step 1: Add new registered surfaces**

Extend Proxmox surface registrations with:

- one settings surface under `settings.tabs`
- one software-item surface under `software_item.tabs`

Use the existing Proxmox surface-registration style in `plugin.rs`.

- [ ] **Step 2: Add interaction handlers**

In `surfaces.rs`, add interaction handlers for:

- preload global defaults
- save global defaults
- preload per-item overrides
- save per-item overrides
- load dynamic backup target dropdown options

Those handlers should use `baseParams` and the Proxmox-owned policy/cache tables, not direct live API calls during form render.

Define the empty-cache UX here too: until sync has populated backup targets for
the relevant Proxmox config, render a disabled/empty dropdown state with clear
surface text instead of attempting a live fetch during form render.

- [ ] **Step 3: Permission the surfaces**

Use:

```rust
Permission::ManageGlobalSettings
Permission::ViewSoftware
Permission::UpdateSoftware
```

as specified by the design, so surface visibility and mutation remain aligned with existing permission boundaries.

- [ ] **Step 4: Verify**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox
```

Expected: new surface registrations and action handlers compile with the rest of the plugin.

## Task 7: Render generic protection/recovery UI in history

**Files:**

- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/routes/history/+page.svelte`

The Rust response/query changes should already be complete in Task 2; this task
is the frontend consumption and presentation follow-through.

- [ ] **Step 1: Extend frontend types**

Ensure `UpdateHistoryResponse` in `types.ts` includes:

```ts
pre_update_protection_status?: string | null;
pre_update_protection_summary?: string | null;
recovery_hint?: string | null;
```

- [ ] **Step 2: Render generic history details**

In `history/+page.svelte`, add a small generic section in the expanded row:

- render `pre_update_protection_summary` when present
- render `recovery_hint` only when present
- do not use Proxmox-specific labels or identifiers

- [ ] **Step 3: Verify**

Run:

```bash
cd frontend
npm run check
```

Expected: history page compiles with the generic fields and no plugin-specific UI typing.

## Task 8: Verification pass

**Files:**

- No new files; run verification only after the implementation tasks above are complete.

- [ ] **Step 1: Rust compile/test sweep**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-registry
cargo check -p uptrakit-plugin-infrastructure-proxmox --features migrations
cargo check -p uptrakit-web-api-queries
cargo check -p uptrakit-web-api
cargo check -p uptrakit-controller
cargo clippy --all-targets --no-default-features --features db-sqlite
```

Expected: all touched backend crates compile.

- [ ] **Step 2: Frontend sweep**

Run:

```bash
cd frontend
npm run lint
npm run test
npm run check
npm run build
```

Expected: the touched frontend code lint-checks, tests pass, type-check passes,
and the embedded-frontend build artifact is available for all-features backend
verification.

- [ ] **Step 3: Focused regression tests**

Run:

```bash
cargo test -p uptrakit-web-api-queries dispatch_next_queued_for_host
cargo test -p uptrakit-web-api-queries update_triggers
cargo test -p uptrakit-web-api-queries create_batch
cargo test -p uptrakit-web-api-queries dispatch_next_in_batch
cargo test -p uptrakit-shared-db --features "migration db-sqlite" migrations_run_incrementally_sqlite
cargo check --all-features
cargo clippy --all-targets --all-features
cargo deny check
cargo test --all-features
cd frontend && npm run check
```

Expected: queued dispatch semantics, immediate and batch trigger paths, shared
DB migration path, dependency policy, and full all-features regression gates
still hold.

Also include one catalog-focused test/assertion in this pass for duplicate
`controller_update_protection` singleton rejection if that coverage was not
added earlier in Task 1.
