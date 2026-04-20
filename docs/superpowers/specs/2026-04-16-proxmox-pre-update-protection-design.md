# Proxmox Pre-Update Protection Design

## Goal

Allow the Proxmox plugin to create a snapshot or backup before a software update runs, make that behavior configurable through global defaults and
per-software-item overrides, and record recovery guidance for failed updates without leaking Proxmox-specific concepts into shared controller or UI
contracts.

## Scope

### V1

- pre-update protection modes:
  - `do_nothing`
  - `snapshot`
  - `backup`
- config scopes:
  - global default
  - per-software-item override
- target discovery:
  - fetch backup-capable targets during Proxmox synchronization
  - cache them for later selection in UI
- update behavior:
  - execute protection on the controller immediately before dispatching the update
  - block the update if the requested protection step fails
- recovery UX:
  - expose generic protection status and recovery guidance in update history

### Out of scope

- per-host override policy
- non-Proxmox infrastructure providers
- automatic rollback or restore execution
- a generic backup/snapshot abstraction shared across unrelated plugins
- a new update-history terminal status such as `aborted`

## Current Codebase Baseline

### Update dispatch and history

- Update dispatch still flows through
  [`crates/ui/web-api-queries/src/queries/update_dispatch.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/update_dispatch.rs).
- Current dispatch supports agent-side `pre_update_hook` and `post_update_hook` assignments, but that mechanism is tied to host-software plugin
  assignments and is the wrong seam for controller-executed Proxmox protection.
- Update history API contracts live in
  [`crates/shared/web-api-types/src/update_history.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/update_history.rs)
  and query assembly lives in
  [`crates/ui/web-api-queries/src/queries/update_history.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/update_history.rs).
- Current public history status values are `queued`, `pending`, `in_progress`, `completed`, and `failed`.

### Shared-surface UI after the redesign

- The legacy extension framework is gone.
- Shared surface slot definitions now live in
  [`crates/shared/surfaces/src/slot.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/slot.rs).
- The relevant current slots are:
  - `settings.tabs`
  - `settings.below.global`
  - `software.tabs`
  - `host_detail.tabs`
  - `software_item.host_context_menu`
  - `extension.page`
- There is currently no software-item detail tab slot.
- The software-item detail route in
  [`frontend/src/routes/software/[id]/+page.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/routes/software/[id]/+page.svelte) is
  already surface-aware, but it only mounts `software_item.host_context_menu`.
- The host detail route in
  [`frontend/src/routes/hosts/[id]/+page.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/routes/hosts/[id]/+page.svelte) is the best
  current model for mounting contextual read surfaces with `baseParams`.

### Surface interaction capabilities

- [`frontend/src/lib/components/surfaces/SurfaceForm.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/lib/components/surfaces/SurfaceForm.svelte)
  supports:
  - `baseParams`
  - `preLoadInteraction`
  - dynamic select option loading
- [`frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte)
  supports provider-query hydration with `baseParams`.
- This is enough to support dynamic dropdowns for cached Proxmox backup targets and preload of existing per-item settings without reviving legacy
  extension constructs.

### Proxmox plugin registration model

- The Proxmox plugin already registers shared surfaces through
  [`crates/plugins/infrastructure/proxmox/src/plugin.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/plugin.rs)
  and
  [`crates/plugins/infrastructure/proxmox/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/surfaces.rs).
- Current Proxmox surfaces are:
  - `proxmox.hosts` in `extension.page`
  - `proxmox.host-info` in `host_detail.tabs`
- This matters because the new feature should extend the same surface model instead of introducing a parallel UI path.

## Design Options

### Recommended: generic controller-side protection seam plus Proxmox-owned surfaces

- Add a small shared controller-side pre/post update protection seam.
- Keep all Proxmox policy resolution, target discovery, execution, audit records, and recovery hint generation inside the Proxmox plugin.
- Expose global defaults and per-software-item overrides through Proxmox-owned shared surfaces.

Why this is the best option:

- keeps Proxmox details out of shared controller and frontend code
- matches the new shared-surface architecture
- solves the dynamic target dropdown requirement cleanly
- avoids overloading host-software role assignments for a controller-only concern

### Rejected: reuse host-software hook assignments with `execution_site=controller`

- This would reuse the old hook assignment path and store Proxmox protection as role plugins on `host_software_item_plugin`.

Why it is rejected:

- the role-assignment model is host-oriented, while the requested policy is global and per-software-item
- it pushes Proxmox-specific semantics into generic plugin assignment flows
- it preserves exactly the complexity that previously made the design stall

### Rejected: generic core backup framework in V1

- This would define a cross-plugin backup/snapshot abstraction immediately.

Why it is rejected:

- there is only one concrete provider right now
- it expands scope into premature generalization
- it would force shared contracts to understand details they do not need yet

## Proposed Design

### Architecture boundary

Add one generic controller-side update protection seam in shared controller code, but keep the policy and implementation plugin-owned.

This must be a new singleton plugin role, distinct from the existing agent-side `LifecycleHook` trait in
[`crates/plugins/infrastructure/core/src/roles.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/roles.rs).

Working name for the new role:

- `ControllerUpdateProtection`

Working API shape:

- `prepare_pre_update_protection(ctx) -> Result<ControllerProtectionDecision>`
- `finalize_post_update(ctx) -> Result<PostUpdateOutcome>`

Working generic payload shapes:

- `ControllerProtectionDecision`
  - `attempted: bool`
  - `succeeded: bool`
  - `protection_status: Option<String>`
  - `protection_summary: Option<String>`
- `PostUpdateOutcome`
  - `recovery_hint: Option<String>`

The caller-facing context must include at least:

- `tenant_id`
- `host_id`
- `software_item_id`
- `update_history_id`
- `final_status` for post-update finalization
- shared DB access

Resolution rule:

- `plugin_config_id` is not part of the shared caller context
- the plugin role resolves host-to-Proxmox mapping and derives `plugin_config_id` internally from `host_id`

The role must be surfaced from plugin descriptor/catalog wiring as a singleton role in the same general style as other controller-side singletons in
[`crates/plugins/infrastructure/core/src/descriptor.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/descriptor.rs).

Access path note:

- shared controller code should access the role through the existing plugin registry dependency and catalog/app-state wiring rather than introducing a
  new cross-layer dependency path just for this feature

Recommended injection shape:

- introduce a concrete `DispatchContext` struct that bundles:
  - `notifier`
  - `protection: Option<Arc<dyn ControllerUpdateProtection>>`

Singleton scope rule:

- this is a controller-startup singleton owned by shared controller state, analogous to other controller-side singleton plugin roles
- in V1, the shared layer invokes the single registered `ControllerUpdateProtection` implementer if present
- in V1, only the Proxmox plugin is expected to implement the role

The dispatch caller in `web-api-queries` should receive that explicit context rather than trying to tunnel the feature through host-software plugin
assignment rows.

Normative absence rule:

- if `protection` is `None`, the shared dispatch layer behaves as a pass-through no-op for controller-side protection

Shared controller responsibilities:

- ask whether a controller-side pre-update protection step applies
- invoke that step before dispatch
- persist generic outcome fields on update history
- invoke plugin post-update finalization after update completion

Persistence rule:

- shared update-history fields are written by the shared caller, not directly by the plugin role
- `prepare_pre_update_protection(...)` and `finalize_post_update(...)` return generic outcome payloads
- the shared caller persists those payloads onto the shared `update_history` row

Proxmox plugin responsibilities:

- resolve effective protection policy
- discover and cache valid backup targets
- execute snapshot or backup creation
- persist Proxmox-specific audit details keyed by `update_history_id`
- produce generic user-facing protection and recovery summaries

Everything outside the Proxmox plugin must remain unaware of:

- VM vs CT distinctions
- Proxmox task IDs
- storage IDs and node semantics
- exact restore commands

Shared contracts should only carry:

- whether protection was attempted
- whether it succeeded
- a short protection summary
- a short recovery hint when the update failed and a recovery point exists

### Effective policy model

Protection policy resolution order:

1. per-software-item override
2. global Proxmox default
3. implicit fallback to `do_nothing`

Runtime keying rule:

1. inside the Proxmox protection role, resolve the target host to its Proxmox mapping
2. inside the Proxmox protection role, derive the relevant `plugin_config_id` from that mapping
3. resolve effective policy for `(software_item_id, plugin_config_id)`
4. if no per-item override exists, fall back to tenant-global default for `(tenant_id, plugin_config_id)`
5. if no row exists, fall back to `do_nothing`

The effective policy shape should support:

- `mode`
- target selection metadata when `mode = backup`

V1 intentionally does not include per-host override. The persistence model should leave room for that later without forcing it now.

### Backup target model

Backup targets must be discovered during Proxmox synchronization and cached in Proxmox-owned storage.

Target identity must be node-aware. It must not collapse to a plain storage name, because storage availability is node-scoped in Proxmox and the same
label can exist in different contexts.

A cached target record should include enough information to:

- display a readable label in the UI
- validate that a selected target still belongs to the relevant Proxmox config and node scope
- survive sync-to-update gaps without querying Proxmox again during form render

The global default target and per-software-item override should be stored per Proxmox config, not as one tenant-wide free string. A software item can
span hosts attached to different Proxmox configs, and V1 must not silently treat those as one namespace.

That implies two persistence keys:

- global default keyed by `(tenant_id, plugin_config_id)`
- per-item override keyed by `(software_item_id, plugin_config_id)`

The software-item surface preload path must therefore first discover the relevant Proxmox configs for the current `software_item_id`, then load or
initialize one policy row per relevant config.

### Update execution rules

Update-history row creation must happen before protection execution in every path so that `update_history_id` exists for both generic shared fields
and Proxmox-owned audit rows.

Immediate dispatch path:

1. load the effective Proxmox protection policy for that software item
2. determine whether the target host belongs to a Proxmox-managed guest with enough metadata to execute the selected mode
3. if mode is `do_nothing`, proceed normally
4. if mode is `snapshot` or `backup` and the host is not Proxmox-resolvable, fail the update attempt rather than silently skipping protection
5. if mode is `snapshot` or `backup`, run the protection step on the controller after the history row is created and before agent dispatch
6. if protection creation fails, persist the failure and mark the update history entry `failed`
7. if protection succeeds, persist generic summary fields plus Proxmox-owned audit details, then dispatch the update

Queued and batch continuation paths:

- The same protection step must run immediately before dispatch in queued promotion paths, including
  [`dispatch_next_queued_for_host(...)`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:101)
  and therefore
  [`dispatch_next_in_batch(...)`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:192).
- Queued rows already have an `update_history_id`; that existing row must be reused for the protection attempt.
- Queued rows must re-resolve effective protection policy at dispatch time, not snapshot it at queue-creation time. This keeps behavior aligned with
  the current effective configuration at the moment work actually starts.
- Protection is evaluated per update item, not once per batch.
- If protection fails for one batch item, that item becomes `failed`, but the batch continues with the next queued item using existing FIFO semantics.
- If protection fails inside `dispatch_next_queued_for_host(...)`, that function must immediately continue queue progression for the same host after
  marking the current row `failed`; otherwise later queued items for that host would stall because no agent result event will arrive to re-trigger
  FIFO dispatch.
- Implementation rule: use an explicit loop inside `dispatch_next_queued_for_host(...)`, not recursive self-calls.

This design deliberately keeps V1 on existing update statuses. A failed protection step is represented as a failed update attempt with explicit output
and protection metadata, rather than inventing a new terminal status.

### Recovery guidance model

V1 should suggest recovery. It should not execute recovery.

The Proxmox plugin should write plugin-owned audit rows keyed by `update_history_id` containing:

- protection mode used
- recovery point identifier
- human-readable recovery label
- status of the protection task
- optional plugin-private details needed for future restore features

Shared update history should expose only generic recovery-facing fields, for example:

- `pre_update_protection_status`
- `pre_update_protection_summary`
- `recovery_hint`

These generic fields should live on the shared `update_history` record as nullable columns so they are available in the existing history API without
plugin-specific joins.

The `recovery_hint` should be populated only when:

- the update ended in `failed`
- a recovery point was created successfully

On successful update completion, `recovery_hint` must be `None` even when a recovery point still exists.

Example user-facing language:

- success: `Pre-update protection created before update`
- failed update with recovery point: `Recovery available: use the recovery point created before this update`

### Shared-surface integration

The frontend part of this feature should be built entirely on the shared-surface runtime.

#### New slot

Add a new shared slot:

- `software_item.tabs`

Slot definition requirements:

- constant name: `SLOT_SOFTWARE_ITEM_TABS`
- slot id: `software_item.tabs`
- `multi_entry = true`
- provider priority range: `100..=999`
- append it to `SURFACE_SLOT_DEFS`, increasing the registry count from 6 to 7

Why this slot is needed:

- the feature is scoped to the software item, not to one host row
- `software_item.host_context_menu` is the wrong abstraction for default policy editing
- the software detail route already has surface plumbing, so adding a dedicated slot is a small, consistent extension

#### Software-item route

Update [`frontend/src/routes/software/[id]/+page.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/routes/software/[id]/+page.svelte)
to:

- load surfaces from `software_item.tabs`
- load their read models when the surface runtime is active
- pass `baseParams` such as `software_item_id`
- render the resulting surface panels in a dedicated detail-tab area

The route should follow the same general pattern already used by
[`frontend/src/routes/hosts/[id]/+page.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/routes/hosts/[id]/+page.svelte) for contextual
surfaces.

#### Global settings surface

Add a Proxmox-owned surface under `settings.tabs` for tenant-wide defaults. It should allow:

- choosing default mode
- choosing default backup target per Proxmox config when mode is `backup`

Permissions:

- surface visibility and mutation require `ManageGlobalSettings`

#### Software-item override surface

Add a Proxmox-owned surface under `software_item.tabs` for per-item overrides. It should allow:

- inheriting global defaults
- overriding mode
- overriding backup target per Proxmox config when relevant

Both forms should use shared-surface capabilities already present in the new runtime:

- preload existing values with the surface contract preload interaction (`pre_load_interaction_id`, wired to `preLoadInteraction` in the Svelte
  component layer)
- load backup target options dynamically
- submit with `baseParams`

Permissions:

- read surface visibility requires `ViewSoftware`
- mutating interactions require `UpdateSoftware`

### UX rules

#### Mode selection

Expose exactly three modes:

- `Do nothing`
- `Snapshot`
- `Backup`

`Backup` should reveal target selectors only when a relevant Proxmox config is in scope.

#### Target selection

Target selection should use dropdowns populated from the cached sync results, not free text.

When multiple Proxmox configs are relevant, selectors should be grouped by config identity. V1 should prefer explicit grouping over a misleading
single global dropdown.

#### Update history presentation

Shared history UI should display generic protection and recovery language only. It should not render raw Proxmox terms or plugin-private identifiers.

## Backend Changes

### Shared controller changes

Add a small shared controller-side protection interface that can be invoked from update dispatch before agent execution and after completion.

The shared layer should not know what Proxmox does internally. It should only orchestrate:

- pre-dispatch protection attempt
- persistence of generic outcome fields
- post-completion finalization callback

Call-site requirements:

- pre-dispatch protection is invoked from the same controller-side dispatch paths that currently call `dispatch_update_to_agent(...)`
- post-update finalization is invoked from the WebSocket result-handling layer, co-located with the existing update finalization flow, with access to
  the controller-owned singleton from shared application state
- reconnect or rollout cleanup paths that mark in-progress updates as failed must also invoke post-update finalization so recovery hints remain
  correct for controller-owned failure paths that do not receive an agent completion message
- for those cleanup paths, the preferred integration is: keep the bulk DB mutation functions returning the affected rows, then have the calling
  WebSocket/service layer invoke `finalize_post_update(...)` per returned row rather than injecting plugin context directly into the bulk mutation
  helpers
- `finalize_post_update(...)` must be idempotent and safe to call on rows that were already finalized through the normal completion flow

### Proxmox client changes

Extend the Proxmox client layer to support:

- listing backup-capable storages in a node-aware way
- creating snapshots for CTs and VMs
- starting backup tasks
- polling task completion

### Proxmox persistence changes

Add Proxmox-owned persistence for:

- cached backup targets discovered during synchronization
- global default protection policy
- per-software-item override policy
- protection audit rows keyed by `update_history_id`

The shared layer must not own Proxmox-specific tables or schemas.

### Synchronization changes

During Proxmox synchronization:

- collect backup-capable targets for the configured nodes
- normalize them into the Proxmox-owned cache
- prune or mark stale cached targets when they disappear

Sync is the only required target refresh path for V1. Form rendering should read from the cache.

## Frontend Changes

### Shared-surface runtime

Add `software_item.tabs` to the shared slot registry in
[`crates/shared/surfaces/src/slot.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/slot.rs) as `SLOT_SOFTWARE_ITEM_TABS`,
mark it `multi_entry`, and wire it through the same surface list/read APIs already used by other slots.

### Software detail page

Mount the new slot on
[`frontend/src/routes/software/[id]/+page.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/routes/software/[id]/+page.svelte) and pass
`software_item_id` as contextual `baseParams`.

### Proxmox surfaces

Add new Proxmox surface registrations and interactions for:

- global default protection settings
- per-software-item override settings
- read panels that show the currently effective protection configuration

### History UI

Extend the update history frontend types and renderers to show generic protection summary and recovery hint fields without exposing plugin-private
details.

## Error Handling

- If selected backup mode requires a target and no valid target is configured, the update must fail before agent dispatch with a clear user-facing
  message.
- If selected mode is `snapshot` or `backup` and the host cannot be resolved to a Proxmox-managed guest through existing mappings, the update must
  fail before agent dispatch. V1 must not silently downgrade to `do_nothing`.
- If a cached target becomes invalid after synchronization, the plugin should reject it at execution time and mark the update failed with recovery
  metadata omitted.
- If snapshot or backup creation times out or returns an error, the update must not dispatch.
- If the update later fails after protection succeeded, the history entry should include a recovery hint.
- If the update succeeds, V1 does not perform cleanup of the created snapshot or backup automatically.

## Testing

### Shared controller

- pre-dispatch protection role invocation
- failure path blocks dispatch
- success path proceeds to agent dispatch
- generic history fields are persisted correctly
- queued-path protection failure still advances host FIFO dispatch
- reconnect and rollout cleanup paths still produce correct post-update recovery metadata

### Proxmox backend

- policy resolution order
- sync-driven backup target cache updates
- node-aware target identity and validation
- snapshot execution path
- backup execution path
- recovery audit persistence

### Frontend

- `software_item.tabs` route integration
- settings surface preload and submit flow
- per-item override preload and submit flow
- dynamic backup target dropdown loading
- history rendering for generic protection and recovery fields

## Acceptance

The design is complete when:

- Proxmox can create either a snapshot or a backup before a selected update
- `do_nothing`, `snapshot`, and `backup` are configurable globally and per software item
- backup targets are discovered during synchronization and selected through dropdowns
- the software-item UI is implemented through shared surfaces, not legacy extension code
- the shared controller and frontend contracts remain Proxmox-agnostic
- failed updates with an available recovery point show generic recovery guidance
- per-host override remains out of scope in both API and UI for V1
