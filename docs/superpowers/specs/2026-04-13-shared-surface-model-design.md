# Shared Surface Model for Built-in and Extension UI

## Summary

Uptrakit should replace the current extension framework with a new shared surface model that both built-in routes and extension-provided functionality use. Built-in routes will continue to own routing, URL state, page orchestration, and product-specific logic, while visible content is rendered through the same shared surface primitives and design language as extension content.

This design deliberately ignores backward compatibility with the existing extension framework. Compatibility is only required within the new framework generation and is enforced by strict controller-side capability gating. Unsupported surfaces must never be exposed to the user.

## Background

The current implementation splits built-in and extension UI along multiple boundaries:

- The schema contract lives in [`crates/shared/extension-framework/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/extension-framework/src/lib.rs) as `ExtensionManifest`, `ExtensionUi`, `ActionDef`, and related types.
- The controller exposes extensions through a registry and generic action endpoints in [`crates/ui/web-api/src/extension_registry.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/extension_registry.rs), [`crates/ui/web-api/src/extension_proxy.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/extension_proxy.rs), and [`crates/ui/web-api/src/routes/extensions.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/extensions.rs).
- The frontend keeps a separate extension registry and renderer path in [`frontend/src/lib/extensions.svelte.ts`](/Users/andreyyantsen/Development/uptrakit/frontend/src/lib/extensions.svelte.ts) and [`frontend/src/routes/extensions/[id]/+page.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/routes/extensions/[id]/+page.svelte).
- Some built-in pages already import extension-specific renderer components directly, especially on `settings`, `software`, and software detail pages.

The result is partially unified visuals but separate architecture. Extension tabs on built-in pages can feel integrated, but extension full pages and many extension interactions still run through a different mental and technical model.

The `TASK-0010` materials, especially [`RFC-0017.md`](/Users/andreyyantsen/Development/uptrakit/docs/internal/changes/TASK-0010/RFC-0017.md) and unpublished design work in `.pipeline-run`, correctly identify two major constraints:

- Built-in and extension UI already share some rendering concerns and should converge on one component model.
- The current wire and REST contracts are vulnerable to incompatible schema evolution, especially around internally tagged enums.

## Goals

- Make extension-provided functionality visually and behaviorally close to built-in functionality in the web UI.
- Ensure route-owned page state, especially tab state, survives refresh for extension-backed tabs embedded in built-in pages.
- Use one shared rendering layer and design language for built-in and extension content.
- Keep built-in routes responsible for navigation, URL state, and page orchestration.
- Make the framework expandable through explicit capability negotiation, not optimistic deserialization.
- Prevent unsupported or partially understood surfaces from reaching the user.
- Provide a clear migration path from the current framework to the new one.

## Non-Goals

- Preserving runtime backward compatibility with the current extension framework.
- Giving extension-provided full pages canonical first-class built-in route identities in this iteration.
- Turning all built-in pages into fully remote schema-driven pages.
- Supporting graceful degradation for unknown surface features.

## High-Level Design

The new model has three layers:

1. A shared contract layer for describing renderable surfaces, interactions, placement, and capabilities.
2. A controller-side registry and admission layer that accepts only supported framework generations and capabilities.
3. A frontend shared renderer layer that built-in routes and extension-attached surfaces both use.

The key architectural boundary is:

- Built-in routes own URL structure, tab persistence, layout composition, data fetching orchestration, and product-specific workflows.
- Both built-in and extension content render through the same surface renderer and shared primitives.
- Extension-owned full pages use a generic surface route, with any legacy extension URL kept only as a compatibility redirect.

## Core Model

### SurfaceDescriptor

`SurfaceDescriptor` replaces `ExtensionManifest`.

It describes:

- `surface_id`: stable identifier
- `label`
- `priority`
- `slot`: named placement target
- `scope`: `global` or `tenant`
- `targeting`: universal or targeted
- `required_permission`
- `provider_kind`: built-in, plugin, service
- `required_capabilities`
- `root_node`: render tree entry point

`SurfaceDescriptor` is the unit registered with the controller and consumed by the frontend.

Surface identity and scope rules:

- Built-in surfaces use a reserved `builtin.` namespace.
- Provider-authored surfaces must use a provider-owned namespace derived from plugin type or service app name.
- `surface_id` must be unique within its effective scope.
- Built-in surfaces cannot be shadowed by provider registrations.

`surface_id` grammar:

- ASCII lowercase letters, digits, `.`, `_`, and `-` only
- must start with an ASCII letter
- maximum length 128 bytes
- case-sensitive normalization is forbidden; producers must emit canonical lowercase IDs

Priority semantics:

- lower numeric values sort first
- built-in default priority is defined per route-owned slot entry
- provider-authored surfaces may only choose priorities within a bounded provider range declared by the slot definition
- ties are broken deterministically by `surface_id`

Targeting semantics:

- `universal` means the surface is resolved without provider selection
- `targeted` means the surface contract is shared, but read and action operations require explicit provider-instance selection
- in this design, targeting is about provider-instance routing, not arbitrary domain-object targeting

### SurfaceSlot

`SurfaceSlot` replaces the current placement model as the primary attachment mechanism.

Examples:

- `settings.tabs`
- `settings.below.global`
- `software.tabs`
- `software_item.host_context_menu`
- `extension.page`

Slots are owned by built-in routes or containers. A route decides how a slot is rendered, how slot entries become tabs or sections, and how slot state maps to the URL.

Slot IDs are not ad hoc strings. They are centrally defined product API identifiers in the shared contract crate and re-exported as constants for controller and frontend use. Registration must fail if a provider references a slot ID outside that registry.

This is the mechanism that lets extension-backed tabs behave like built-in tabs after refresh: `/settings` remains the route, and the active tab is just a stable slot entry ID kept in `?tab=...`.

### SurfaceNode

`SurfaceNode` replaces `ExtensionUi` as the render tree.

Recommended initial node families:

- `Section`
- `TextBlock`
- `KeyValue`
- `Table`
- `Form`
- `ActionBar`
- `Tabs`
- `Callout`
- `EmptyState`
- `ModalTrigger`
- `WorkflowTrigger`

These nodes are intentionally presentation-oriented and declarative. They can reference interactions and data sources, but they do not own routing or page-level orchestration.

### Data Contract

The surface model must carry an explicit read-path contract alongside render nodes.

Introduce `DataSourceDescriptor` as a first-class contract referenced by `SurfaceNode`s that need data. At minimum, it must support:

- `Static`: embedded read-only data
- `ControllerQuery`: controller-owned read path
- `ProviderQuery`: provider-proxied read path

Each data source declares:

- `data_source_id`
- `result_schema`
- `pagination`
- `sorting`
- `filtering`
- `refresh_policy`
- `empty_state`

`data_source_id` grammar and scope follow the same lexical rules as `surface_id`. A `data_source_id` must be unique within its containing `surface_id` contract.

`Table`, `KeyValue`, and data-backed `Form` nodes do not embed opaque “data source” references. They reference a typed `data_source_id`.

`DataLoad` is a read-only interaction kind used to execute a declared `DataSourceDescriptor`. It is not a generic catch-all action and must not be used for mutations.

Schema format rules:

- `input_schema` and `result_schema` use a shared constrained JSON Schema profile defined by the new shared crate
- the allowed subset must be identical across controller, frontend, and CLI consumers
- controller registration must reject schemas outside the supported profile

Read-path authorization rules:

- `ControllerQuery` is allowed only for built-in and controller-local plugin surfaces
- service-backed providers may not declare arbitrary controller-owned read paths
- if provider-authored controller queries are ever introduced later, they must use an explicit controller allowlist keyed by `data_source_id`

`controller-local` means descriptors materialized by in-process controller startup code rather than any remote registration transport.

Refresh policy rules:

- `manual`
- `interval { seconds }`
- `sse { topic }`

Initial constraints:

- minimum interval refresh is 10 seconds
- `ProviderQuery` data sources may not declare `interval` below the minimum
- `sse` refresh is controller-owned; providers do not declare arbitrary topics
- allowed `sse` topics come from a controller-owned topic registry
- registrations using unsupported refresh policies must be rejected

Form node rules:

- a `Form` node renders fields from the referenced `FormSubmit` interaction `input_schema`
- the form node may add presentation metadata such as sectioning, labels, or field ordering, but may not widen the accepted input contract

Trigger node rules:

- `ModalTrigger` opens a renderer-owned modal containing nested `SurfaceNode`s
- `WorkflowTrigger` starts a declared `Workflow` interaction and may render step-local nested nodes
- modal open/close state is renderer-owned; workflow execution state is interaction-owned
- `Workflow` is an ordered multi-step interaction contract whose steps have explicit local input/output contracts and controller-visible progression state

### InteractionDescriptor

`InteractionDescriptor` replaces the current flat `ActionDef` model.

Recommended initial interaction kinds:

- `MutationAction`
- `FormSubmit`
- `Workflow`
- `Navigate`
- `DataLoad`
- `ConfirmableAction`

Each interaction declares:

- `interaction_id`
- `kind`
- `required_permission`
- `input_schema`
- `sensitive_fields`
- `timeout`
- `confirmation`
- `transport`: controller-local, provider-proxied, or direct built-in API

`interaction_id` grammar and scope follow the same lexical rules as `surface_id`. An `interaction_id` must be unique within its containing `surface_id` contract.

Read-only surfaces must not imply executable actions. Any mutation path must be explicitly represented by an interaction node or reference.

Transport rules:

- `controller-local`: handled entirely inside the controller process.
- `provider-proxied`: routed over the controller-managed provider transport and supported in both directions, so controller-to-provider and provider-to-controller flows can use the same action envelope shape.
- `direct built-in API`: allowed only for built-in surfaces defined in controller-owned source code. Provider-authored registrations may not declare this transport.

For `direct built-in API`, the interaction must bind to an explicit controller-side allowlisted method-and-path target. Authorization remains controller-enforced and tied to the interaction identity, not delegated to the frontend.

That allowlist should use stable controller-owned operation identities where available, not unconstrained literal path strings.

Sensitive-field rules:

- `sensitive_fields` is enforced, not advisory.
- Provider-proxied interactions that accept sensitive fields require a provider encryption key to be advertised at registration time.
- Sensitive params for provider-proxied interactions use the current ECIES P-256 client-side sealing model, or a transport-equivalent wrapper with the same security properties.
- The controller treats provider-bound ciphertext as opaque and must not decrypt it.
- Only the addressed provider instance may decrypt provider-proxied sensitive params.
- The controller must reject any request where a field declared in `sensitive_fields` is supplied outside the encrypted sensitive-params envelope.
- `input_schema` defines the cleartext request contract only.
- Sensitive-field logical validation is split: the controller validates cleartext params against `input_schema` and validates that all declared sensitive fields are present only in the encrypted envelope; the addressed provider validates decrypted sensitive-field values against its local interaction schema after decryption.

## Ownership Model

### Built-in Routes

Built-in routes continue to own:

- URL structure
- query parameter parsing
- active tab persistence
- local page state
- SSE subscriptions and other live page concerns
- composition of multiple slots into one page shell

### Shared Renderer

The renderer owns:

- translating `SurfaceNode` into Svelte components
- consistent layout primitives
- shared action button behavior
- shared loading, empty, error, and permission states
- slot entry rendering

### Providers

Providers only supply surface and interaction descriptors plus any required action handlers. They do not control route semantics.

## Routing and Refresh Behavior

For this iteration:

- Extension-owned full pages use `/surfaces/[surface_id]` as the canonical route.
- `/extensions/[surface_id]` may remain temporarily only as a compatibility redirect to `/surfaces/[surface_id]`.
- Built-in routes expose their extension-attached functionality through their own route-local slot rendering.

Examples:

- `/settings?tab=notifications.telegram`
- `/software?tab=proxmox.hosts`

The route decides which slot entries are valid and which default tab is selected.
Because the tab identity is part of the route-owned URL state, refresh preserves the
user’s location regardless of whether that tab content is built-in or
extension-provided.

## Compatibility and Capability Gating

Compatibility must be explicit and strict.

### Framework Negotiation

Every provider registration includes:

- `framework_generation`
- `capabilities`
- `surfaces`
- `interactions`
- optional provider encryption metadata when sensitive interactions are supported

The controller has a compiled-in supported generation range and supported capability set.

Capability sets are explicit and named. Initial capability families should include:

- supported surface node kinds
- supported interaction kinds
- supported data source kinds
- supported targeting modes
- sensitive-field support
- provider-to-controller invocation support

Registration succeeds only when:

- the provider generation is supported
- all required capabilities are supported
- all slot references are valid
- all declared node and interaction kinds are permitted by the negotiated capability set
- all declared permissions are valid controller-known permission identifiers
- the registration stays within admission resource limits

### Failure Behavior

If compatibility checks fail:

- the controller rejects the registration
- no surface is exposed through REST or UI
- the provider receives an explicit rejection reason
- the frontend never sees unsupported nodes

This avoids the current failure mode where new enum variants can break deserialization or produce unclear partial behavior.

### Scope of Rejection

For the initial implementation, rejection should occur at the registration batch level. Per-surface partial acceptance can be added later if needed, but defaulting to all-or-nothing keeps safety and reasoning simpler.

Admission resource limits must be part of the contract. Initial defaults:

- at most 64 surfaces per registration batch
- at most 256 interactions per registration batch
- maximum surface tree depth of 16
- maximum serialized registration payload size of 512 KiB
- maximum declared page size of 200 rows
- maximum declared interaction timeout of 300 seconds
- minimum declared interaction timeout of 1 second

Registrations that exceed these limits must be rejected before entering the runtime registry.

Runtime budget rules must also be enforced. Initial defaults:

- maximum 32 in-flight action requests per provider connection
- maximum 128 in-flight action requests per tenant
- maximum action response payload size of 1 MiB
- maximum data query result page size of 200 rows
- controller-enforced cancellation when deadlines expire
- per-provider rate limiting on repeated failing requests

## Backend Architecture

### Shared Crate

Create a new shared crate to replace the current extension framework. It should define:

- surface contracts
- data source contracts
- interaction contracts
- registration and invocation payloads
- capability types
- generation identifiers

The old extension framework crate should not be evolved in place.

### Controller Registry

Replace the current extension registry with a surface registry indexed by:

- effective tenant scope
- `surface_id`
- `slot`
- provider
- targeting
- permissions

The registry should store both built-in and provider-registered surfaces in one normalized shape.

Tenant partitioning rules:

- Service-backed registrations are bound to the tenant associated with the authenticated controller connection and may not self-assert a different tenant.
- Plugin-backed and built-in surfaces may be global or tenant-aware, but registry lookup must always resolve through the active tenant context.
- Action dispatch and provider discovery must validate tenant compatibility before resolving a target provider.

Collision rules:

- for universal surfaces, duplicate `surface_id` in the same effective scope is a registration error
- for targeted surfaces, `surface_id` identifies the shared surface contract while provider membership is tracked separately per provider connection
- built-in surfaces always win namespace ownership
- multiple surfaces may attach to the same slot only when the slot is declared multi-entry by the route owner

Slot registry metadata must define whether each slot is single-entry or multi-entry.

Targeted surface consistency rules:

- the first accepted targeted registration for a given `(tenant scope, surface_id)` establishes the canonical contract
- subsequent providers joining that targeted surface must match the canonical `root_node`, interaction definitions, data-source definitions, and required capabilities
- any mismatch is a registration error for the later provider

Built-in surface onboarding:

- built-in surfaces are inserted into the same runtime registry during controller startup using the shared normalized contract types
- built-in surfaces do not use the remote `SurfaceRegistration` transport path

### Invocation

Replace the current extension WS protocol split with:

- `SurfaceRegistration`
- `SurfaceActionRequest`
- `SurfaceActionCancel`
- `SurfaceActionResponse`

This removes the current split between manifest registration and action library registration.

Minimum payload requirements:

- `SurfaceRegistration`: provider identity, negotiated framework generation, capability set, effective tenant binding, surfaces, interactions, data sources, optional encryption metadata
- `SurfaceActionRequest`: request ID, tenant context, surface ID, interaction ID, target provider ID when required, regular params, optional encrypted sensitive params, controller-derived caller origin, idempotency metadata
- `SurfaceActionCancel`: request ID, target provider ID, cancellation reason
- `SurfaceActionResponse`: request ID, success flag, structured result payload or structured error payload

Targeted surface rules:

- targeted surfaces require explicit provider discovery
- the controller must expose a provider-discovery endpoint analogous to the current extension provider listing flow
- targeted `SurfaceActionRequest`s must carry a validated provider ID
- the controller must reject provider IDs that are not registered for that surface in the active tenant scope

The invocation contract must support both controller-initiated and provider-initiated action requests so existing cross-provider workflows remain possible.

Authorization rules:

- `caller_origin` is controller-derived metadata and must not be trusted from frontend or provider-supplied input
- valid `caller_origin` values are controller-owned principals only: authenticated user session, built-in system principal, or provider principal
- provider-initiated requests may only target controller-allowed interactions exposed to that provider kind and tenant context
- provider-initiated requests may not impersonate end users
- controller authorization must run before routing any provider-initiated request to another provider or built-in handler

Request lifecycle rules:

- disconnecting a provider removes all provider-registered surfaces from the runtime registry
- in-flight requests targeting a disconnected provider fail immediately with a structured transport error
- repeated delivery of the same mutation request ID must be treated idempotently by the controller routing layer where feasible, or rejected as a duplicate when not feasible
- late responses for timed-out or cancelled requests must be ignored by the registry/proxy layer
- controller deadline expiry for provider-proxied work must emit `SurfaceActionCancel` when the transport supports cancellation; ignoring late responses is only the fallback behavior for non-cancellable provider work
- idempotency metadata retention windows and duplicate behavior must be defined by the transport implementation and tested; duplicates for still in-flight mutations should resolve to a deterministic duplicate outcome rather than double execution
- transports should retain idempotency metadata for at least 15 minutes by default unless a stricter deployment-wide policy is configured

Runtime schema rules:

- controller must validate action request params against `input_schema` before dispatch
- controller must validate declared data/action responses against `result_schema` before forwarding them to frontend or CLI consumers
- schema validation failures must surface as structured contract errors, not renderer crashes

Error taxonomy:

- `permission_denied`
- `invalid_request`
- `schema_validation_failed`
- `unsupported_capability`
- `provider_unavailable`
- `timeout`
- `duplicate_request`
- `internal_error`

Built-in and provider discovery contracts:

- the controller must expose a provider-discovery response shape for targeted surfaces that includes provider ID, display label, tenant-compatible availability, and any provider encryption metadata
- built-in surfaces are enumerated through the same registry-backed read path as provider surfaces once bootstrapped into the runtime registry

## Frontend Architecture

Recommended structure:

- `frontend/src/lib/surfaces/contract.ts`
- `frontend/src/lib/surfaces/registry.svelte.ts`
- `frontend/src/lib/components/surfaces/`
- `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`

Shared primitive set:

- table renderer
- form renderer
- key-value renderer
- action bar and button renderer
- workflow and modal renderer
- slot/tab renderer

Built-in routes should stop importing `components/extensions/*` directly. They should render built-in and extension surfaces through one renderer path.

## CLI Implications

The current CLI dynamically pattern-matches on `ExtensionUi` and `ActionUi` in [`crates/ui/cli/src/commands/extensions.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/cli/src/commands/extensions.rs).

Under the new model, the CLI should consume controller-vetted surface and interaction descriptors rather than mirroring the old enum-specific logic. The CLI should support only interaction kinds that make sense in a terminal context. Unsupported presentation-only surfaces should never block compatible registrations, but CLI-specific unsupported rendering should result in explicit command diagnostics at CLI rendering time, not at transport deserialization time.

## Migration Plan

### Phase 0: Define the Cutover Contract

- Introduce the new surface framework behind a dedicated rollout flag.
- Do not activate the shared-surface runtime on mixed deployments.
- The cutover release must upgrade controller, frontend, and first-party service providers in one coordinated release train.
- Until that cutover is complete, shared-surface endpoints stay fail-closed and production-inert.
- The controller owns compatibility state and must refuse activation unless all
  required first-party providers report a compatible framework generation and
  capability set for that rollout mode.

When rollout is inactive, the runtime must remain inert:

- `GET /api/v1/surfaces` returns an empty list.
- surface read and interaction endpoints return `surface_runtime_inactive`.
- surface provider-listing endpoints behave as absence rather than partial metadata exposure.

`required first-party providers` means the controller-defined set of enabled first-party external services for that deployment mode. Optional providers disabled by configuration are excluded from the activation dependency set.

Phases 1 through 6 may land behind the rollout flag, but remain production-inert until the Phase 0 activation condition is satisfied.

### Phase 1: Define the New Contract

- Create the new shared crate and contract types.
- Add generation and capability definitions.
- Define the new WS payloads and REST DTOs.

### Phase 2: Replace Controller Registry and Admission

- Implement the new surface registry.
- Add registration validation and rejection reasons.
- Replace current extension registration paths.

### Phase 3: Build Shared Frontend Renderer

- Build the new shared surface components.
- Replace extension-only renderer components with the new surface renderer.
- Keep temporary wrappers only if they reduce churn during migration.

### Phase 4: Migrate Built-in Route Slots

Start with the routes already closest to convergence:

- `settings`
- `software`
- `software/[id]`

These routes already import extension renderer components or consume extension slot concepts. They should become the first built-in adopters of the new surface renderer.

### Phase 5: Migrate Plugin-Backed Surfaces

- Convert controller-local plugin extensions to new descriptors.
- Remove dependence on old manifests and actions.

### Phase 6: Migrate Service-Backed Surfaces

- Convert `agent-ssh`, `mqtt`, and any similar providers to the new registration protocol.
- Add provider capability declarations and controller-side gating.

### Phase 7: Remove the Old Framework

Delete:

- the old extension framework crate
- old WS extension registration types
- old REST extension DTOs
- frontend extension store helpers
- extension-specific renderer directory
- CLI logic tied to old `ExtensionUi` and `ActionUi`

## Testing Strategy

### Contract Tests

- serialization tests for all new contract types
- generation and capability validation tests
- registration rejection reason coverage
- admission resource limit coverage

### Controller Tests

- valid registration accepted
- unsupported generation rejected
- missing capability rejected
- invalid slot rejected
- cross-tenant resolution rejected
- duplicate `surface_id` rejected
- action invocation routing by provider kind
- provider-initiated action request routing preserved
- sensitive-field ciphertext treated as opaque by the controller

### Frontend Tests

- built-in and extension content render through the same renderer
- slot rendering order and priority
- built-in route tab state persists through refresh for extension-backed tabs
- unsupported surfaces never appear because registry input is already filtered

### End-to-End Tests

- extension-backed `settings` tab survives refresh on the same route
- targeted surfaces still require correct provider selection
- service/controller framework mismatch prevents surface exposure
- mixed-deployment rollout flag prevents partial cutover

## Risks and Mitigations

### Risk: Over-generalizing built-in pages into schema content

Mitigation:

- keep routes and page orchestration built-in
- limit schema ownership to renderable surfaces and interactions

### Risk: Slot taxonomy becomes unstable

Mitigation:

- define slot IDs centrally
- treat slot IDs as product API within the repo

### Risk: Migration leaves two rendering systems alive too long

Mitigation:

- once a route is migrated, remove mixed rendering paths for that route

### Risk: CLI expectations diverge from web UI

Mitigation:

- make the controller the compatibility gate
- treat CLI rendering support as a separate consumer contract over vetted interactions

## Decisions

- Use a hybrid ownership model: built-in routes own route state; both built-ins and extensions use the same surface renderer.
- Use strict capability gating, not graceful degradation, for framework compatibility.
- Keep generic extension page routes for now; do not introduce canonical built-in route identities for extension-owned pages in this iteration.
- Ignore backward compatibility with the current extension framework and migrate directly to a new framework generation.

## Implementation Notes

This design supersedes the current extension framework shape rather than extending it. Existing `TASK-0010` RFC/design artifacts were used as input for current-state analysis and compatibility constraints, but this spec is the source document for the new design direction.
