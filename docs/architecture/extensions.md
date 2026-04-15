# Shared Surface Runtime Architecture

This document describes the current architecture for dynamic UI integrations in Uptrakit.
The old standalone extension-framework crate is gone; the active model is the shared surface
runtime used by built-in pages, plugin-backed providers, and service-backed providers.

## Contract ownership

The canonical contract is owned by:

- `crates/shared/surfaces/` — identifiers, slots, surface nodes, interactions, data sources,
  registration protocol, and validation policy.
- `crates/shared/wire/src/surfaces.rs` — wire barrel re-export (`pub use uptrakit_surfaces::*`).
- `crates/shared/web-api-types/src/surfaces.rs` — REST envelope types used by web API, frontend,
  and openapi client.

Framework compatibility is explicit via `FrameworkGeneration` and
`FrameworkGenerationRange` in the contract.

## Slot registry ownership

Surface slots are centrally declared in `crates/shared/surfaces/src/slot.rs`.

Current slot IDs:

- `settings.tabs`
- `settings.below.global`
- `software.tabs`
- `host_detail.tabs`
- `software_item.host_context_menu`
- `extension.page`

Each slot definition includes:

- single-entry vs multi-entry behavior
- provider priority bounds

Controller admission rejects unknown slot IDs and duplicate occupancy for single-entry slots.

## Controller runtime

The controller runtime is composed of:

- `crates/ui/web-api/src/surface_registry.rs` — source-of-truth provider/surface catalog and
  admission policy enforcement
- `crates/ui/web-api/src/surface_proxy.rs` — interaction dispatch, request correlation,
  idempotency, cancellation, and timeout handling
- `crates/ui/web-api/src/routes/surfaces.rs` — REST endpoints for list/providers/read/invoke

`SurfaceRegistry` tracks provider registrations from three sources:

- service runtime (`register_service`)
- compiled-in built-ins (`bootstrap_builtin`)
- compiled-in plugins (`bootstrap_plugin`)

## Provider registration protocol

Services register through `ServiceMessage::SurfaceRegistration`.
Plugins and built-ins register in-process during controller startup.

Each registration includes:

- provider identity (`provider_id`, `provider_kind`)
- `framework_generation`
- provider capability set
- effective tenant binding (`scope` plus optional `tenant_id`)
- one or more `RegisteredSurface` entries (descriptor + interactions + data sources)
- optional encryption metadata for sensitive interaction parameters

Service registrations are tied to authenticated service identity and tenant context.
Provider rotation for a service ID is supported; in-flight requests for the replaced provider are
failed to avoid stale routing.

## Strict capability gating

Admission is fail-closed. A registration is rejected when any policy check fails.

Core gates:

- unsupported framework generation
- missing required capabilities
- invalid slot/contract
- invalid transport usage
- tenant-binding mismatch
- allowlist violations (`controller_query`, SSE topic, direct built-in operation)
- payload and contract-size/depth limits

Rejections are returned as structured reasons with machine-readable codes
(`UnsupportedGeneration`, `MissingCapability`, `InvalidSlot`, `InvalidTransport`,
`SchemaOrLimitFailure`).

## Interaction routing

Surface interactions are resolved per tenant and per surface:

- universal surfaces can auto-resolve a provider
- targeted surfaces require `target_provider_id`

Routing then follows declared transport:

- `ProviderProxied` — proxied to connected service provider
- `ControllerLocal` — executed in controller (currently plugin providers)
- `DirectBuiltInApi` — built-in operation IDs only, controller allowlisted

`SurfaceProxy` enforces idempotency and timeout behavior and maps failures to typed surface error
codes.

## Frontend unified renderer path

The frontend path is unified for built-in and provider-backed surfaces:

- `frontend/src/lib/surfaces/registry.svelte.ts` — runtime surface index by slot, provider cache,
  read-model cache, rollout state
- `frontend/src/lib/components/surfaces/` — shared renderer components
  (`SurfaceReadPanel`, `SurfaceRenderer`, `SurfaceTable`, `SurfaceForm`, `SurfaceWorkflow`, ...)
- `frontend/src/routes/extensions/[id]/+page.svelte` — dynamic route keyed by `surface_id`

Because extension-page navigation uses `/extensions/{surface_id}`, refreshing keeps users on the
same surface page.

## Rollout signal

`/api/v1/surfaces/runtime-status` exposes controller-owned rollout state (`active`), letting the
frontend keep behavior fail-closed during activation windows.

## Legacy compatibility seam

Some plugin/service code paths still use legacy extension action payload shapes internally for
controller-local action handler compatibility. That seam is implemented inside
`uptrakit-plugin-infrastructure-core` and service modules, but the active UI contract and transport
for runtime registration and rendering are shared surfaces.
