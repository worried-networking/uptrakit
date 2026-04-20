# Plugin Boundary Isolation Track A Design

## Summary

Uptrakit should complete the static plugin-boundary cleanup that `TASK-0007` only partially covered. Track A removes remaining removable direct
plugin-crate dependencies from non-plugin crates, retires shared `PluginTypeId` convenience helpers that bypass the registry, and makes
`uptrakit-plugin-infrastructure-registry` the sanctioned boundary for non-plugin consumers that need plugin metadata, descriptor access, or other
static plugin queries.

This track is intentionally narrow. It does not solve semantic leakage such as plugin-specific settings routes, plugin-specific keys like
`dashboard_icons.enabled`, or production `plugin_ids::*` behavior branching. Those are deferred to later tracks.

## Background

The original `TASK-0007` documents and approved artifacts solved only the first half of the architectural problem: preventing direct crate-edge
leakage from non-plugin crates into plugin crates. The approved scope explicitly left plugin-specific behavior knowledge for later.

That left two kinds of residue in the codebase:

- Some non-plugin crates still import plugin crates or depend on them directly in `Cargo.toml`.
- Shared helpers on `PluginTypeId`, especially `display_name()` and `is_package_manager()`, let non-plugin code infer plugin-specific meaning without
  going through the registry.

The desired architecture is stricter:

- plugins define plugin behavior
- the registry/catalogue is the single non-plugin access boundary
- non-plugin crates should not know plugin crate layouts
- non-plugin crates should not derive plugin semantics from hardcoded shared helpers

Track A addresses only that static architectural boundary.

## Goals

- Eliminate direct plugin-crate imports from non-plugin production crates outside the explicit operational carve-out.
- Eliminate removable direct plugin-crate dependencies from non-plugin `Cargo.toml` manifests.
- Make `uptrakit-plugin-infrastructure-registry` the sanctioned boundary for non-plugin metadata and descriptor access.
- Remove or retire production use of `PluginTypeId::display_name()` and `PluginTypeId::is_package_manager()`.
- Allow small additive registry convenience methods where needed to keep consumers from rebuilding descriptor logic.
- Update `.sentrux/rules.toml` so the enforced static dependency rules match the intended boundary.

## Non-Goals

- Fixing plugin-specific settings routes such as
  [`settings_dashboard_icons.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/settings_dashboard_icons.rs).
- Migrating plugin-specific settings keys such as `dashboard_icons.enabled`.
- Removing production `plugin_ids::*` usage where those IDs drive plugin-specific behavior.
- Reworking plugin capability policy or plugin-specific branching in shared, UI, or core flows.
- Designing CI or lint policy for semantic leakage patterns beyond static crate-edge rules.

## Current Problems

### Direct plugin-crate imports remain outside plugins and registry

Remaining residue exists in several non-plugin areas, including:

- UI query layer under [`crates/ui/web-api-queries/src/queries/`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries)
- shared agent logic under [`crates/shared/agent-core/src/`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src)
- scheduler execution under
  [`crates/shared/scheduler-engine/src/executors/`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/src/executors)
- controller and SSH agent code under [`crates/core/controller/src/`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src) and
  [`crates/core/agent-ssh/src/`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src)

These consumers still reach into plugin crates directly instead of consuming registry exports.

### Shared helper methods bypass the boundary

[`crates/shared/types/src/plugin_type_id.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/plugin_type_id.rs) contains
convenience helpers that encode plugin-specific knowledge in a shared crate:

- `PluginTypeId::display_name()`
- `PluginTypeId::is_package_manager()`

Those helpers make the registry optional for common plugin classification and naming tasks, which weakens the intended architectural boundary.

### Static rule enforcement is incomplete

`.sentrux/rules.toml` does not yet fully encode the final non-plugin-to-plugin boundary. Some rule families are still too broad or incomplete relative
to the architecture that later ADRs describe.

## Design Principles

- Keep Track A narrow and mechanical.
- Prefer registry-backed access over duplicating plugin knowledge in shared code.
- Add only narrow registry helpers that directly reduce consumer plumbing.
- Do not absorb semantic cleanup into this track.
- Keep follow-up work for Tracks B and C explicit instead of partially hiding it in Track A.

## High-Level Design

Track A has four design units:

1. Registry surface completion
2. Consumer migration from direct plugin imports to registry imports
3. Shared helper retirement on `PluginTypeId`
4. Sentrux rule alignment

The execution order should follow that structure, because consumer migration should not start until the registry exports the minimal supported surface
those consumers need.

## Design Unit 1: Registry Surface Completion

The registry crate must become the sanctioned boundary for non-plugin crates that need plugin metadata, descriptor access, or other static
plugin-facing queries.

Primary files:

- [`crates/plugins/infrastructure/registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs)
- possibly
  [`crates/plugins/infrastructure/registry/src/registry.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/registry.rs)

Design rules:

- For non-plugin crates, direct dependencies on `uptrakit-notification-plugin-core` and leaf plugin crates remain in scope for removal.
- For non-plugin crates that only need metadata, descriptor access, classification, or static plugin-shape queries, the sanctioned boundary in this
  track is `uptrakit-plugin-infrastructure-registry`, not a mix of registry plus direct `infrastructure-core` imports.
- Prefer additive re-exports or narrow convenience methods over broader architectural reshaping.
- If a non-plugin consumer only needs metadata, classification, or a lookup result, expose that through the registry instead of forcing the consumer
  to import descriptor internals.
- Convenience methods are acceptable if they avoid pushing `PluginDescriptor` plumbing into consumers.
- The registry should not become a grab bag for arbitrary semantic policy. Methods added for Track A should stay close to lookup, descriptor access,
  and static plugin-shape questions.

Examples of acceptable Track A additions:

- a registry-backed plugin family/classification query using the existing `PluginFamily` type already re-exported by the registry, preferably
  `plugin_family(plugin_type_id: &PluginTypeId) -> Option<PluginFamily>`
- a registry-backed “supports type settings schema” or similar static descriptor query if needed by existing consumers
- a registry re-export or narrow wrapper for notification-core types that current non-plugin code still imports directly

Track A does not require a registry-backed display-label API. There are currently no known production callers of `PluginTypeId::display_name()`, so
dead-call-site cleanup is sufficient unless migration work exposes a real caller.

Examples that belong in later tracks instead:

- policy methods that decide product behavior based on plugin identity
- plugin-specific special cases for dashboard-icons or similar features

### Explicit Operational Protocol Carve-Out

Some direct `uptrakit-plugin-infrastructure-core` usage in non-plugin crates is not metadata access. It is operational protocol surface used to
construct runtimes, pass batch items and results, use `HostRuntime`/`HostCapabilities`, or implement `agent-infra` traits.

Track A does not attempt to remove those implementation dependencies. They remain an explicit carve-out for:

- [`crates/shared/agent-core/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/Cargo.toml)
- [`crates/shared/agent-core`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core)
- [`crates/shared/scheduler-engine/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/Cargo.toml)
- [`crates/shared/scheduler-engine`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine)
- [`crates/core/agent-ssh/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/Cargo.toml)
- [`crates/core/agent-ssh`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh)

This carve-out is limited to operational protocol types. It does not justify keeping direct leaf-plugin imports, direct notification-core imports, or
metadata/classification helpers outside the registry.

Allowed operational symbol families for the carve-out are:

- runtime construction and runtime traits such as `construct_host_runtime`, `HostRuntime`, and `HostCapabilities`
- batch protocol types such as `BatchDetectItem`, `BatchFetchItem`, `BatchFetchResult`, and `BatchUpdateItem`
- execution and lifecycle protocol types such as `PluginCapability`, `HostCompatibility`, `UpdateLifecycleContext`, and `PluginError`
- `agent-infra` traits and related types such as `InfraBundle`, `InfraActionInvoker`, `InfraPluginContext`, `GuestBootstrapExecutor`,
  `GuestBootstrapParams`, `GuestBootstrapResult`, and `SudoCommandEntry`

Any remaining direct `uptrakit-plugin-infrastructure-core` usage inside the carve-out crates must map to one of those families. Metadata helpers,
plugin-name maps, descriptor-label helpers, and leaf-plugin dispatch do not qualify.

Track B or a separate architectural follow-up should decide whether these protocol types move to a neutral shared crate or remain a formalized
exception.

## Design Unit 2: Consumer Migration

After the registry surface is sufficient, all remaining non-plugin direct imports should move to registry-backed imports.

### UI/query layer

Primary target files:

- [`crates/ui/web-api/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/Cargo.toml)
- [`crates/ui/web-api/src/notifications/message_builder.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/notifications/message_builder.rs)
- [`crates/ui/web-api/src/routes/notifications.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/notifications.rs)
- [`crates/ui/web-api-queries/src/queries/plugin_configs.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/plugin_configs.rs)
- [`crates/ui/web-api-queries/src/queries/discovery_allowlist.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/discovery_allowlist.rs)
- [`crates/ui/web-api-queries/src/queries/notifications.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/notifications.rs)
- [`crates/ui/web-api-queries/src/queries/software_items/crud.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/software_items/crud.rs)
- [`crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs)
- [`crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs)
- [`crates/ui/web-api-queries/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/Cargo.toml)

Scope rules:

- replace direct plugin crate imports with registry imports
- remove plugin crate manifest deps once code no longer needs them
- include the live `uptrakit-notification-plugin-core` imports and manifest dependency in `crates/ui/web-api`
- do not change higher-level query behavior unless required by helper removal

### Shared layer

Primary target files:

- [`crates/shared/agent-core/src/client.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/client.rs)
- [`crates/shared/agent-core/src/update.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/update.rs)
- [`crates/shared/agent-core/src/version_check.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/version_check.rs)
- [`crates/shared/agent-core/src/config_test.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/config_test.rs)
- [`crates/shared/agent-core/src/connection_context.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/connection_context.rs)
- [`crates/shared/scheduler-engine/src/executors/fetch_releases.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/src/executors/fetch_releases.rs)

Scope rules:

- these files are Track A audit targets because many current usages are operational protocol dependencies covered by the explicit carve-out above
- `config_test.rs` is a production module despite its name and should be treated as such during the audit
- migrate only removable metadata/helper/classification usage in this layer; do not force protocol-type migration into Track A
- keep runtime behavior stable
- if helper removal requires a registry query, use the registry directly rather than recreating helper logic locally

### Core layer

Primary target files:

- [`crates/core/controller/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/Cargo.toml)
- [`crates/core/controller/src/main.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src/main.rs)
- [`crates/core/controller/src/ssh_agent/mod.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src/ssh_agent/mod.rs)
- [`crates/core/agent-ssh/src/runtime_support.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/runtime_support.rs)
- [`crates/core/agent-ssh/src/extension.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/extension.rs)
- [`crates/core/agent-ssh/src/commands/bootstrap.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/bootstrap.rs)
- [`crates/core/agent-ssh/src/commands/sync.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/sync.rs)
- [`crates/core/agent-ssh/src/commands/bootstrap_proxmox.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/bootstrap_proxmox.rs)
- [`crates/core/agent-ssh/src/operations/bootstrap.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/bootstrap.rs)
- [`crates/core/agent-ssh/src/operations/sync.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/sync.rs)
- [`crates/core/agent-ssh/src/operations/bootstrap_proxmox.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/bootstrap_proxmox.rs)

Scope rules:

- `crates/core/controller` remains in scope for direct-dependency cleanup where the usage is helper or utility driven
- `crates/core/agent-ssh` is primarily an audit target in Track A because many imports are operational protocol dependencies covered by the explicit
  carve-out
- do not redesign SSH agent workflows in this track

## Design Unit 3: Shared Helper Retirement

The shared `PluginTypeId` helper methods must stop being the place where non-plugin code derives plugin meaning.

Primary file:

- [`crates/shared/types/src/plugin_type_id.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/plugin_type_id.rs)

Required outcome:

- production use of `PluginTypeId::is_package_manager()` is removed
- `PluginTypeId::display_name()` is either deleted as dead API or clearly retained as transitional debt with no production callers

Implementation flexibility:

- the methods may be deleted outright if no needed callers remain
- or they may be retained temporarily but unused in production if a staged migration or compatibility window is needed

Track A preference:

- remove them if that is straightforward after migrating consumers
- otherwise mark them clearly as transitional debt and ensure production code no longer depends on them

Important boundary:

- Track A does not attempt to remove every place where raw `PluginTypeId` values exist
- Track A only removes the shared helper shortcuts that encode plugin-specific knowledge without registry participation
- Non-plugin crates must not replace the retired helpers with new hardcoded plugin-name maps, string-prefix classifiers, or equivalent local logic.
  Any replacement must go through the registry.

## Design Unit 4: Sentrux Rule Alignment

Primary file:

- [`.sentrux/rules.toml`](/Users/andreyyantsen/Development/uptrakit/.sentrux/rules.toml)

Track A should align the rules with the later ADR direction, especially the rule-structure refinements from `TASK-0007`.

Required properties:

- non-plugin families must deny direct dependencies on plugin families
- the registry/catalogue path must remain the sanctioned boundary
- rule families should be explicit enough to cover `releases`, `package-managers`, `hooks`, `notifications`, `enhancements` (that is,
  `crates/plugins/enhancements/**`), `discovery`, and `infrastructure`
- UI and core boundaries must explicitly cover `hooks/**` and `enhancements/**`, which are not optional family omissions for this track
- the rewrite should remain focused on static crate-edge enforcement, not semantic policy

Track A does not need to solve:

- semantic leakage detection
- provenance policy
- allowlist seeding or gate-script productization beyond the rule file itself

## File Map

Expected primary and audit files for this track:

- [`crates/plugins/infrastructure/registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs)
- [`crates/plugins/infrastructure/registry/src/registry.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/registry.rs)
- [`crates/shared/types/src/plugin_type_id.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/plugin_type_id.rs)
- [`crates/ui/web-api/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/Cargo.toml)
- [`crates/ui/web-api/src/notifications/message_builder.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/notifications/message_builder.rs)
- [`crates/ui/web-api/src/routes/notifications.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/notifications.rs)
- [`crates/ui/web-api-queries/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/Cargo.toml)
- [`crates/ui/web-api-queries/src/queries/plugin_configs.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/plugin_configs.rs)
- [`crates/ui/web-api-queries/src/queries/discovery_allowlist.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/discovery_allowlist.rs)
- [`crates/ui/web-api-queries/src/queries/notifications.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/notifications.rs)
- [`crates/ui/web-api-queries/src/queries/software_items/crud.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/software_items/crud.rs)
- [`crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs)
- [`crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs)
- [`crates/shared/agent-core/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/Cargo.toml)
- [`crates/shared/agent-core/src/client.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/client.rs)
- [`crates/shared/agent-core/src/update.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/update.rs)
- [`crates/shared/agent-core/src/version_check.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/version_check.rs)
- [`crates/shared/agent-core/src/config_test.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/config_test.rs)
- [`crates/shared/agent-core/src/connection_context.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/connection_context.rs)
- [`crates/shared/scheduler-engine/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/Cargo.toml)
- [`crates/shared/scheduler-engine/src/executors/fetch_releases.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/src/executors/fetch_releases.rs)
- [`crates/core/controller/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/Cargo.toml)
- [`crates/core/controller/src/main.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src/main.rs)
- [`crates/core/controller/src/ssh_agent/mod.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src/ssh_agent/mod.rs)
- [`crates/core/agent-ssh/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/Cargo.toml)
- [`crates/core/agent-ssh/src/runtime_support.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/runtime_support.rs)
- [`crates/core/agent-ssh/src/extension.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/extension.rs)
- [`crates/core/agent-ssh/src/commands/bootstrap.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/bootstrap.rs)
- [`crates/core/agent-ssh/src/commands/sync.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/sync.rs)
- [`crates/core/agent-ssh/src/commands/bootstrap_proxmox.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/bootstrap_proxmox.rs)
- [`crates/core/agent-ssh/src/operations/bootstrap.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/bootstrap.rs)
- [`crates/core/agent-ssh/src/operations/sync.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/sync.rs)
- [`crates/core/agent-ssh/src/operations/bootstrap_proxmox.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/bootstrap_proxmox.rs)
- [`.sentrux/rules.toml`](/Users/andreyyantsen/Development/uptrakit/.sentrux/rules.toml)

Likely documentation audit targets:

- [`docs/development/plugin-system.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/plugin-system.md)
- [`docs/development/plugin-guidelines.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/plugin-guidelines.md)

## Acceptance Criteria

Track A is complete when all of the following are true:

- outside the explicit operational carve-out, non-plugin production crates no longer import plugin crates directly
- for all workspace packages under `crates/ui/**`, `crates/core/**`, and `crates/shared/**`, excluding the explicit operational carve-out crates
  listed above, `[dependencies]`, `[build-dependencies]`, and target-specific non-dev dependency tables no longer declare direct dependencies on:
  - any `uptrakit-plugin-*` crate other than `uptrakit-plugin-infrastructure-registry`
  - `uptrakit-notification-plugin-core`
- within the explicit operational carve-out crates, the only direct plugin dependency that may remain is `uptrakit-plugin-infrastructure-core`, plus
  `uptrakit-plugin-infrastructure-registry` where those crates still need sanctioned metadata access, and `uptrakit-plugin-infrastructure-core` may
  remain only for operational protocol usage that maps to the allowlist above
- dev-dependency tables are out of scope for Track A; this track is concerned with production dependency tables, build dependency tables, and
  production code
- production use of `PluginTypeId::is_package_manager()` is gone
- `PluginTypeId::display_name()` has no production callers and is either removed or explicitly left as unused transitional debt marked with
  `#[deprecated(note = \"No Track A replacement; remove callers or add a dedicated registry label lookup later\")]`
- non-plugin production crates do not replace those helpers with hardcoded plugin-name maps or string-prefix classifiers
- remaining direct `uptrakit-plugin-infrastructure-core` usage inside the carve-out crates is limited to operational protocol symbols, not metadata
  helpers, plugin-name maps, or leaf-plugin dispatch
- registry-backed replacements compile cleanly
- `.sentrux/rules.toml` reflects the intended static boundary, including `hooks/**` and `enhancements/**`, and `sentrux` passes against the final
  Track A rule set with zero suppressions covering the plugin-boundary rule families; the carve-out must be encoded in the rule definitions
  themselves, not in inline suppression annotations
- development docs do not describe direct non-registry plugin imports as an accepted non-plugin pattern

## Testing Strategy

Verification should stay aligned with the narrow scope:

- repo-wide token scan for direct plugin crate references in Rust source:
  `rg -n 'uptrakit_(plugin_|notification_plugin_core)' crates/ui crates/core crates/shared --glob '*.rs'` Remaining matches must be limited to
  `uptrakit_plugin_infrastructure_registry` and the explicit operational carve-out modules. Matches that occur only inside `#[cfg(test)]` code or
  test-only files are informational and are not Track A failures.
- repo-wide search for direct manifest deps from non-plugin crates:
  `rg -n 'uptrakit-plugin-|uptrakit-notification-plugin-core' crates/ui crates/core crates/shared --glob 'Cargo.toml'` Remaining matches must be
  limited to `uptrakit-plugin-infrastructure-registry` everywhere, plus `uptrakit-plugin-infrastructure-core` only in the explicit operational
  carve-out manifests. When triaging matches, ignore `[dev-dependencies]` sections and fail on `[dependencies]`, `[build-dependencies]`, and
  target-specific non-dev dependency tables.
- repo-wide search for helper regressions and local classifier reintroduction:
  `rg -n 'display_name\\(|is_package_manager\\(|starts_with\\(\"package_manager_\"\\)|package_manager_|plugin_ids::|`
  `releases_|notifications_|hooks_|enhancements_'` `crates/ui crates/core crates/shared` This search is only a signal. It must be paired with manual
  review of changed files that touch plugin classification logic so Track A does not reintroduce helper-equivalent local branching. `plugin_ids::`
  matches are expected and belong to Track B unless they are newly introduced as a replacement for the retired helpers.
- workspace minimal-feature verification: `cargo check --no-default-features --features db-sqlite`
- workspace all-features verification after the frontend build prerequisite is satisfied:
  `cd frontend && npm ci && npm run build && cd .. && cargo check --all-features`
- targeted crate checks for directly touched top-level crates, especially `cargo check -p uptrakit-web-api`,
  `cargo check -p uptrakit-web-api-queries`, and `cargo check -p uptrakit-controller`
- targeted tests for the affected workspace after migration, at minimum `cargo test --all-features`
- targeted unit tests or compile-time coverage for new registry exports or convenience methods
- `sentrux check .` after the rule rewrite
- doc audit for references that still describe direct non-registry plugin dependencies

This track should not require new tests for dashboard-icons behavior, plugin-specific settings flows, or semantic capability policy, because those
belong to later tracks.

## Deferred Follow-Up

### Track B

Track B should handle semantic/plugin-specific leakage, including:

- dashboard-icons settings routes and setting keys
- plugin-specific capability branching in UI, shared, and core flows
- production `plugin_ids::*` usage where those IDs encode behavior
- moving more plugin-specific knowledge behind generic registry or plugin-type-settings surfaces

### Track C

Track C should handle policy and enforcement beyond static crate edges, including:

- semantic leakage detection in CI
- denylist or lint enforcement for helper-pattern regressions
- broader rollout/productization around gate scripts or policy tooling

## Recommended Next Step

Use this design as the spec basis for a task-by-task implementation plan that stays scoped to Track A only. The plan should keep the execution order:

1. registry surface
2. UI/query consumers
3. shared consumers
4. core consumers
5. sentrux rules
6. verification and doc audit
