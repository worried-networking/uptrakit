# Shared Surface Runtime — Development Guide

This guide documents how to build and integrate provider-backed UI functionality using the shared
surface runtime.

The old `uptrakit-extension-framework` crate has been removed. Runtime UI integration now uses
`uptrakit_surfaces` (via `uptrakit_internal_wire::surfaces`) plus the controller `SurfaceRegistry`
and shared frontend renderer.

## Quick map

- Contract types: `crates/shared/surfaces/`
- Wire barrel: `crates/shared/wire/src/surfaces.rs`
- Controller registry and admission: `crates/ui/web-api/src/surface_registry.rs`
- Controller dispatch/correlation: `crates/ui/web-api/src/surface_proxy.rs`
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

Slots are centrally owned in `crates/shared/surfaces/src/slot.rs`. Do not invent slot IDs in
providers. Use declared constants and semantics:

- `SLOT_SETTINGS_TABS`
- `SLOT_SETTINGS_BELOW_GLOBAL`
- `SLOT_SOFTWARE_TABS`
- `SLOT_HOST_DETAIL_TABS`
- `SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU`
- `SLOT_EXTENSION_PAGE`

Slot validation is controller-enforced during admission.

## Registration contract

A registration contains:

- `provider`: provider identity
- `framework_generation`: currently v1.0
- `capabilities`: provider contract capability set
- `effective_tenant_binding`: global or tenant scope, with tenant ID when scoped
- `surfaces`: array of `RegisteredSurface` (descriptor + interactions + data sources)
- `encryption_metadata` (optional): required for sensitive params on proxied service providers

Services send registration after connection setup when `UiExtensions` is part of the agreed
capability set.

## Strict controller gating (fail-closed)

Controller admission rejects incompatible registrations. Main gates:

- framework generation range mismatch
- missing required capabilities
- invalid slot or invalid contract shape
- transport misuse for provider kind
- tenant-binding mismatch against authenticated service context
- allowlist failures (`controller_query`, SSE topic, direct built-in operation IDs)
- payload and depth limits

Do not rely on graceful fallback for incompatible contracts. Fix the provider contract until
admission succeeds.

## Service integration pattern

In service handlers:

1. Build `SurfaceRegistration` payload(s) from service state.
2. Send `ServiceMessage::SurfaceRegistration` once connected (and whenever rotating provider ID).
3. Handle `ControllerMessage::SurfaceActionRequest` in
   `ServiceHandler::on_surface_action_request`.
4. Respond with `ServiceMessage::SurfaceActionResponse`.

Service-initiated action calls are supported via `ServiceMessage::SurfaceActionRequest`, with
correlated `ControllerMessage::SurfaceActionResponse`.

## Plugin integration pattern

Plugin descriptors can provide both:

- legacy extension action handlers (`extensions` section) for controller-local action logic
- shared surface registrations (`surfaces` section) for runtime UI projection

`PluginSurfaceOps::surface_registrations()` is aggregated by `PluginCatalog`, and the controller
bootstraps these registrations into `SurfaceRegistry`.

For plugins migrating from legacy descriptors, use
`build_plugin_surface_registrations_from_extensions(...)` in
`uptrakit-plugin-infrastructure-core` as the compatibility bridge.

## Frontend integration pattern

The frontend loads and renders surfaces through shared runtime modules:

- `loadSurfaceRegistry()` fetches rollout status and surface list (`/api/v1/surfaces/*`)
- `getSurfacesBySlot(slot)` drives slot rendering and sidebar integration
- `SurfaceReadPanel` + `SurfaceRenderer` render shared nodes and interactions

Extension-page nav items are derived from the `extension.page` slot and route to
`/extensions/{surface_id}`, so page refresh keeps users on the same provider-backed page.

The old extension-only renderer path (`frontend/src/lib/components/extensions/`) is no longer the
active rendering path.

## REST and CLI surfaces

REST endpoints:

- `GET /api/v1/surfaces`
- `GET /api/v1/surfaces/runtime-status`
- `GET /api/v1/surfaces/{surface_id}/providers`
- `GET /api/v1/surfaces/{surface_id}/read`
- `POST /api/v1/surfaces/{surface_id}/interactions/{interaction_id}`

CLI uses `uptrakit surfaces` commands against the same surface API.

## Migration notes

- Remove any dependency on `uptrakit-extension-framework`.
- Move new UI contract work to `uptrakit_surfaces`.
- Keep legacy extension action payload usage only as an internal compatibility seam when needed.
- Prefer slot-driven shared renderer integration over route-specific custom UI code.
