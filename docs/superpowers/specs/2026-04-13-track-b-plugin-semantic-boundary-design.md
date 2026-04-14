# Track B Plugin Semantic Boundary Design

**Task Context:** Follow-on planning for `TASK-0007` after the static import/dependency track. This design covers semantic leakage only.

**Goal:** Remove plugin-specific knowledge from all non-plugin production code so that only the plugins registry/catalogue owns plugin metadata, classification, plugin-wide settings semantics, and capability-specific behavior decisions.

## Scope

This track covers production code under non-plugin crates that currently knows concrete plugin facts without going
through the registry/catalogue.

In scope:

- plugin-specific settings keys, routes, DTOs, and wiring in non-plugin crates
- plugin-type classification logic in non-plugin crates
- plugin display-name tables in non-plugin crates
- production branching on concrete plugin IDs or plugin-type prefixes
- generic lifecycle dispatch that still requires plugin-specific pre-checks in consumers
- removing the bespoke dashboard-icons API surface and moving it to generic plugin type settings

Out of scope:

- tests, docs, and internal change artifacts using explicit plugin names
- the already-merged static import/dependency cleanup track
- plugin-to-plugin dependencies
- preserving compatibility for `/api/v1/settings/dashboard-icons`
- migrating existing `dashboard_icons.enabled` tenant state into the new generic surface

## Boundary Rule

Non-plugin production code may:

- depend on `uptrakit-plugin-infrastructure-registry`
- hold and transport opaque `PluginTypeId` values
- ask generic questions through registry/catalogue APIs
- consume generic plugin type settings endpoints
- consume generic plugin metadata only for display/configuration surfaces, not for behavior branching

Non-plugin production code may not:

- branch on concrete plugin IDs or ID prefixes
- own plugin display-name lookup tables
- own plugin-family classification helpers
- define plugin-specific settings keys, endpoints, DTOs, or route modules
- pre-check plugin-specific enablement before invoking a generic plugin hook
- use raw plugin metadata or category values as an indirect substitute for plugin-ID branching

This means the registry/catalogue becomes the only production source of truth for:

- display names and other display-only metadata
- type-settings presence, schema, and sample/default payloads
- capability presence
- registry-owned predicate helpers needed for behavior decisions
- lifecycle dispatch behavior tied to plugin-owned tenant settings

Non-plugin production behavior decisions should be expressed in capability terms or through narrow registry predicates, not
through raw plugin-family/category metadata.
Registry predicates used for behavior decisions must remain capability- or domain-generic. Track B should not replace
direct ID branches with identity-specific helpers such as `is_dashboard_icons(...)`.

## Current Leaks To Remove

Known semantic leaks in the current code shape include:

- `crates/ui/web-api/src/routes/settings_dashboard_icons.rs`
- `crates/ui/web-api/src/router.rs`
- `crates/ui/web-api/src/routes/mod.rs`
- `crates/ui/web-api-auth/src/setting_key.rs` via `DashboardIconsEnabled`
- `crates/shared/web-api-types/src/settings_dashboard_icons.rs`
- `crates/ui/web-api/src/routes/software_items/mod.rs` pre-checking dashboard-icons enablement
- `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` pre-checking dashboard-icons enablement
- `crates/shared/types/src/plugin_type_id.rs` via `is_package_manager()` and `display_name()`
- non-plugin production callers that special-case concrete plugin IDs or plugin categories

The parent `TASK-0007` artifacts intentionally deferred this category of leak. Track B exists to close that gap.

## Recommended Architecture

### 1. Registry-Owned Metadata

`PluginTypeId` remains an opaque identifier type, not a metadata owner.

The registry/catalogue should expose narrow helpers for the generic questions consumers actually need. Existing generic
capability accessors should be used where sufficient; new helpers should be added only where the current surface forces
consumers back into plugin-specific logic.
If a question can already be expressed as an existing capability lookup, use that. New registry predicates are warranted
only when the behavior turns on a domain-generic distinction that the current capability surface cannot represent.

Target outcome:

- remove `PluginTypeId::is_package_manager()`
- remove `PluginTypeId::display_name()`
- move all production callers to registry-backed metadata/capability queries

`is_package_manager()` callers should land on existing registry descriptor/capability answers where possible. If that is
still insufficient after the rewrite inventory is built, Track B may add one domain-generic registry predicate backed by
descriptor data rather than by concrete ID matching.

If a consumer needs to know whether a plugin type has tenant-scoped type settings, it asks the registry.
If it needs a display name, it asks the registry.
If it needs to decide whether a generic flow applies, it uses generic capabilities or registry-owned predicate helpers, not
raw metadata and not ID
comparisons.

Track B does not require moving the physical definition of canonical plugin-ID constants. The success criterion is that
non-plugin production code no longer imports or branches on concrete IDs for behavior decisions.
If those constants remain in `crates/shared/types`, imports are permitted only in plugin crates and registry/catalogue
implementation code, not in other non-plugin production crates.

### 2. Plugin-Owned Type Settings

Dashboard-icons moves from a bespoke tenant setting to generic plugin type settings.

This should reuse the existing generic plugin type settings surface:

- `crates/ui/web-api/src/routes/plugin_type_settings.rs`
- `crates/ui/web-api-queries/src/queries/plugin_type_settings.rs`
- the existing `plugin_type_settings` persistence table/entity
- the existing plugin descriptor `TypeSettingsOps` hook for schema/sample exposure

`DashboardIconsConfig` should implement the existing `TypeSettings` contract with an `enabled` flag, exposed through the
descriptor's `TypeSettingsOps` hook. Unset state should behave as enabled by default so dropping
`dashboard_icons.enabled` does not silently disable the feature for tenants that never configured it.
The generic plugin type settings UI is expected to render that boolean field without bespoke frontend work; Track B should
verify this explicitly before deleting the old endpoint.

Target outcome:

- delete the bespoke dashboard-icons settings route/module
- delete the dashboard-icons-specific web-api types
- delete `SettingKey::DashboardIconsEnabled`
- expose dashboard-icons configuration only through generic plugin type settings

No migration is required for the old key. Existing `dashboard_icons.enabled` rows become dead data and may be ignored or
cleaned up later in a separate maintenance step.
Tenants that explicitly opted out through the old `dashboard_icons.enabled = false` setting will see icons become enabled
again after Track B. That behavior change is an accepted trade-off for this track.

### 3. Registry-Side Lifecycle Enablement

The current consumer-side pre-check for dashboard-icons exists because lifecycle dispatch lacks a generic way to apply
plugin-owned tenant settings at dispatch time.

Track B should move that logic behind a generic lifecycle dispatch seam. Non-plugin consumers should:

1. build the generic lifecycle event
2. call the generic lifecycle hook with a pre-resolved generic lifecycle context
3. consume the returned patch or `None`

They should not know which plugin may respond or which plugin-specific setting disables it.

Chosen implementation direction:

- the application layer loads generic tenant-scoped type settings before entering lifecycle dispatch and passes a
  pre-resolved synchronous context into the dispatch call
- extend `SoftwareItemLifecycleOps::on_software_item_created(...)` with a hard signature change so it accepts that
  lifecycle context alongside the generic event; all `SoftwareItemLifecycle` implementors are updated atomically in the
  same branch
- the lifecycle context should expose a simple synchronous map/view such as
  `HashMap<PluginTypeId, serde_json::Value>` carrying already-fetched generic type-settings payloads, not an async query
  trait and not a direct persistence dependency
- extend the lifecycle-plugin invocation path so each plugin receives its own resolved type settings through that context
- keep the enablement decision in the plugin: dashboard-icons reads its own resolved `enabled` flag and returns `None`
  when disabled
- keep the consumer-facing result unchanged: `Option<SoftwareItemPatch>`

The registry/catalogue must not depend directly on DB entities, web-api query crates, or other application-layer
persistence code. The application layer provides generic settings lookup and pre-resolves the context; the plugin side
owns plugin-specific interpretation of the returned settings payload.

### 4. Consumer Rewrite Rule

For every remaining production code path in non-plugin crates:

- if the code compares against `plugin_ids::...`, replace it with a registry metadata/capability query
- if the code imports `plugin_ids::...` from shared types, remove that import from non-plugin production code
- if the code calls `PluginTypeId` convenience classification/display helpers, replace it with a registry query
- if the code imports or names a plugin-specific settings surface, delete that surface and use generic plugin type
  settings

The design target is not “fewer hardcoded plugin names.” It is “none in non-plugin production code.”

## Expected Affected Areas

Primary files/modules expected to change:

- `crates/shared/types/src/plugin_type_id.rs`
- `crates/plugins/enhancements/dashboard-icons/src/config.rs`
- registry/core plugin-op surfaces under `crates/plugins/infrastructure/core/src/`
- registry re-exports/helpers under `crates/plugins/infrastructure/registry/src/`
- `crates/ui/web-api/src/routes/plugin_type_settings.rs`
- `crates/ui/web-api-queries/src/queries/plugin_type_settings.rs`
- `crates/ui/web-api/src/routes/settings_dashboard_icons.rs`
- `crates/ui/web-api/src/router.rs`
- `crates/ui/web-api/src/routes/mod.rs`
- `crates/ui/web-api/src/routes/software_items/mod.rs`
- `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`
- `crates/ui/web-api-auth/src/setting_key.rs`
- `crates/shared/web-api-types/src/settings_dashboard_icons.rs`
- `crates/shared/web-api-types/src/lib.rs`
- production code using `plugin_ids::...` or `is_package_manager()` in non-plugin crates, especially under
  `crates/ui/web-api-queries/src/queries/`

Router/OpenAPI cleanup must explicitly remove the dedicated `DashboardIconsApiDoc` wiring in
`crates/ui/web-api/src/router.rs`.

No bespoke frontend dashboard-icons migration is expected because the generic plugin type settings UI already exists, but
the implementation must still regenerate OpenAPI/typed clients and verify that no in-repo caller still references the
removed bespoke endpoint.

The exact rewrite inventory should be produced from code search before implementation begins and captured as a persisted
implementation artifact, not kept as an implicit checklist.

## Data Flow After Track B

### Dashboard-icons configuration

1. Client fetches generic plugin-type metadata.
2. Registry reports that dashboard-icons has type settings and returns its generic form schema and sample/default.
3. Client reads/writes dashboard-icons settings only through generic plugin-type-settings endpoints.
4. Non-plugin server code stores/retrieves those settings generically, without naming dashboard-icons-specific routes or
   keys.

### Software-item lifecycle dispatch

1. Non-plugin consumer creates a generic `SoftwareItemCreatedEvent`.
2. The application layer passes a pre-resolved generic lifecycle context into the dispatch call.
3. Registry/core dispatch enumerates lifecycle plugins and reads each plugin's generic tenant-scoped type settings from
   that context without performing I/O.
4. Each lifecycle plugin receives its own resolved settings through the lifecycle invocation context.
5. The dashboard-icons plugin interprets its own `enabled` setting and returns `None` when disabled.
6. Registry/core merges plugin patches and returns `Option<SoftwareItemPatch>`.
7. Consumer applies the patch or skips on `None`.

No consumer-side dashboard-icons branch remains.

## Error Handling

- Unknown plugin type in generic settings flows remains a generic registry/API error, not a plugin-specific branch.
- Missing dashboard-icons type settings should be treated as the plugin default, not as an error.
- Lifecycle plugin failures remain best-effort and isolated per existing lifecycle semantics.
- Removing bespoke endpoints is a deliberate breaking change; callers that still hit the removed dashboard-icons endpoint
  will receive normal 404/route-missing behavior after rollout.

## Testing Strategy

Required tests for Track B:

- registry-backed metadata/classification tests proving former `PluginTypeId` helper callers now use registry answers
- registry predicate tests proving new behavior helpers are capability/domain-generic rather than plugin-identity-specific
- `TypeSettingsOps` schema/sample tests proving dashboard-icons advertises the expected generic form schema and
  sample/default payload
- dashboard-icons type-settings tests:
  - unset settings -> enabled by default
  - explicit enabled = true -> enabled
  - explicit enabled = false -> disabled
- regression test proving leftover `dashboard_icons.enabled` rows no longer affect behavior after Track B
- lifecycle dispatch tests proving non-plugin consumers no longer pre-check dashboard-icons state
- end-to-end lifecycle integration test: app-layer preload -> dispatch context -> dashboard-icons plugin ->
  `None` when disabled / patch when enabled
- web-api route/openapi tests proving the bespoke dashboard-icons settings surface is removed
- UI/schema verification that the generic settings form renders and submits the dashboard-icons `enabled` boolean field
- regression tests for any rewritten production plugin-ID branches

Required negative checks:

- no non-plugin production code references `dashboard_icons.enabled`
- no non-plugin production code references `settings_dashboard_icons`
- no non-plugin production code imports `plugin_ids::...` from shared types
- no non-plugin production code compares concrete plugin ID strings or prefixes
- no non-plugin production code uses `PluginTypeId::is_package_manager()` or `PluginTypeId::display_name()`
- no new identity-specific registry predicates/helpers such as `is_<plugin_name>(...)` or `has_<plugin_name>(...)`

These checks should be enforced by a dedicated CI policy/grep step with an explicit allowlist for tests, docs, and
internal change artifacts. They should not exist only as unit-test expectations.

Track B does not introduce a dashboard-icons-specific authorization exception. Generic plugin type settings writes continue
to use the generic authorization model already attached to that surface. If that model later proves too coarse for
display-only settings, that should be handled as a separate follow-on auth task rather than a plugin-specific carve-out.

## Rollout

Track B should be implemented as one bounded breaking-change track after Track A.

Recommended order:

1. add the lifecycle type-settings seam: pre-resolved generic lifecycle context in plugin infrastructure, with no direct
   application-layer dependency from registry/catalogue
2. convert dashboard-icons to generic type settings
3. add semantic-boundary regression checks and inventory-backed verification gates
   these may start with a temporary implementation-tracking baseline/allowlist in the same branch, then tighten as the
   rewrites land
4. migrate all in-repo callers and regenerate OpenAPI/typed clients
5. remove dashboard-icons bespoke API surface
6. remove `PluginTypeId` helper shortcuts and rewrite remaining plugin-ID branches together so callers are not split
   across overlapping partial migrations

The breaking API removal and helper removals should ship in the same branch so the codebase does not sit in a hybrid
state.
The bespoke dashboard-icons endpoint should not be removed until all in-repo callers are migrated, OpenAPI output is
updated, and the generic settings flow passes end-to-end verification.

## Risks

- the lifecycle dispatch seam requires a small additive plugin-infrastructure abstraction and trait-signature change
  before the consumer rewrite is possible
- some current production flows may rely on plugin-category shortcuts more broadly than the initial inventory suggests
- generic capability metadata may not be sufficient for every current special case, requiring one or two new
  capability/domain-generic registry helper methods
- removing `PluginTypeId` helper methods may surface hidden production callers outside the currently known files

## Alternatives Rejected

### Keep a dashboard-icons compatibility shim

Rejected because it preserves plugin-specific knowledge in non-plugin production code and weakens the boundary target.

### Keep `PluginTypeId` classification/display helpers as “generic enough”

Rejected because they encode plugin-specific facts in a shared non-registry type and encourage future leakage.

### Generate shared plugin metadata tables outside the registry

Rejected because it still places plugin-specific knowledge outside the registry/catalogue boundary, only in a more
automated form.

## Readiness

This design is ready for implementation planning if the implementation plan is limited to Track B only and produces:

- an exact rewrite inventory of semantic leaks in non-plugin production code
- a persisted rewrite inventory artifact tied to the implementation plan and CI checks
- the lifecycle type-settings seam design, including the pre-resolved context and signature change
- the dashboard-icons deletion list
- verification steps proving zero plugin-specific knowledge remains in non-plugin production code
