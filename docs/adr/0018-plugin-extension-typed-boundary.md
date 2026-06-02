# ADR 0018 — Typed Plugin Extension Boundary

Date: 2026-06-02
Status: Accepted (retroactive, ratified by implementation in commits `ce0dba5d3` and `c50be1d01`)

## Context

The previous plugin extension boundary used `dyn Any` for controller access and `Result<_, String>` for
error propagation. Surface-action handlers received an opaque `Box<dyn Any>` that they had to downcast
at runtime, and all validation and integration failures were collapsed to untyped strings. This made
the contract between plugins and the controller invisible to the type system and unreviewable by the
compiler.

The spec `docs/superpowers/specs/2026-04-17-plugin-extension-api-typing-design.md` called for replacing
that boundary with narrow, workflow-scoped controller traits, typed reusable error enums, and a
deterministic mapping from plugin errors to web-edge proxy errors. A first migration wave applied the
new boundary to five plugin families: Proxmox (update-protection and surface-action), email, telegram,
and webhook (notification channels), and Docker (release fetching). These changes shipped on `main` in
commits `ce0dba5d3` and `c50be1d01`; this ADR records the decisions retrospectively.

## Decision

Three narrow, workflow-scoped controller traits were introduced in
`crates/plugins/infrastructure/core/src/roles.rs`:

- **`SurfaceActionController`** — exposes `tenant_id()`, `user_id()`, and (under `plugin-ops`)
  `tenant_db()`. Passed to surface-action handlers via `SurfaceActionContext` in
  `crates/plugins/infrastructure/core/src/descriptor.rs`.
- **`UpdateProtectionController`** — exposes `tenant_db()` for the pre/post update protection workflow.
- **`UpdateHookController`** — exposes `tenant_db()` for the pre/post update hook workflow.

Each trait grants only the identity fields and persistence seam that the corresponding workflow
actually needs, rather than a monolithic god accessor or an untyped `dyn Any` escape hatch.

Three typed, reusable error contracts were introduced:

- **`PluginConfigValidationError`** (`crates/plugins/infrastructure/core/src/plugin_config.rs`) —
  `InvalidField { field, message }`, `InvalidIdentifier`, `Contract` variants; field name is `&'static str`.
- **`SurfaceActionError`** (`crates/plugins/infrastructure/core/src/descriptor.rs`) —
  `InvalidInput`, `ControllerIntegration`, `PluginInternal` variants; all carry `String` payload.
- **`PluginOpsError`** (`crates/plugins/infrastructure/core/src/plugin_ops.rs`) —
  `UnknownPluginType`, `ConfigParse`, `ConfigValidation(PluginConfigValidationError)`.

At the web edge, `map_surface_action_error` in
`crates/ui/surface-proxy/src/proxy/controller_local.rs` maps `SurfaceActionError` to
`SurfaceProxyError`: `InvalidInput` → `SchemaValidationFailed`; `ControllerIntegration` and
`PluginInternal` both collapse to `SendFailed` after emitting a `tracing::error!` event. The collapse
is deliberate (see Consequences).

## Alternatives Considered

### 1. Per-plugin capability stores

Each plugin family would receive a dedicated store interface (e.g. `NotificationChannelStore`,
`EmailSmtpSettingsStore`, `ProxmoxProtectionStore`) exposing only the exact rows that plugin reads or
writes. This would give each plugin the narrowest possible read/write surface.

Rejected because the resulting trait surface scaled linearly with the number of plugins without buying
additional safety over a shared tenant-scoped accessor. The tenant-isolation guarantee is enforced by
`TenantDb` itself — specifically via `TenantDb::find_via_tenant_join` for join-table queries and BEGIN
IMMEDIATE transactions for read-then-write patterns, both of which are codified in
`docs/development/coding-standards.md`. A per-plugin store would duplicate that isolation layer without
eliminating the obligation to follow the same rules at the query site.

### 2. Typed controller traits (chosen)

`TenantDb` is exposed once, via the three workflow-scoped traits. Isolation is not per-plugin but is
enforced per-query by the rules in `docs/development/coding-standards.md`. The trait surface grows
only when a new workflow category is introduced, not when a new plugin is added.

## Consequences

- **Broader persistence surface than per-plugin stores**: `&TenantDb` gives a plugin access to all
  tenant-scoped tables, not only the rows it owns. Reviewers must continue to enforce the tenant
  isolation rules from `docs/development/coding-standards.md` at every plugin query site:
  `TenantDb::find_via_tenant_join` for join-table queries (e.g. `service_host`), and BEGIN IMMEDIATE
  transactions for any read-then-write sequence.

- **Deliberate `ControllerIntegration` → `SendFailed` collapse**: `map_surface_action_error` collapses
  both `ControllerIntegration` and `PluginInternal` to `SurfaceProxyError::SendFailed`. This is
  acceptable because the `tracing::error!` call at the mapping site preserves controller-side failure
  detail in logs before the collapse. A `ControllerIntegration` variant on `SurfaceProxyError` is a
  future option if callers need to distinguish the two cases at the HTTP edge, but doing so would
  require a follow-up ADR.

- **Open observability gap**: no workspace tracing-capture helper exists for asserting the
  `tracing::error!` side effect in tests. The obligation is documented via a `// TODO(adr-0018)` comment
  next to `map_surface_action_error` in
  `crates/ui/surface-proxy/src/proxy/controller_local.rs`. Resolution paths: export a capture helper
  from `uptrakit-tracing-init` under a `test-support` feature, or approve `tracing-test` as a
  `surface-proxy` dev-dependency so the error event can be asserted rather than assumed.

- **Migration scope**: the typed boundary is in place for Proxmox, email, telegram, webhook, and Docker
  plugins. Plugin families added after `c50be1d01` must implement the appropriate controller trait
  rather than using any `dyn Any` pattern.
