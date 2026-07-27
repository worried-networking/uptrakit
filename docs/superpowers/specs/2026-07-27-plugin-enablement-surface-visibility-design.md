# Plugin Enablement Single Source + Surface Visibility Enforcement — Design

**Date:** 2026-07-27
**Status:** Approved (grilling decisions locked with owner)
**Depends on:** interaction registration model (ADR-0028), REST method model (ADR-0030). Identity
grammar (ADR-0031 series) improves — but is not required for — provider-id resolution (see §4).
**Non-goals:** hot-reload of plugin singletons; plugin identity-string redesign; feature-flag
unification (ADR-0032 spec); surface interaction protocol changes; removal of the unwired orphan
files under `proxy/` — `prepared.rs`, `validation.rs`, `bookkeeping.rs`, `dispatch.rs`,
`idempotency.rs` are all undeclared in any `mod` statement (separate pending spec; note
`validation.rs` carries its own stale `NoTenantCompatibleProvider` mapping — a re-wiring trap).

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

| #   | Decision               | Choice                                                                                                                                                                                                                                                                                                                                                                                            |
| --- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Enablement model       | **Effective = boot ∧ live.** A plugin is tenant-effective only when the boot catalog constructed it AND the live snapshot says enabled. Disable takes effect immediately; enable stays pending-restart (existing badge). ADR-0006 Decision 2 (no hot-reload) stands.                                                                                                                              |
| D2  | Enforcement placement  | **Registry takes a required visibility filter parameter.** Tenant-facing `SurfaceRegistry` enumeration/resolution methods gain a caller-supplied filter applied during provider enumeration; every leg inherits it structurally. `SurfaceProxy` stores the same filter at construction (its own builder idiom, deny-by-default — §3.4). Dependency direction unchanged (web-api → surface-proxy). |
| D3  | Fail posture           | **Fail-closed.** A Plugin-kind provider whose `provider_id` resolves to no descriptor is not visible. No `.unwrap_or(true)` anywhere on the surfaces path (snapshot rule: never `unwrap_or` in security paths).                                                                                                                                                                                   |
| D4  | Absent-row default     | **Absent = disabled, no seeding.** Documented as deliberate; no boot write, no migration. Admin UI lists Instance plugins from descriptors regardless of row.                                                                                                                                                                                                                                     |
| D5  | `delete_channel`       | **Stays unguarded, documented.** Deletion is cleanup and must work for channels whose plugin type is no longer compiled in. The dispatch-leg gate (D2) covers the disabled-plugin path upstream.                                                                                                                                                                                                  |
| D6  | Admin tier on surfaces | **No admin override on surfaces legs.** Surface availability is `effective_enabled` for every tier. The `ManageGlobalSettings` override remains only on plugin-listing/config endpoints (predicate call sites). Admin management story = instance-plugins routes + pending-restart badge, never a broken surface page.                                                                            |
| D7  | ADR handling           | **New ADR-0033** records the effective-enablement model and structural enforcement; **ADR-0006 is amended** (status note pointing at 0033; Decisions 1–3 unchanged, Decision 4's "single predicate gates … the surfaces registry" superseded by the two-gate model below).                                                                                                                        |

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

Coordination note (ADR-0032 contribution-monotonicity): the boot-enablement filter lives **only**
on the catalog's runtime `surface_registrations()`; monotonicity guards read descriptor-level
builders (`(ops.registrations)()` / `all_descriptors()`) and must keep doing so — pointing a
presence guard at the catalog's filtered output would let the boot gate silently defeat it. The
synthetic test descriptor stays test-local and never enters `all_descriptors()`. Record this in
ADR-0033.

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
- `resolve_surface_read` and `resolve_surface_action_for_method` thread the same parameter. A
  required parameter (not a default) makes it impossible to resolve without deciding visibility —
  the drift-proofing D2 exists for.
- `SurfaceProxy` follows its own existing composition idiom instead of a per-call parameter: the
  filter is stored at construction via a builder step
  (`with_provider_visibility(Arc<dyn SurfaceProviderVisibility>)`, mirroring the existing
  `with_local_executor`), and `invoke`/`invoke_inner` use the stored filter for their internal
  resolution. **The stored filter is the only gate on the provider-origin leg** (`invoke_inner`
  re-resolves internally with no `AppState` in scope), so `PluginEffectiveEnablement` must hold
  the **live handles** — `Arc<dyn PluginOps>` plus the `Arc<ArcSwap<InstancePluginSnapshot>>`
  itself — and `.load()` the snapshot on every `plugin_provider_visible` call. Capturing a loaded
  `Arc<InstancePluginSnapshot>` at construction would freeze the filter at boot state and break
  D1's disable-is-immediate on exactly this leg (boot ordering is favorable: the ArcSwap exists
  before the proxy is built in `boot/components.rs`). Deliberate divergence from the
  `local_executor` precedent: the **default is deny-all-plugin-providers** (fail-closed), not a
  permissive no-op — a proxy constructed without the production wiring must hide plugin surfaces,
  never serve them ungated; the `AppStateBuilder` fallback (`app_state.rs`
  `.unwrap_or_else(|| Arc::new(SurfaceProxy::new()))`) then hides rather than leaks, and the plan
  must confirm that fallback is unreachable in the real controller boot. Service/BuiltIn
  providers are unaffected by the default. Test blast radius: the deny default is a
  **behavioral** change on the type, not just a signature change — the plan must grep and triage
  **all** `SurfaceProxy::new()` sites in `crates/ui/web-api` **and** `crates/ui/surface-proxy`
  (surface-proxy's own `proxy/tests/controller_local.rs` + `controller_owned/*` suites construct
  Plugin-kind ControllerLocal fixtures that flip RED under deny-all), routing surface-exercising
  ones through `AllProvidersVisible`.
- web-api implements the trait once (`PluginEffectiveEnablement`, backed by §3.2) and supplies it
  at every call site: `list_surfaces`, `list_surface_providers`, `read_surface`, the invoke
  handler, and the proxy construction in boot wiring. The service-WS provider-origin path
  (`routes/service_ws/handler/message_processor.rs` — resolves via
  `resolve_surface_action_for_method`, then invokes through `surface_proxy_deps`) supplies the
  filter for its own pre-resolution call, but its enforcement backstop is the proxy's **stored**
  live-handle filter, not `AppState` availability. Ledger discipline: the plan must enumerate
  **every** production caller of the changed registry methods and of `SurfaceProxy::invoke`
  workspace-wide (a reviewer pass found all current callers inside `crates/ui/web-api`;
  `uptrakit_service_sdk::ServiceSurfaceProxy` is an unrelated same-named type) — re-run the
  inventory grep at plan time, not a memory list. The orphan `proxy/` files (see Non-goals) must
  not be cited or edited.
- A permissive impl for tests lives in `surface-proxy`'s existing test support
  (`AllProvidersVisible`, `#[cfg(test)]` or the crate's testing feature) so unrelated registry
  tests stay focused; production callers all pass the real filter.
- `enforce_required_permission` (`routes/surfaces.rs`) keeps its dynamic
  `has_permission()` call: descriptor-declared permission strings are runtime data, so the typed
  `permission_extractor!` cannot express them. This is the already-documented **"Runtime-valued
  (surfaces)"** exception class in `docs/security/auth-and-authorization.md` (a class distinct
  from `// APPROVED: custom auth path`, which is reserved for bespoke token-extraction handlers)
  — its sanctioned marker is the `x-required-permission: "dynamic: …"` OpenAPI extension the
  surfaces route family already carries. No code marker is added; re-typing the gate to a typed
  permission value is deferred to the access-management refactoring (already tracked there).

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

`running_enabled` on the admin summary (boot state via `instance_enabled`) is **intentionally
unchanged** by a live-disable — only the surface/transport gate closes; the boot-constructed
singleton stays loaded (ADR-0006 Decision 2). Do not "fix" `running_enabled` to track live state:
that would break the pending-restart badge and mask a genuine restart-needed condition. The UI
renders primary state from `enabled` (desired).

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
2. **Harness wiring (reuse first, then the real gaps).** The production bootstrap loop already
   exists in the harness: `test_harness/mod.rs::build_test_state_with_plugin_surfaces` mirrors
   the boot wiring (bootstraps every `surface_registrations()` entry + real
   `PluginSurfaceLocalExecutor`), and `build_test_state_with_plugin_ops` accepts a `plugin_ops`
   override — do **not** rewrite these. The actual new deliverables are: (a) a `TestApp`
   construction path that reaches `build_test_state_with_plugin_surfaces` (today only
   `build_test_state` is reachable from `TestApp::new()`), (b) an override parameter **added to**
   `build_test_state_with_plugin_surfaces` and forwarded into its internal
   `build_test_state_with_plugin_ops` call — today it hardcodes `None`, so the existing override
   (`Option<Arc<dyn PluginOps>>`, which a synthetic `PluginCatalog::new` with the §5.1 fixture +
   chosen `InstancePluginStates` satisfies) is unreachable from the surfaces-wired builder — and
   (c) the proxy's `with_provider_visibility` wiring in that path. Route-level tests use
   `TestApp`/`TestClient` per the harness rule.
3. **Per-leg matrix tests** (route level, success + failure): each row of §3.5 against
   `list_surfaces`, `list_surface_providers`, `read_surface`, invoke — asserting both presence
   and the exact 404 shape, for a non-admin and an admin user. The 404 assertion is
   **byte-identical body** to the unknown-surface response on `list_surface_providers`, the
   invoke path, **and `read_surface`** (read maps through the non-collapsing `map_lookup_error`
   and its empty-check is being re-derived per §3.4 — the leg most exposed to a filter-ordering
   slip) — this pins the requirement that the visibility drop precedes the tenant-compat
   classification, so an all-invisible surface can never surface the distinct
   `NoTenantCompatibleProvider` message. The live-toggle rows flip via the
   instance-plugins route (not by poking the ArcSwap) so the test drives the production write
   path.
4. **Provider-origin tests**: service-WS-shaped invocation against a live-disabled plugin is
   denied without any `AuthenticatedUser` involved — two cases: (a) statically disabled at
   harness build, and (b) **toggle-then-invoke without restart**: disable via the
   instance-plugins route, then fire the provider-origin invocation and assert denial — this
   RED-catches a filter frozen at construction (§3.4 live-handle requirement).
5. **Catalog guard test** (§3.3) including the RED case: a registration whose dispatch key is
   absent must fail the guard (perturb a value, not delete a symbol — dead-code deny would mask
   the RED).
6. **`delete_channel` test** (§3.6).
7. **Predicate call-site regression**: existing `plugin_type_settings` / `plugin_configs`
   integration tests keep passing with the effective-enablement input; one new case pins the
   pending-restart-enabled row (live=enabled, boot=disabled ⇒ hidden from non-admin).

## 6. Documentation deliverables

| Artifact                                                              | Change                                                                                                                                                                                                                      |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/adr/0033-effective-plugin-enablement-and-surface-visibility.md` | new ADR: D1–D7, two-gate model, per-tier matrix; scope note: D1's disable-is-immediate is per-process (in-memory ArcSwap, single-controller assumption — no cross-replica propagation); §3.3 monotonicity coordination note |
| `docs/adr/0006-instance-scoped-plugins.md`                            | status note: Decision 4 superseded by ADR-0033; Decision 2 refined (effective = boot ∧ live)                                                                                                                                |
| `docs/security/surfaces.md`                                           | surfaces read/list/invoke/provider-origin are gated on effective enablement; fail-closed posture; 404 shape                                                                                                                 |
| `docs/development/surfaces.md`                                        | `SurfaceProviderVisibility` parameter contract for registry consumers                                                                                                                                                       |
| `docs/development/plugin-system.md`                                   | effective-enablement semantics; absent-row default is deliberate (D4)                                                                                                                                                       |
| `docs/development/notifications.md`                                   | `delete_channel` divergence rationale (D5)                                                                                                                                                                                  |
| `CONTEXT.md`                                                          | glossary: "Effective Enablement"                                                                                                                                                                                            |
| `crates/ui/web-api/openapi.json` + generated SDK                      | regen only if route annotations change (`list_surface_providers` extension text becomes true rather than aspirational; run `./scripts/regen-api.sh` if any `#[utoipa::path]` text is touched)                               |

No wire-protocol change (no new payloads; provider-origin gating is a controller-side decision),
so `asyncapi.yaml` is untouched.

## 7. Alternatives rejected

- **Full hot-reload** (rebuild catalog on toggle): reverses ADR-0006 Decision 2; singleton
  teardown + in-flight safety + background-task cancel compose non-trivially; not forced by the
  use case.
- **Strict restart-required** (both legs read boot only): disabling a misbehaving plugin would
  have zero runtime effect until restart — wrong fail direction. (Scope honesty: live-disable
  closes the surface-dispatch and transport gates only; it is not a kill switch — a
  boot-constructed singleton's background tasks run until restart, per the hot-reload non-goal.)
- **Registry learns auth types**: inverts the dependency direction (surface-proxy ← web-api auth
  types); cycle or duplication.
- **Per-leg call sites + CI grep gate**: keeps the drift class that produced this bug; the prior
  docs-only mitigation for a same-class bug in this area is already disproven by recurrence.
- **`provider_to_type` index on the catalog**: rejected in a prior review round —
  `PluginMetadataOps::all()` already exposes the needed set, and an index couples correctness to
  string literals a rename would rewrite.

## 8. Verification (implementation gates)

- Canonical gate set, both feature worlds: `cargo check --no-default-features --features
db-sqlite`, `cargo check --all-features`, `cargo clippy --all-targets --no-default-features
--features db-sqlite`, `cargo clippy --all-targets --all-features`, `cargo test --all-features`
  (the `--all-features` forms need `frontend/build` first). The `--all-features` world is
  load-bearing here, not optional: the changed registry/proxy signatures have feature-gated call
  sites (`notifications-*`, `dashboard-icons`, `embed-frontend`) invisible to the minimal world.
  Full-workspace test run required for the signature change (crate-scoped `-p` runs cannot see
  cross-crate golden/fixture breakage).
- `python3 ci/verify_db_access_policy.py`, `bash ci/verify_handler_state_contract.sh` (surfaces
  handlers change), `cargo xtask audit-coverage-check` if any audit-emitting handler moves.
- Grep gates: zero `.unwrap_or(true)` in `routes/surfaces.rs` visibility paths; zero references
  to the orphan `proxy/` files (`prepared.rs`, `validation.rs`, `bookkeeping.rs`, `dispatch.rs`,
  `idempotency.rs`) in the diff.
