# Plugin Enablement Single Source + Surface Visibility Enforcement — Design

**Date:** 2026-07-27
**Status:** Approved (grilling decisions locked with owner)
**Depends on:** interaction registration model (ADR-0028), REST method model (ADR-0030). Identity
grammar (ADR-0031 series) improves — but is not required for — provider-id resolution (see §4).
**Non-goals:** hot-reload of plugin singletons; plugin identity-string redesign; feature-flag
unification (ADR-0032 spec); surface interaction protocol changes; `proxy/prepared.rs` +
`proxy/validation.rs` orphan removal (separate pending spec).

## 1. Problem

Two sources of truth for Instance-scoped plugin enablement diverge after any runtime toggle, and
the surface-visibility invariant of ADR-0006 Decision 4 is enforced on none of the surfaces legs.

### Verified reality

- `PluginCatalog::instance_states` (`crates/plugins/infrastructure/core/src/catalog.rs`,
  `InstancePluginStates`) is snapshotted at `PluginCatalog::new` (boot). The disabled-Instance
  `continue` in the constructor skips singleton construction **and** `surface_dispatch`
  population, but the descriptor stays in the index (in-code comment: visibility predicates must
  still see it).
- `PluginSurfaceOps::surface_registrations()` iterates `descriptors.values()` with **no** enabled
  filter — boot-disabled plugins' surfaces ARE bootstrapped into the `SurfaceRegistry` while their
  dispatch entries are not. Result: a visible surface whose every interaction fails
  `no registered interaction … on surface …` (`PluginSurfaceActionOps::handle_surface_action`).
- `AppState::instance_plugin_snapshot` (`ArcSwap<InstancePluginSnapshot>`) is updated at runtime
  by the instance-plugins routes (`routes/instance_plugins.rs` `.store(...)` sites); the catalog
  is not. `test_harness/fixtures.rs::upsert_instance_plugin_setting` documents the independence.
- The visibility predicate `crate::visibility::is_plugin_visible_to_user` **does** run in
  production at `routes/plugin_type_settings.rs` (4 sites) and `routes/plugin_configs/crud.rs`,
  keyed correctly by `type_id`. It has never run effectively on the surfaces leg:
  `routes/surfaces.rs::list_surfaces` keys the descriptor lookup with
  `PluginTypeId::new(&item.provider_id)` where `provider_id` is the descriptor's declared surface
  provider id (`plugin.<…>` grammar), so the lookup always misses and falls to `.unwrap_or(true)`.
- Tenant-facing surface legs and their current gating:
  - `list_surfaces` — predicate called with wrong key (dead), fail-open.
  - `list_surface_providers` — no visibility filter at all, despite its OpenAPI
    `x-required-permission` extension claiming "results filtered by descriptor visibility".
  - `read_surface` (`SurfaceRegistry::resolve_surface_read`) — permission checks only.
  - invoke (`SurfaceRegistry::resolve_surface_action_for_method` via the HTTP handler) —
    permission checks only (`enforce_required_permission` is not visibility-aware).
  - provider-origin invocation (`SurfaceProxy::invoke` → `invoke_inner`, service-WS initiated) —
    no visibility or enablement gate; no `AuthenticatedUser` exists on this path.
- Tier-1 `ControllerExecutor` notification CRUD checks `plugin_ops.transport(...).is_none()` on
  create/update/test (`web-api-queries/src/queries/notifications.rs::create_channel_in_tx` and
  siblings); `delete_channel` performs a bare tenant-scoped delete with no such check.
- Both stores agree that an absent `instance_plugin_setting` row means disabled
  (`InstancePluginStates::enabled` and `InstancePluginSnapshot::enabled` both default `false`).
- Dormant today only because no in-repo plugin is both `PluginScope::Instance` and
  surface-bearing (`dashboard-icons` is Instance-scoped, declares no surfaces). Nothing couples
  `scope` to `surfaces`; the only cross-field invariant is Tenant-can't-declare-`instance_config`.

## 2. Decisions (locked)

| #   | Decision               | Choice                                                                                                                                                                                                                                                                                                                 |
| --- | ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Enablement model       | **Effective = boot ∧ live.** A plugin is tenant-effective only when the boot catalog constructed it AND the live snapshot says enabled. Disable takes effect immediately; enable stays pending-restart (existing badge). ADR-0006 Decision 2 (no hot-reload) stands.                                                   |
| D2  | Enforcement placement  | **Registry takes a required visibility filter parameter.** Tenant-facing `SurfaceRegistry` enumeration/resolution methods gain a caller-supplied filter applied during provider enumeration; every leg inherits it structurally. Dependency direction unchanged (web-api → surface-proxy).                             |
| D3  | Fail posture           | **Fail-closed.** A Plugin-kind provider whose `provider_id` resolves to no descriptor is not visible. No `.unwrap_or(true)` anywhere on the surfaces path (snapshot rule: never `unwrap_or` in security paths).                                                                                                        |
| D4  | Absent-row default     | **Absent = disabled, no seeding.** Documented as deliberate; no boot write, no migration. Admin UI lists Instance plugins from descriptors regardless of row.                                                                                                                                                          |
| D5  | `delete_channel`       | **Stays unguarded, documented.** Deletion is cleanup and must work for channels whose plugin type is no longer compiled in. The dispatch-leg gate (D2) covers the disabled-plugin path upstream.                                                                                                                       |
| D6  | Admin tier on surfaces | **No admin override on surfaces legs.** Surface availability is `effective_enabled` for every tier. The `ManageGlobalSettings` override remains only on plugin-listing/config endpoints (predicate call sites). Admin management story = instance-plugins routes + pending-restart badge, never a broken surface page. |
| D7  | ADR handling           | **New ADR-0033** records the effective-enablement model and structural enforcement; **ADR-0006 is amended** (status note pointing at 0033; Decisions 1–3 unchanged, Decision 4's "single predicate gates … the surfaces registry" superseded by the two-gate model below).                                             |

## 3. Design

### 3.1 Two distinct questions, two gates

The design separates concerns ADR-0006 conflated:

- **"May this user know the plugin exists?"** — per-user, answered by
  `is_plugin_visible_to_user` (unchanged shape, admin override intact). Governs plugin-listing and
  config endpoints only.
- **"Is this plugin's surface functionality live?"** — user-independent, answered by **effective
  enablement**: `Tenant` scope → always; `Instance` scope → catalog boot state ∧ live snapshot;
  unknown plugin → `false`. Governs every surfaces leg, all tiers, including provider-origin.

Because the surfaces gate is user-independent, one filter implementation serves the HTTP legs and
the provider-origin leg alike — no auth types leak into `surface-proxy`.

### 3.2 Effective enablement accessor (web-api)

One new small view in `uptrakit-web-api` (module `visibility.rs`), built from the two existing
sources already on `AppState` — per the prior review lesson, **no new derived index**; descriptor
resolution uses the existing `PluginMetadataOps::all()` / `get()` accessors:

- `effective_instance_enabled(plugin_ops, snapshot, type_id) -> bool`:
  `Tenant` → `true`; `Instance` → `plugin_ops.instance_enabled(type_id) && snapshot.enabled(type_id)`;
  unknown `type_id` → `false`. (`instance_enabled` already returns boot state for Instance scope
  and `false` for unknown; the snapshot must only be consulted in the `Instance` arm — Tenant
  plugins have no row.)
- Surface-provider resolution: `provider_id` → descriptor by scanning `plugin_ops.all()` for
  `descriptor.surfaces` whose declared `provider_id` matches (exact string equality; small N).
  No match on a Plugin-kind provider → not visible (D3).
- `is_plugin_visible_to_user`'s `Instance` arm switches its `enabled` input from
  `snapshot.enabled(...)` to the effective value, so listing/config endpoints stop showing
  pending-restart-enabled plugins to non-admins. All predicate call sites
  (`plugin_type_settings.rs`, `plugin_configs/crud.rs`, `surfaces.rs`) are swept in the same
  change — the signature change makes the compiler enumerate them; the plan must still re-grep.

### 3.3 Catalog: registration follows construction

`PluginSurfaceOps::surface_registrations()` gains the same gate as the constructor: skip
Instance-scoped descriptors whose boot state is disabled. Registration and dispatch then derive
from the same boot decision — the visible-but-undispatchable class dies at the root for the
boot-disabled case. The descriptor index itself stays complete (listing predicate needs it).

New guard test in `infrastructure-core` (catalog tests): build a catalog containing a synthetic
Instance-scoped surface-bearing descriptor in both boot states and assert
**registration ⊆ dispatch**: for every `PluginHandled` interaction in `surface_registrations()`,
`surface_dispatch` contains the `(surface, action, method)` key; and for the boot-disabled state,
the plugin contributes zero registrations. Import `PluginSurfaceOps` explicitly (trait method, not
inherent — prior snippet lesson).

### 3.4 Registry: required visibility filter

In `uptrakit-surface-proxy`:

```rust
/// Decides whether a plugin-backed surface provider is currently servable.
/// Service- and BuiltIn-kind providers are not consulted.
pub trait SurfaceProviderVisibility: Send + Sync {
    fn plugin_provider_visible(&self, provider_id: &str) -> bool;
}
```

- `list_surfaces_for_tenant` and `list_targeted_providers_for_surface` gain a
  `visibility: &dyn SurfaceProviderVisibility` parameter and drop invisible Plugin-kind providers
  during enumeration — removed as if never registered, **not** flagged tenant-incompatible.
  Consequence: `resolve_surface_read` / `resolve_surface_action_for_method` (which build on the
  provider enumeration) collapse an all-providers-filtered surface to the `SurfaceNotFound`
  error shape, preserving ADR-0006's no-existence-side-channel (404 indistinguishable from
  unknown surface; never the distinct `NoTenantCompatibleProvider` message). The empty-check in
  `resolve_surface_read` that today reads raw `surface_to_providers` before filtering must be
  re-derived from the post-filter set.
- `resolve_surface_read`, `resolve_surface_action_for_method`, and `SurfaceProxy::invoke` /
  `invoke_inner` thread the same parameter. A required parameter (not a default) makes it
  impossible to resolve without deciding visibility — the drift-proofing D2 exists for.
- web-api implements the trait once (`PluginEffectiveEnablement`, backed by §3.2) and passes it
  at every call site: `list_surfaces`, `list_surface_providers`, `read_surface`, the invoke
  handler, and the service-WS provider-origin path. Ledger discipline: the plan must enumerate
  **every** production caller of the changed registry methods and of `SurfaceProxy::invoke`
  workspace-wide (including `agent-ssh-runtime` if it constructs a registry) — an inventory step,
  not a memory list. `proxy/prepared.rs` / `proxy/validation.rs` are unwired orphans; do not cite
  or edit them.
- A permissive impl for tests lives in `surface-proxy`'s existing test support
  (`AllProvidersVisible`, `#[cfg(test)]` or the crate's testing feature) so unrelated registry
  tests stay focused; production callers all pass the real filter.

### 3.5 Per-tier outcome matrix (the contract the tests pin)

For an Instance-scoped surface-bearing plugin:

| Boot     | Live              | Non-admin surfaces legs  | Admin surfaces legs | Admin instance-plugins UI |
| -------- | ----------------- | ------------------------ | ------------------- | ------------------------- |
| enabled  | enabled           | listed + dispatchable    | same                | enabled                   |
| enabled  | disabled          | absent + 404 (immediate) | same                | disabled (took effect)    |
| disabled | enabled           | absent + 404             | same                | pending restart badge     |
| disabled | disabled / no row | absent + 404             | same                | disabled                  |

`delete_channel` remains reachable for existing rows regardless of plugin state via its own
permission-gated route path — but the surface-dispatch route to it 404s when the owning plugin is
not effective, like every other interaction.

### 3.6 `delete_channel` divergence (D5)

Documented in `docs/development/notifications.md`: create/update/test validate against a live
transport because they must interpret config; delete is cleanup and intentionally works for
unknown/no-longer-compiled channel types (otherwise rows orphan permanently). New test pins
delete-succeeds-for-unknown-channel-type.

## 4. Dependency note (fail-closed vs identity)

D3's fail-closed posture is safe _now_ because every first-party surface-bearing plugin declares
its `provider_id` on the descriptor's surfaces declaration — resolution is exact string match
against compiled-in descriptors, so the miss case is a genuine inconsistency, not a routine
event. The ADR-0031-series identity work tightens the grammar but this spec does not block on it.
If that work renames provider ids, the resolution in §3.2 keys on the descriptor's own declared
value and follows automatically; only literals in tests/docs need the rename sweep.

## 5. Testing

Feature world: all commands below run under the canonical
`--no-default-features --features db-sqlite` world unless stated; the new fixtures must **not**
hide behind `feature = "dashboard-icons"` (the existing `upsert_instance_plugin_setting` fixture
is gated on it — the new synthetic-plugin fixtures live outside that gate).

1. **Synthetic descriptor fixture.** A `'static` Instance-scoped surface-bearing
   `PluginDescriptor` (OnceLock-leaked, mirroring `visibility.rs` test fixtures) with one
   `PluginHandled` interaction, in `infrastructure-core`'s testing support so both the catalog
   guard test (§3.3) and web-api tests consume one fixture.
2. **Harness wiring (named deliverables, not assumed).** `TestApp` today neither injects extra
   descriptors into the catalog nor bootstraps plugin registrations into its `SurfaceRegistry`
   (three known production/harness divergences recorded in the mistakes ledger). This spec adds:
   a `TestApp` construction option taking extra descriptors + `InstancePluginStates`, and a
   fixture that runs the production bootstrap loop (`surface_registrations()` →
   `bootstrap_plugin`) against the harness registry. Route-level tests use `TestApp`/`TestClient`
   per the harness rule.
3. **Per-leg matrix tests** (route level, success + failure): each row of §3.5 against
   `list_surfaces`, `list_surface_providers`, `read_surface`, invoke — asserting both presence
   and the exact 404 shape (same body as unknown surface), for a non-admin and an admin user.
   The live-toggle rows flip via the instance-plugins route (not by poking the ArcSwap) so the
   test drives the production write path.
4. **Provider-origin test**: service-WS-shaped invocation against a live-disabled plugin is
   denied without any `AuthenticatedUser` involved.
5. **Catalog guard test** (§3.3) including the RED case: a registration whose dispatch key is
   absent must fail the guard (perturb a value, not delete a symbol — dead-code deny would mask
   the RED).
6. **`delete_channel` test** (§3.6).
7. **Predicate call-site regression**: existing `plugin_type_settings` / `plugin_configs`
   integration tests keep passing with the effective-enablement input; one new case pins the
   pending-restart-enabled row (live=enabled, boot=disabled ⇒ hidden from non-admin).

## 6. Documentation deliverables

| Artifact                                                              | Change                                                                                                                                                                                        |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/adr/0033-effective-plugin-enablement-and-surface-visibility.md` | new ADR: D1–D7, two-gate model, per-tier matrix                                                                                                                                               |
| `docs/adr/0006-instance-scoped-plugins.md`                            | status note: Decision 4 superseded by ADR-0033; Decision 2 refined (effective = boot ∧ live)                                                                                                  |
| `docs/security/surfaces.md`                                           | surfaces read/list/invoke/provider-origin are gated on effective enablement; fail-closed posture; 404 shape                                                                                   |
| `docs/development/surfaces.md`                                        | `SurfaceProviderVisibility` parameter contract for registry consumers                                                                                                                         |
| `docs/development/plugin-system.md`                                   | effective-enablement semantics; absent-row default is deliberate (D4)                                                                                                                         |
| `docs/development/notifications.md`                                   | `delete_channel` divergence rationale (D5)                                                                                                                                                    |
| `CONTEXT.md`                                                          | glossary: "Effective Enablement"                                                                                                                                                              |
| `crates/ui/web-api/openapi.json` + generated SDK                      | regen only if route annotations change (`list_surface_providers` extension text becomes true rather than aspirational; run `./scripts/regen-api.sh` if any `#[utoipa::path]` text is touched) |

No wire-protocol change (no new payloads; provider-origin gating is a controller-side decision),
so `asyncapi.yaml` is untouched.

## 7. Alternatives rejected

- **Full hot-reload** (rebuild catalog on toggle): reverses ADR-0006 Decision 2; singleton
  teardown + in-flight safety + background-task cancel compose non-trivially; not forced by the
  use case.
- **Strict restart-required** (both legs read boot only): disabling a misbehaving plugin would
  have zero runtime effect until restart — wrong fail direction.
- **Registry learns auth types**: inverts the dependency direction (surface-proxy ← web-api auth
  types); cycle or duplication.
- **Per-leg call sites + CI grep gate**: keeps the drift class that produced this bug; the prior
  docs-only mitigation for a same-class bug in this area is already disproven by recurrence.
- **`provider_to_type` index on the catalog**: rejected in a prior review round —
  `PluginMetadataOps::all()` already exposes the needed set, and an index couples correctness to
  string literals a rename would rewrite.

## 8. Verification (implementation gates)

- `cargo check/clippy --all-targets --no-default-features --features db-sqlite` (workspace) and
  `cargo test --all-features` (needs `frontend/build`); full-workspace test run required for the
  registry signature change (crate-scoped `-p` runs cannot see cross-crate golden/fixture
  breakage).
- `python3 ci/verify_db_access_policy.py`, `bash ci/verify_handler_state_contract.sh` (surfaces
  handlers change), `cargo xtask audit-coverage-check` if any audit-emitting handler moves.
- Grep gates: zero `.unwrap_or(true)` in `routes/surfaces.rs` visibility paths; zero references
  to `proxy/prepared.rs` in the diff.
