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
- allowlist failures (`controller_query`, SSE topic, direct built-in operation IDs)
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

Interaction IDs follow a naming convention by kind: data-retrieval interactions use noun phrases
(`discovered-guests`, `unmatched-guests`); mutations keep verbs (`bootstrap-proxmox-guest`,
`match`).

## Plugin integration pattern

Plugin descriptors provide shared surface registrations and the controller-local interaction logic
needed to service those surfaces.

`PluginSurfaceOps::surface_registrations()` is aggregated by `PluginCatalog`, and the controller
bootstraps these registrations into `SurfaceRegistry`.

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
- `GET /api/v1/surfaces/{surface_id}/providers`
- `GET /api/v1/surfaces/{surface_id}/read`
- `POST /api/v1/surfaces/{surface_id}/interactions/{interaction_id}`

## Migration notes

- Move new UI contract work to `uptrakit_surfaces`.
- Prefer slot-driven shared renderer integration over route-specific custom UI code.
