# Shared Surface Runtime — Development Guide

This guide documents how to build and integrate provider-backed UI functionality using the shared
Surfaces runtime.

Runtime UI integration uses `uptrakit_surfaces` (via `uptrakit_internal_wire::surfaces`) plus the
controller `SurfaceRegistry` and shared frontend renderer.

## Quick map

- Contract types: `crates/shared/surfaces/`
- Wire barrel: `crates/shared/wire/src/surfaces.rs`
- Controller registry and admission: `crates/ui/web-api/src/surface_registry.rs`
- Controller dispatch/correlation: `crates/ui/surface-proxy/src/proxy.rs` (crate `uptrakit-surface-proxy`)
- REST endpoints: `crates/ui/web-api/src/routes/surfaces.rs`
- Frontend runtime store: `frontend/src/lib/surfaces/registry.svelte.ts`
- Frontend shared renderer: `frontend/src/lib/components/surfaces/`

## Provider models

Three provider kinds are supported:

- `Service` — runtime registration over WebSocket (`ServiceMessage::SurfaceRegistration`)
- `Plugin` — controller startup bootstrap (`PluginSurfaceOps::surface_registrations()`)
- `BuiltIn` — controller startup bootstrap for built-in controllers/providers

Provider identity is `provider_id` + `provider_kind`.

## Slot registry ownership

Slot IDs are fixed by `crates/shared/surfaces/src/slot.rs`. Do not invent slot IDs in providers,
and do not treat slot names as a separate visual system. Use the declared constants and semantics:

- `SLOT_SETTINGS_TABS`
- `SLOT_SETTINGS_BELOW_GLOBAL`
- `SLOT_SOFTWARE_TABS`
- `SLOT_HOST_DETAIL_TABS`
- `SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU`
- `SLOT_SURFACE_PAGE`

Slot validation is controller-enforced during admission.

## Registration contract

A registration contains:

- `provider`: provider identity
- `framework_generation`: currently v1.0
- `capabilities`: provider contract capability set
- `effective_tenant_binding`: global or tenant scope, with tenant ID when scoped
- `surfaces`: array of `RegisteredSurface` (descriptor + interactions + data sources)
- `encryption_metadata` (optional): required for sensitive params on proxied service providers

Services send registration after connection setup when `UiSurfaces` is part of the agreed
UI-surface capability set, and the controller records compatibility from the provider-reported
framework generation and capabilities.

## Strict controller gating (fail-closed)

Controller admission rejects incompatible registrations. Main gates:

- framework generation range mismatch
- missing required capabilities
- invalid slot or invalid contract shape
- transport misuse for provider kind
- tenant-binding mismatch against authenticated service context
- allowlist failures (`controller_query`, SSE topic)
- payload and depth limits

The built-in UI and surface-backed UI must stay visually aligned; the runtime is a parity path,
not a separate design system.

## Canonical Runtime States

The shared Surfaces runtime uses the following canonical render-state IDs to describe UI
conditions:

- `loading`
- `permission_denied`
- `no_compatible_provider`
- `contract_mismatch`
- `hydration_action_failure`
- `no_surface_content`

These are render states, not error codes. The runtime surfaces different states depending on
loading, authorization, provider compatibility, contract validation, action hydration, or
empty-content conditions.

Do not rely on graceful fallback for incompatible contracts. Fix the provider contract until
admission succeeds.

> Surface-backed UI must render through the same visual primitives and token adapter as built-in UI.
> If a new primitive is needed, promote it into the shared frontend component set first.

## Service integration pattern

In service handlers:

1. Build `SurfaceRegistration` payload(s) from service state.
2. Send `ServiceMessage::SurfaceRegistration` once connected (and whenever rotating provider ID).
3. Handle `ControllerMessage::SurfaceActionRequest` in
   `ServiceHandler::on_surface_action_request`.
4. Respond with `ServiceMessage::SurfaceActionResponse`.

Service-initiated action calls are supported via `ServiceMessage::SurfaceActionRequest`, with
correlated `ControllerMessage::SurfaceActionResponse` — but only when the target interaction is
unpermissioned or opts in via `InteractionDescriptor.provider_invocable`. `provider_invocable` is
a wire field with a fail-closed default: omitted on the wire, it deserializes to `false`, so a
permissioned interaction stays closed to provider-origin calls until it explicitly opts in.
Registration admission rejects the flag on permissioned interactions owned by `Service`-kind
providers — only `Plugin`/`BuiltIn` providers may combine it with `required_permission`. See
[Surface Security](../security/surfaces.md#provider-origin-invocation) for the caller-origin gate.

`ServiceSurfaceProxy` (`crates/shared/service-sdk/src/surface_proxy.rs`) implements the
service-side oneshot-correlation pattern for these session-scoped messages: each outbound request
is tracked by a generated correlation ID mapped to a `tokio::sync::oneshot::Sender`, and the
matching response resolves it.

Interaction, data-source, and surface identifiers follow a single naming convention
([ADR-0031](../adr/0031-surface-identifier-naming.md)):

- **Interaction IDs and data-source IDs:** kebab-case only — `^[a-z][a-z0-9]*(-[a-z0-9]+)*$`. No underscores, no
  dots, no provider/surface prefixes (the surface ID already namespaces).
- **Surface IDs:** dot-separated kebab-case segments — each segment matches the interaction regex. First segment
  names the provider family (`proxmox`, `notifications.email`, `mqtt`, `ssh-agent`).
- **CRUD over a collection:** one plural noun registered under multiple HTTP methods — GET (list / item read via
  `/{item_id}`), POST (create), PUT `/{item_id}` (replace), DELETE `/{item_id}`. Never `list-`, `get-`,
  `create-`, `edit-`, `delete-`, `remove-`, `preload-`, `load-`, `save-` prefixes. **One GET registration
  serves both list and item read** — `(surface, noun, GET)` is a single registered interaction whose handler
  branches on `params["id"]` presence; registering list and item-get separately would collide on the
  `(id, method)` uniqueness key.
- **Two shapes outside the buckets (allowed, by rule):** a read-only singleton may pair with a _separate_ POST
  domain operation instead of a PUT (docker `current-tag` + `switch-tag`); an item-_targeted_ domain operation
  stays a collection-level POST with the item id in `params["id"]` (POST accepts no item segment —
  notifications `test`). Providers read `params["id"]` uniformly regardless of whether the framework populated
  it from the path segment, query, or body (companion spec, reserved-key contract).
- **Singleton resources** (settings blobs with no collection): singular/uncounted noun under GET + PUT
  (`smtp`, `global-defaults`, `overrides`).
- **Domain operations** (not CRUD): imperative verb phrase under POST (`test-connection`, `discover`, `match`,
  `bootstrap`, `switch-tag`, `sync`). Bare verbs fine; no nouns the surface already implies.
- **Data sources** pair with their GET interaction: `DataSourceKind::ProviderQuery.operation_id` equals the
  paired GET interaction ID, and the data-source ID uses the same noun (`mappings` ↔ GET `mappings`).
- **Workflow step-submit IDs** follow the domain-operation rule (`sync-connect`, `bootstrap-execute` — already
  compliant).

Third-party and externally-registered service providers get this convention as normative guidance only: the
wire-level identifier charset validator (`validate_surface_identifier`) stays permissive and does not itself
reject a non-conforming ID.

First-party registrations are guard-tested: the `crates/plugins/infrastructure/registry` catalog guard
(`tests/surface_id_naming_guard.rs`) plus its `agent-ssh-runtime`/`mqtt-runtime` sibling tests assert every
compiled-in surface/interaction/data-source ID against this convention.

Presence of business-critical plugin surfaces under feature unification is guarded separately by
`contribution_monotonicity_guard.rs` in the same test directory
([ADR-0032](../adr/0032-plugin-contribution-monotonicity.md)).

## Plugin integration pattern

Plugin descriptors provide shared surface registrations and the controller-local interaction logic
needed to service those surfaces.

`PluginSurfaceOps::surface_registrations()` is aggregated by `PluginCatalog`, and the controller
bootstraps these registrations into `SurfaceRegistry`.

## Unified plugin registration model

Plugin-side surfaces and interactions have a single source of truth (ADR-0028;
`crates/plugins/infrastructure/core/src/registration.rs`): the `declare_plugin!` macro's
`surfaces: { provider_id, registrations }` arm, where `registrations` is a `fn() -> Vec<PluginSurfaceRegistration>`.
Each interaction is a `RegisteredInteraction::new(descriptor, delivery)`, pairing a wire
`surfaces::InteractionDescriptor` with an `InteractionDelivery`. `RegisteredInteraction::new` derives
`descriptor.transport` from the delivery and overwrites whatever the caller set — transport is never
authored independently, closing the drift class that let a legacy dispatch table and a registered
interaction list disagree for over a year.

### Delivery kinds

- `InteractionDelivery::PluginHandled(handler)` — dispatched to a plugin `InteractionHandler` fn pointer.
- `InteractionDelivery::ControllerExecutor` — executed entirely by controller-side code; the plugin
  declares the interaction but has no handler for it.

### Exact-id routing and duplicate-surface-id admission

`PluginCatalog::build` derives an exact-id `BTreeMap<(String, String), InteractionHandler>` dispatch map
from every plugin's `registrations()` call at build time (only `PluginHandled` entries), replacing the
longest-prefix `starts_with` routing an earlier revision of this runtime used. `PluginSurfaceActionOps::handle_surface_action`
does an exact-pair lookup; a miss returns the same error shape the prefix router produced for an
unroutable action. On surface-id admission, the **same plugin** re-registering the **same** `surface_id`
across calls is fine; a **different** plugin claiming an already-seen `surface_id` is a hard
catalog-construction error. Exact-id routing is safe here because every dispatched `surface_id` is
post-registry-resolution (from a `ResolvedSurfaceAction`) — no dynamic or parameterized surface ids
exist.

### Controller-local executor table + bidirectional guard

`ControllerExecutor` interactions must have a matching row in `CONTROLLER_LOCAL_EXECUTOR_TABLE`
(`crates/ui/surface-proxy/src/proxy/controller_local.rs`), a single `(surface_id, interaction_id, ExecutorTier)`
const table. `ExecutorTier::ControllerExecutes` is Tier 1 (controller-side code + audit, no plugin call);
`ExecutorTier::PluginWithAudit` is Tier 2 (plugin invoke + controller-side audit wrap); Tier 3 (plugin
invoke, no audit) is the fallthrough and has no table rows. A bidirectional integration test,
`crates/ui/web-api/tests/interaction_executor_guard.rs`, proves the table and the catalog's derived
interaction deliveries agree in both directions: every table row has a matching registration with the
expected delivery kind, and every `ControllerExecutor`-delivery registration has a Tier-1 table row — a
registered-but-tableless `ControllerExecutor` interaction is registered but unexecutable, and fails the
guard instead of failing silently at request time. The guard proves existence and delivery kind, not
audit-tier correctness: Tiers 2 and 3 both map to `PluginHandled`, so a row moved between an audited and
an unaudited tier stays a hand-authored correctness class, unguarded by this test.

## Frontend integration pattern

The frontend loads and renders surfaces through shared runtime modules:

- `loadSurfaceRegistry()` fetches surface list (`/api/v1/surfaces/*`)
- `getSurfacesBySlot(slot)` drives slot rendering and sidebar integration
- `SurfaceReadPanel` + `SurfaceRenderer` render shared nodes and interactions

Shared Surfaces nav items are derived from the `surface.page` slot and route to
`/surfaces/{surface_id}`. That is the canonical page route for provider-backed surfaces.

`frontend/src/lib/components/surfaces/` is the canonical rendering path for provider-backed pages,
and it must use the same visual primitives and token adapter as the built-in UI.

## REST surfaces

REST endpoints:

- `GET /api/v1/surfaces`
- `GET /api/v1/surfaces/{surface_id}`
- `GET /api/v1/surfaces/{surface_id}/providers`
- `GET|POST|PUT|DELETE /api/v1/surfaces/{surface_id}/interactions/{interaction_id}`
- `GET|PUT|DELETE /api/v1/surfaces/{surface_id}/interactions/{interaction_id}/{item_id}` (`POST` on this path is a
  static 405 stub — create always targets the collection, never a specific item)

These endpoints are utoipa-registered; frontend access goes through the generated SDK (the hand-written
`frontend/src/lib/api/surfaces.ts` is retired). Full route family, query contract, and error semantics: [Shared
Surface API](../api/surfaces.md).

### Provider visibility filter

The tenant-facing `SurfaceRegistry` methods (`list_surfaces_for_tenant`,
`list_targeted_providers_for_surface`, `resolve_surface_read`,
`resolve_surface_action_for_method`) take a required `visibility: &dyn SurfaceProviderVisibility`
parameter — a caller cannot resolve without deciding plugin visibility. Production callers pass the
controller's `PluginEffectiveEnablement` (stored on `SurfaceProxyDeps`); `SurfaceProxy` stores the
filter at construction (`with_provider_visibility`, deny-all default) for its internal resolution,
which is the only gate on the provider-origin leg. Tests that don't exercise enablement use the
`testing`-gated `AllProvidersVisible`. Service- and BuiltIn-kind providers are never consulted.

### Declaring an interaction's method

Every `InteractionDescriptor` carries `http_method` (`get` | `post` | `put` | `delete`; wire default `post`).
Authoring rules:

- `DataLoad` interactions are always `GET`, regardless of the declared value: a declared `put`/`delete` is rejected
  at admission, while a declared `post` (or an omitted field) silently normalizes to `get` — the two are
  indistinguishable at admission time. Prefer declaring `get` explicitly for new `DataLoad` interactions so the
  intent is legible in the descriptor even though it is not enforced.
- `Workflow` interactions must declare `post`.
- `FormSubmit`, `MutationAction`, and `ConfirmableAction` declare whichever of `post`/`put`/`delete` matches the
  operation's semantics (`post` is the default when omitted). There is no `patch` — mutations that replace a
  resource use `put` (full replace), never a partial-update verb.
- `DataLoad` interactions must not declare non-empty `sensitive_fields` (GET params travel in the query string —
  see [Shared Surface Security](../security/surfaces.md) for the failure mode this admission rule prevents).

Optionally declare `params: Vec<ParamFieldDescriptor>` (`key`, `schema`, `required`) for fields that need strict
typed parsing. This matters most for `GET` interactions: a param not listed in `params` is forwarded to the
provider as an untyped JSON string parsed from the query string, while a declared param is parsed per its
`SchemaContract` and rejected with `422 schema_validation_failed` on a bad value. Declared keys must not collide
with the framework-reserved keys `id`, `page`, `per_page`, `target_provider_id`, `timeout_seconds`. Mutating methods
(`POST`/`PUT`/`DELETE`) apply the same per-field validation to the JSON body when `params` is declared.

An `interaction_id` may register under more than one `http_method` within the same surface — registration
uniqueness is `(surface_id, interaction_id, http_method)`, extending ADR-0028's exact-ID dispatch (see
[ADR-0030](../adr/0030-surfaces-rest-method-model.md)). A content node that references an interaction by bare ID
resolves only if that ID has exactly one registered method; a bare reference to a multi-method ID is rejected at
admission rather than silently defaulting to `post`.

### Item addressing

Interactions targeting a specific row in a collection can be invoked via the trailing `/{item_id}` path segment
instead of (or as well as) an `id` field in the query string or body — the framework injects the path segment's
value into `params["id"]`, overwriting anything already present under that key. Providers read `params["id"]`
uniformly regardless of which route carried it in.

## Migration notes

- Move new UI contract work to `uptrakit_surfaces`.
- Prefer slot-driven shared renderer integration over route-specific custom UI code.
