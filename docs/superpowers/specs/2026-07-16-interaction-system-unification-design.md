# Interaction System Unification — Single-Source Plugin Surface Registrations

Date: 2026-07-16
Status: Draft (pending implementation plan)
Origin: D9 follow-up of
[2026-07-15-proxmox-guest-flow-provider-invocable-design.md](2026-07-15-proxmox-guest-flow-provider-invocable-design.md).

## Problem

The controller carries two parallel per-plugin interaction declarations:

- **(a) Legacy**: `SurfaceActionDescriptor` lists + a `handle_action` fn pointer, declared via
  `declare_plugin!(surface_actions: { actions, handle_action })` plus `owned_surface_ids`, prefix-routed by
  `PluginCatalog` (`crates/plugins/infrastructure/core/src/catalog.rs`, route build ~`:272-318`).
- **(b) Registered**: hand-authored `InteractionDescriptor`s in each plugin's `*_surface_registrations()`, declared via
  `declare_plugin!(surfaces: { registrations })`, consumed by `SurfaceRegistry`.

Only (b) gates resolvability; only (a) drives dispatch. Nothing links them — that gap is how `list-all-unmatched`
stayed dispatchable-but-unresolvable for 15 months. The stopgap is the parity guard test
`every_legacy_dispatchable_action_is_a_registered_interaction`
(`crates/plugins/infrastructure/proxmox/src/surfaces.rs`, ~`:2843`), which covers only the proxmox crate.

### Verified current state (2026-07-16, line numbers are hints — anchor by symbol)

**The legacy `.actions` lists are production-dead on the controller.** `SurfaceActionLibrary`
(`descriptor.rs:220-224`: `{ actions, owned_surface_ids, handle_action }`) is consumed by:

- `catalog.rs:273` — reads only `handle_action` + `owned_surface_ids` (longest-prefix `starts_with` routing,
  `route_surface_action` ~`:313-318`); never calls `(ext.actions)()`.
- agent-ssh runtime `build_actions()` (`crates/core/agent-ssh-runtime/src/surface_runtime.rs:163-203`) — the **only**
  live `.actions` consumer; collects Infrastructure-family actions (proxmox agent's `discovered-guests` +
  `bootstrap-proxmox-guest` under feature `agent-infra`), weaves them into the runtime's own `ssh-agent.hosts` surface,
  and converts to registered `InteractionDescriptor`s in `build_interactions()` (`:560-635`).
- Tests (the parity guard; per-plugin `surface_actions_not_empty`-style unit tests).

Nothing serves `SurfaceActionDescriptor` to the frontend or HTTP API: zero hits in `web-api-types`, `openapi.json`,
`frontend/src`. Consequences, each verified:

- All 13 production `.with_api_submit(...)` sites (proxmox `add-config`, webhook/telegram/email CRUD actions) are dead
  data. The live "Add Plugin Config" flow is the built-in settings page
  (`frontend/src/routes/settings/PluginConfigsTab.svelte` → generated-SDK `createPluginConfig`), not surface-driven.
- The local-executor Tier 1b allowlist (`("proxmox.hosts", "add-config")`,
  `crates/ui/surface-proxy/src/proxy/controller_local/proxmox_add_config.rs`) is unreachable: no `add-config`
  interaction is registered, so `SurfaceRegistry::resolve_surface_action` fails with `InteractionNotFound` before the
  executor runs. Deleting it does not change audit posture — plugin-config creation flows through the REST route today
  in every deployment.
- `row_visible_when`, `batch_action`, `icon` on **controller**-plugin actions feed nothing. On **agent**-collected
  actions they are live: the runtime maps `row_visible_when` into `SurfaceTableRowAction.visible_when`
  (`surface_runtime.rs:319-323`; wire field on `crates/shared/surfaces/src/surface.rs:244`) and uses `batch`/`icon` in
  surface assembly.

**Dispatch is heterogeneous.** `ControllerSurfaceAction` (17 variants) + `resolve_controller_surface_action` exist
only in proxmox (`surfaces.rs:145`, `:165`). Docker matches `(surface_id, action_id)` tuples
(`docker/src/surfaces.rs:140`); webhook/telegram/email match `action_id` only — and only a **subset** of their action
lists (webhook's `handle_action` matches only `"list"`). Their `create`/`edit`/`test`/`delete` interactions are
executed by `local_executor.rs` **Tier 1a** directly (controller-side code + audit), never reaching the plugin. One
action is registered on two surfaces: `load-backup-target-options` on `proxmox.settings.update-hooks` and
`proxmox.software-item.update-hooks` (OR-pattern arm, proxmox `surfaces.rs:184-187`).

**The local-executor tier ladder** (`crates/ui/surface-proxy/src/proxy/local_executor.rs:126-341`, first match wins):

| Tier | Pairs | Behavior |
| --- | --- | --- |
| 1a | notification `{create,edit,test,delete}` on `notifications.{channel}` | controller DB code + audit, no plugin call |
| 1b | `("proxmox.hosts","add-config")` | unreachable (above) |
| 2a/2b/2c | notification settings saves; `("docker.item-host-actions","switch-tag")`; proxmox update-protection/scaling saves | plugin invoke **+ controller audit** |
| 3 | everything else | plugin invoke, no audit |

**One dispatch entry point bypasses registry resolution.** Enumerating every production caller of
`handle_surface_action` (contrarian pass, 2026-07-16): besides the post-resolution surface paths and a pass-through
decorator (`routes/service_ws/handler/update_tracking.rs:432-445`, delegates to the inner catalog), exactly one
out-of-band caller exists — the **public, unauthenticated** notification callback route
(`crates/ui/web-api/src/routes/notifications.rs`, `notification_callback` → `:1467`), which dispatches the
pseudo-action `"handle_callback"` on `surface_id = format!("notifications.{channel_type}")` directly through the
catalog's prefix routing. `handle_callback` is **not a registered interaction** anywhere (only telegram's
`handle_action` matches it, `telegram/src/surfaces.rs:50`); it reaches the plugin today purely because prefix routing
doesn't consult registrations. An exact-id dispatch map built from registrations would silently break every inbound
Telegram Bot API callback — addressed by D2a.

**No sudo metadata exists on actions** (the D9 sketch was stale on this): `SurfaceActionDescriptor` carries only
`timeout_seconds`; sudo lives on `PluginDescriptor.sudo` and is untouched by this spec.

## Decisions taken (grilling, 2026-07-16)

One spec for the whole unification; `RegisteredInteraction` pairing struct with minimal churn (plugin developers
should not touch "global" files); delete dead data/files; small authoring builder for the agent side; fold
`owned_surface_ids` derivation into `declare_plugin!`; **no interaction id renames** (dual-registration machinery not
needed — the version-skew constraint from D3/D9 is not triggered); permission typing (`Option<String>` →
`Option<Permission>`) stays deferred; include the executor-allowlist guard.

## Design

### D1 — Plugin-local single-source registration types (infrastructure-core)

New types in `crates/plugins/infrastructure/core` (non-wire; handlers cannot live on the serde `InteractionDescriptor`):

```rust
/// Per-interaction async handler. Mirrors the existing `SurfaceActionHandler`
/// alias (descriptor.rs:336) minus the surface_id/action_id params, which the
/// dispatch map already resolved.
pub type InteractionHandler = for<'a> fn(
    &'a SurfaceActionContext<'a>,
    serde_json::Value,
) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, SurfaceActionError>> + Send + 'a>>;

/// How an interaction is executed. The wire `InteractionTransport` is DERIVED
/// from this — never authored separately (a second author-settable transport
/// would recreate the two-declaration drift this spec eliminates).
#[non_exhaustive] // public cross-crate enum, per coding-standards.md#public-enum-extensibility
pub enum InteractionDelivery {
    /// Dispatched to the plugin's handler (local-executor Tier 2 or 3; any
    /// controller-side audit wrap stays keyed in local_executor, untouched).
    PluginHandled(InteractionHandler),
    /// Executed by controller-side code in local_executor Tier 1; the plugin
    /// declares the interaction but has no handler. Guarded by D5.
    ControllerExecutor,
}

pub struct RegisteredInteraction {
    // PRIVATE fields — construction only via `new()`, which derives
    // `descriptor.transport` from `delivery`. A struct literal would bypass
    // the derivation and reintroduce an author-settable transport.
    descriptor: InteractionDescriptor,
    delivery: InteractionDelivery,
}

pub struct PluginSurface {
    pub descriptor: surfaces::SurfaceDescriptor,
    pub interactions: Vec<RegisteredInteraction>,
}

pub struct PluginSurfaceRegistration {
    // mirrors wire SurfaceRegistration minus provider fields, holding
    // RegisteredInteraction instead of InteractionDescriptor
    pub surfaces: Vec<PluginSurface>,
}
```

- `RegisteredInteraction::new(descriptor, delivery)` **overwrites** `descriptor.transport` from the delivery — both
  variants map to `ControllerLocal`, which is what every catalog-registered interaction uses today (verified: zero
  `ProviderProxied`-transport registrations in plugin crates; those come exclusively from the service-registration
  path, which D4 serves with its own type). One knob, one truth. No `ProviderProxied` or `DirectBuiltInApi` delivery
  variants: neither has a producer in this spec's scope, and `#[non_exhaustive]` makes adding either later additive —
  the same treatment the spec gives `DirectBuiltInApi` explicitly.
- Stripping to the wire type: `PluginSurfaceRegistration::to_wire(...) -> surfaces::SurfaceRegistration` drops
  deliveries. The existing `PluginSurfaceOps::surface_registrations()` (consumed by controller boot,
  `crates/core/controller-runtime/src/boot/components.rs:274-289`) keeps its signature — derived by stripping — so
  boot and `SurfaceRegistry` are untouched.
- fn pointers, not closures: per-interaction handlers are named `fn`s coerced to `InteractionHandler`, same idiom as
  today's `SurfaceActionHandler` wrappers (e.g. `docker_handle_surface_action`, docker `plugin.rs:433`). A shared
  handler registered on two surfaces (proxmox `load-backup-target-options`) is the same `fn` referenced from both
  `PluginSurface` entries.
- Proxmox's leaf handlers take `&TenantDb`; each per-action shim obtains it via the same call
  `handle_action_inner` uses today (`ctx.tenant_db()`, proxmox `surfaces.rs:433-441`) and delegates to the existing
  leaf fn unchanged. `ControllerSurfaceAction` and `resolve_controller_surface_action` are deleted; the 17 shims are
  the mechanical replacement, each colocated with its registration.

### D2 — `declare_plugin!` folds to one surfaces arm; catalog derives routing

- The macro's `surface_actions: { actions, handle_action }` and `owned_surface_ids:` arms
  (`crates/plugins/infrastructure/core/src/macros.rs:60-70`) are **deleted**. The `surfaces:` arm changes to take a
  `fn() -> Vec<PluginSurfaceRegistration>`. Plugin developers touch only their own crate: registrations fn + handler
  shims; no global-file edits per new interaction (the whole point of the fold).
- `PluginDescriptor.surface_actions` (`descriptor.rs:588`) is deleted. `SurfaceRegistrationOps` (`descriptor.rs:234`)
  is retyped to the new fn.
- `PluginCatalog::build` derives, from one `registrations()` invocation per plugin:
  - the exact-id dispatch map `(surface_id, interaction_id) → InteractionHandler` (only `PluginHandled` entries);
  - admission uniqueness: the **same surface id** registered by two different plugins is a build error (replaces the
    prefix-overlap check; strictly tighter — exact-id routing is safe because every dispatched `surface_id` comes from
    a `ResolvedSurfaceAction`, i.e. post-registry-resolution, and no dynamic/parameterized surface ids exist);
  - the wire registrations served by `surface_registrations()`.
- `PluginSurfaceActionOps::handle_surface_action` (`catalog.rs:370-381`) looks up the exact pair; a miss is the same
  `SurfaceActionError` shape as today's "no plugin handles surface" / proxmox "unknown action" errors.
  `route_surface_action` and `surface_action_routes` die with the prefix machinery.
- **Named deliverable — delivery-tagged accessor**: `to_wire()` drops deliveries, so the D5 guard cannot see them
  through `surface_registrations()`. The catalog exposes a read-only accessor over what it already computed at build
  time — e.g. `interaction_deliveries() -> impl Iterator<Item = (&str, &str, InteractionDeliveryKind)>` (surface id,
  interaction id, fieldless kind mirror of `InteractionDelivery`, since the handler pointer itself is irrelevant to
  the guard). Without this the D5 reverse direction is unimplementable. `InteractionDeliveryKind` is
  `#[non_exhaustive]` like its source, and the mirror is produced by a `kind()` method on `InteractionDelivery`
  **inside the defining crate with an exhaustive match (no wildcard arm)** — a future delivery variant then fails
  compile until its kind counterpart exists, instead of silently blinding the D5 guard.
- **`to_wire(...)` provider fields**: `framework_generation`, tenant binding, and `capabilities` are derivable
  boilerplate (verified identical across docker/webhook/proxmox registrations), but `provider_id`
  (`"plugin.releases_docker"`, …) is genuinely per-plugin — it threads through `to_wire(...)` as a parameter sourced
  from the plugin's existing constant, not a new hand-authored field on the registration type. Exact signature at
  plan time.
- Feature split preserved additively: proxmox keeps the `cfg!(feature = "agent-infra")` expression-position split
  (`plugin.rs:63-76` idiom — registrations return `vec![]` under `agent-infra`; agent surfaces register via the
  service runtime instead). No `#[cfg(not(...))]` anywhere.

#### D2a — `handle_callback` moves off the surface dispatch path

The notification callback route (see Problem) is not a surface interaction: it is an inbound webhook keyed by
`channel_type`, invoked unauthenticated by external services (Telegram Bot API), with its own verification inside the
plugin. Registering it as an interaction would expose it on the authenticated user invoke path — a surface-area
expansion, not a fix. Instead:

- Add a callback method to the `NotificationTransport` role trait
  (`crates/plugins/infrastructure/core/src/roles.rs:380`) — default implementation returns
  "callback not supported for this channel"; telegram overrides it with the existing `handle_callback` body
  (`telegram/src/surfaces.rs:103`), removed from its `handle_action` match.
- `notification_callback` (`routes/notifications.rs`) resolves the plugin by `channel_type` — the same
  `plugin_ops.transport(&channel_type_id)` resolution the route file already uses at `:792` (`NotificationOps` is a
  supertrait of `PluginOps`, `plugin_ops.rs:389-397`) — and calls the trait method; the `"handle_callback"`
  pseudo-action string and the `format!("notifications.{channel_type}")` surface-id fabrication die. Exact method
  signature at plan time, with two pinned constraints: (a) the method is **not** `deliver`-shaped — telegram's body
  writes the notification log via `ctx.tenant_db()`, so it must carry a `SurfaceActionContext` (or equivalent DB
  access) and be `plugin-ops`-feature-gated, the trait's first DB-coupled method; (b) `transport()` returns `Option`
  (`plugin_ops.rs:319`) — the `None` arm maps to an explicit error, not a panic or silent 200.
- Net behavior unchanged: telegram callbacks work; other channel types get the same "unsupported" error shape they
  effectively get today (`InvalidInput` from their `handle_action` fallthrough — confirm the route's error mapping in
  the plan).
- This removes the **last** out-of-band `handle_surface_action` caller, restoring the invariant D2's exact-id map
  rests on: dispatch == registrations, with no dispatch entry point outside the model.

### D3 — Deletions (dead data + superseded machinery)

Once every plugin is on D1/D2:

- `SurfaceActionDescriptor`, its builders, and `ApiSubmitDescriptor`
  (`crates/plugins/infrastructure/core/src/surface_form_authoring.rs:249-364`) — including all 13 `with_api_submit`
  sites and every `*_surface_actions()` list in proxmox/docker/webhook/telegram/email. `SurfaceActionUi` and the form
  authoring types stay only if the D4 agent builder still consumes them (implementation decides; anything with zero
  remaining consumers dies — deny-level `dead_code` enforces this).
- `SurfaceActionLibrary`, `SurfaceActionHandler`, the catalog `.actions`/prefix plumbing, and the
  `__declare_surface_action_library_static!` macro arm (`macros.rs:803-813`).
- The parity guard test (proxmox `surfaces.rs:2843-2885`) — deleted only in the same change that lands D5's
  bidirectional guard; the drift class must never be unguarded in between.
- Tier 1b: `proxmox_add_config.rs` gate + `execute_allowlisted_proxmox_add_config_action` + its audit emitter, and the
  Tier 1b arm in `local_executor.rs:209-241` (unreachable, per Problem).
- Doc references to the deleted machinery (D7 inventory).

**Not deleted here**: the five orphaned `crates/ui/surface-proxy/src/proxy/*.rs` files (`prepared`, `validation`,
`dispatch`, `bookkeeping`, `idempotency`) — owned by the pending "Remove Orphaned surface-proxy Module Files" spec.
Sequencing note: that spec should land **first** (it deletes files a reviewer might otherwise cite as live dispatch
code). If it has not landed when this spec is implemented, verify at plan time that its deletions do not collide with
D5's const-table location (`controller_local/` module layout) — the diff should be disjoint, but confirm rather than
assume.

### D4 — Agent-side inversion (agent-ssh runtime + proxmox agent module)

Today the runtime authors 8 built-in actions and collects the proxmox agent's 2 as `SurfaceActionDescriptor`s, then
derives `InteractionDescriptor`s (`build_interactions`: destructive/`confirm_entity_field` → `ConfirmableAction`,
wizard → `Workflow` + steps, timeout clamp 1–300 → `u16`, `permission_or_none`). Inversion keeps that derivation logic
in **one** authoring builder instead of scattering hand-written descriptors:

- New agent authoring type (infrastructure-core, gated `#[cfg(feature = "agent-infra")]` alongside the existing
  agent-infra module) — working name `AgentInteraction`: carries the descriptor-building inputs (id, label, icon,
  kind-derivation inputs, timeout, permission, form/wizard UI), **placement metadata the wire
  `InteractionDescriptor` cannot carry** (`row_visible_when`, `batch`, primary-vs-row placement), and the agent-side
  handler. The builder produces the `InteractionDescriptor` via the same derivation rules `build_interactions()`
  implements today (the logic moves, not duplicates).
- Agent handler signature differs from the controller's (`&InfraPluginContext<'_>`, `&SurfaceActionRequest` →
  `Option<SurfaceActionResponse>`-shaped today) — a separate handler alias, not a generic over D1's type. The
  `GuestExec::handle_service_extension_action` trait method (`roles.rs:858-867`) **stays** as the runtime→plugin entry
  point (minimal churn); the proxmox impl's body becomes a lookup into its own `AgentInteraction` table, replacing the
  hand-written `match request.interaction_id` in `agent/surface_actions.rs:22-35`. Descriptor and dispatch now share
  one construction site on the agent side too.
- `PluginDescriptor` gains the agent-surfaces hook replacing the `.actions` collection path (the agent-infra analogue
  of D2; shape mirrors the existing `agent_migrations` field precedent, `descriptor.rs:602-605`). The runtime's
  `build_actions()` infra-collection (`surface_runtime.rs:195-201`) and `collect_infra_primary_actions()` literal
  filter (`:69-80`) switch to reading it; the hardcoded placement literals (`SSH_HOSTS_ROW_ACTION_IDS`,
  the `"bootstrap-proxmox-guest"` filter) are replaced by the placement metadata.
- The runtime's own 8 built-ins are authored with the same builder (they are the majority consumer; this is what
  keeps the builder honest).
- Admission-rule interplay (D1 of the guest-flow spec): everything this path registers is `ProviderKind::Service` +
  `InteractionTransport::ProviderProxied`, so the builder does **not** expose `provider_invocable` — a flagged
  permissioned interaction from a service is rejected at admission (`ProviderInvocableForbiddenForServiceProviders`).
- **Wire-output equivalence gate**: before the old path is deleted, capture the current `ssh-agent.hosts` wire
  `SurfaceRegistration` JSON as a fixture; the new path must produce identical output (interaction set, kinds,
  workflow steps, timeouts, permissions, `visible_when`, batch flags). This is the agent-side replacement for the
  parity test — it catches silent metadata loss the wire *type* cannot.

### D5 — Bidirectional executor guard

The original drift class relocates, for `ControllerExecutor` interactions, to the string-matched allowlist arms in
`local_executor.rs`. Close both directions:

- Refactor the five `allowlisted_*` gate fns (`crates/ui/surface-proxy/src/proxy/controller_local/*.rs`) to read from
  **one const table** of `(surface_id, interaction_id, tier)` entries in surface-proxy. The gates keep their exact
  current predicates and per-tier behavior (audit, DB routing) — this is a data-extraction refactor, not a behavior
  change; each pair keeps its tier byte-for-byte.
- Guard test (lives in `web-api`, the lowest crate that sees both the plugin catalog and surface-proxy — both are
  already regular `[dependencies]` there, plus `[dev-dependencies]` re-declarations with testing features). Reads the
  D2 `interaction_deliveries()` accessor on one side and the const table on the other:
  - every const-table pair exists among the catalog's registered interactions, with delivery matching the tier
    (Tier 1 ⇒ `ControllerExecutor`; Tier 2 ⇒ `PluginHandled`);
  - every registered `ControllerExecutor` interaction appears in a Tier-1 const-table row (the reverse direction —
    a registered-but-unexecutable interaction fails loud);
  - green-on-empty protection: assert the iterated sets are non-empty and contain named known members
    (`("notifications.webhook", "create")` — webhook is unconditionally in web-api's catalog;
    `("docker.item-host-actions", "switch-tag")`) before asserting membership. Telegram/email registrations are
    feature-gated and web-api's defaults exclude them, so their known-member assertions are `#[cfg]`-gated on the
    corresponding features, and the Verification section names the exact `--features` command that exercises them —
    otherwise the reverse-direction check for those plugins silently never runs outside `--all-features` builds.
- **Scope honesty**: the guard proves interaction *existence and delivery kind*, not *audit-tier value*. Tiers 2 and 3
  both map to `PluginHandled`, so an interaction moved between an audited tier and the unaudited fallthrough is
  invisible to it. That audit-correctness class is pre-existing (the tiers are hand-authored today) and stays with the
  const table, whose rows the gates themselves read — the table cannot drift from gate behavior, only from intent.

### D6 — Tests (success + failure per AGENTS.md)

- **infrastructure-core**: catalog admission rejects duplicate surface ids across plugins (RED by registering the same
  id twice); dispatch map lookup hit/miss; `RegisteredInteraction::new` transport derivation per delivery variant
  (iterate all variants, not one — drift-class guard, `strum` if already a dep, else exhaustive match).
- **per plugin**: dispatch derivation smoke test — every `PluginHandled` registration resolves and its handler is the
  one registered (replaces the per-plugin `surface_actions_not_empty` tests); existing behavioral handler tests are
  unchanged (handlers themselves don't move).
- **proxmox both build worlds**: `agent-infra` has `default = []`, but any **workspace-wide** run already compiles it
  ON via feature unification — `uptrakit-agent-ssh-runtime` depends on the registry with
  `features = ["daemon", "agent-infra"]` unconditionally (`agent-ssh-runtime/Cargo.toml:47`). The world the canonical
  gates never exercise is agent-infra **OFF** in isolation; cover both explicitly with the scoped clippy + test runs
  in Verification (bare `-p` = OFF — where the controller registrations are non-empty; `--features agent-infra` = ON).
- **agent-ssh runtime**: wire-output equivalence fixture test (D4); existing `handle_surface_request_internal` tests
  keep passing (entry-point behavior unchanged).
- **web-api**: D5 guard; the existing provider-origin e2e tests (`routes/service_ws/handler/tests.rs`) keep passing
  unchanged — they exercise the registry + gate + dispatch path end to end and double as the regression net for D2.
- **D2a callback route**: telegram callback dispatches through the new trait method (success), and a channel type
  without a callback override returns the unsupported-error shape (failure). The route's existing inline `TestApp`
  tests (`notifications.rs` test module, e.g. `notification_callback_success_writes_audit_and_updates_log`) must keep
  passing unchanged through the D2a rewiring; the unsupported-channel test is the new deliverable.
- RED/GREEN discipline: the D5 guard must be shown RED both directions (delete a const-table row; register a
  `ControllerExecutor` interaction without a row) before the parity test is deleted.

### D7 — Documentation deliverables

| File | Change |
| --- | --- |
| `docs/adr/0028-single-source-plugin-interaction-registration.md` | **New ADR**: the two-system history, the drift incident, the single-source decision, transport-derived-from-delivery rule, exact-id routing |
| `docs/development/plugin-system.md` | `declare_plugin!` reference: replace the `surface_actions:`/`owned_surface_ids:` block (`:570`) with the unified `surfaces:` arm |
| `docs/development/plugin-guidelines.md` | Authoring guide: replace `surface_actions()` catalogue section (`:1487`) and the `SurfaceActionDescriptor` icon example (`:1806-1827`) with `RegisteredInteraction` authoring; agent-side `AgentInteraction` section |
| `docs/development/notifications.md` | The two `declare_plugin!` examples (`:145-155`, `:351-355`) and the wiring prose (`:424`, `:865-875` hand-enumerates `surface_actions.actions`/`handle_action`) — grep `surface_actions` over the whole file, fix every hit; document the D2a `NotificationTransport` callback method (grep `handle_callback` too) |
| `docs/development/surfaces.md` | New section: unified registration model, delivery kinds, the executor const table + guard |
| `docs/api/batch-actions.md` | `:117` references `SurfaceActionDescriptor`/`batch_action` — repoint at the agent authoring builder's batch placement |
| `docs/development/proxmox-plugin.md` | `agent/surface_actions.rs` row (`:56`) + action-table prose |
| `docs/architecture/ssh-agent.md` | Surface-assembly description (placement metadata replaces hardcoded literals) |
| `crates/plugins/AGENTS.md` | Authoring rules: one registration source, transport never authored directly |
| `AGENTS.md` | Plugin-system stub sentence if it names the deleted arms (grep `surface_actions` — currently zero hits, so likely no change; verify at implementation) |

No `regen-api.sh` needed: no HTTP contract, OpenAPI schema, or interaction id changes. No `asyncapi.yaml` change:
surface registration payloads remain unmodeled (pre-existing gap recorded in the guest-flow spec). Rustdoc on every
new public type; every fn whose signature changes gets its doc-comment re-checked in the same edit
(stale-comment class).

## Alternatives considered

- **Keep hand-written matches + guard tests forever**: status quo shape; guards are maintained-by-update and
  per-plugin — the class stays alive everywhere a guard is missing. Rejected.
- **Handlers on the wire `InteractionDescriptor`**: impossible — serde type crossing the service wire; fn pointers
  don't serialize. The plugin-local pairing struct is the only shape that keeps the wire contract intact.
- **Model audit/tier in `InteractionDelivery`**: rejected — audit emitters live in surface-proxy and cannot move into
  plugin crates (dependency direction); modeling them plugin-side would create a second author-settable copy of the
  tier table. Tiers stay in local_executor, guarded by D5 instead.
- **Big-bang vs plugin-by-plugin releases**: single release. Everything is in-tree, no wire change, no id change; a
  half-migrated tree is itself a drift state. Implementation is still phased per crate for reviewability.

## Out of scope

- Interaction id renames (`list`, `discover`, …) and the transitional dual-registration machinery — only needed if a
  rename ever happens (D9 constraint stands, recorded in the ADR).
- Permission typing `Option<String>` → `Option<Permission>` on descriptors/gates (own spec).
- Removing the five orphaned surface-proxy files (owned by the pending orphan-removal spec; sequencing note in D3).
- A `DirectBuiltInApi` delivery variant (zero plugin users today; additive later).
- Modeling surface registration payloads in `asyncapi.yaml` (pre-existing gap).

## Verification

- Canonical workspace gates (quality-gates.md): `cargo fmt --all`; clippy + test with
  `--no-default-features --features db-sqlite`; `cargo clippy --all-targets --all-features` +
  `cargo test --all-features` (needs `frontend/build/`); `cargo deny check` (no new deps expected — verify none
  slipped in).
- Scoped feature-gated runs (exact `-p` names verified against each `Cargo.toml`):

  ```sh
  # agent-infra ON and OFF worlds (workspace runs unify agent-infra ON — see D6)
  cargo clippy --all-targets -p uptrakit-plugin-infrastructure-proxmox --features agent-infra
  cargo test -p uptrakit-plugin-infrastructure-proxmox --features agent-infra
  cargo clippy --all-targets -p uptrakit-plugin-infrastructure-proxmox
  cargo test -p uptrakit-plugin-infrastructure-proxmox   # OFF world: controller registrations are non-empty here
  # D5 guard with the feature-gated notification plugins compiled in (no frontend/build needed)
  cargo test -p uptrakit-web-api --no-default-features \
    --features db-sqlite,notifications-telegram,notifications-email
  ```

  (Confirm the exact web-api feature names against its `Cargo.toml` at plan time — `notifications-telegram`/
  `notifications-email` exist today at `crates/ui/web-api/Cargo.toml:18-19`.)

  plus the touched crates on default features: `uptrakit-plugin-infrastructure-core`,
  `uptrakit-plugin-infrastructure-registry`, `uptrakit-plugin-releases-docker`,
  `uptrakit-notification-plugin-webhook`, `uptrakit-notification-plugin-telegram`,
  `uptrakit-notification-plugin-email`, `uptrakit-agent-ssh-runtime`, `uptrakit-surface-proxy`, `uptrakit-web-api`.
- Deletion-completeness greps (must be zero hits in `crates/` and `docs/` outside CHANGELOGs/historical specs):

  ```sh
  grep -rn "SurfaceActionDescriptor\|SurfaceActionLibrary\|owned_surface_ids\|ControllerSurfaceAction\|resolve_controller_surface_action\|with_api_submit" \
    crates/ docs/development docs/architecture docs/api
  ```

- Presence greps (survivors must be non-zero): `RegisteredInteraction`, `InteractionDelivery`,
  `InteractionDeliveryKind`, `interaction_deliveries`, the D5 const table.
- `python3 ci/check_plugin_semantic_boundary.py` and `bash ci/verify_agents_md_budget.sh` (AGENTS.md files touched).
- `markdownlint --config .markdownlint.json` on every touched doc.
- Manual: existing deployment smoke — invoke one interaction per execution path (`proxmox.hosts`/`discover`
  PluginHandled; `notifications.webhook`/`create` ControllerExecutor; agent `bootstrap-proxmox-guest` via the
  service-registered ProviderProxied transport) and confirm identical behavior + audit rows.
