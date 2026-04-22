# Surfaces Dynamic Update Design

**Date:** 2026-04-22
**Status:** Approved

## Problem

Frontend loads available Surfaces once on page load via `GET /api/v1/surfaces`. If a service
connects or disconnects after the page is open, the UI never learns about it. Surfaces provided
by the newly-connected service do not appear; surfaces from a disconnected service remain visible
with stale provider availability.

## Scope

- Any service type (not only agent-ssh) can register surface providers.
- Both surface appearance (new provider joined) and disappearance (provider left) must be handled.
- No payload required on the event — coarse "registry changed" signal, frontend re-fetches.

## Out of Scope

- Fine-grained per-surface or per-provider diff events.
- Push of surface read-model content (separate concern).

## Architecture

### Backend — new `AdminEvent` variant

Add to `crates/shared/web-api-types/src/events.rs`:

```rust
SurfacesChanged,
```

No fields. The `EventBroadcaster` is already per-tenant, so the signal is automatically scoped.
Adding a variant to `AdminEvent` is an additive change; existing match sites need a wildcard arm
added if exhaustive.

### Backend — emission sites

Two call sites in `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`, both of which
already hold `AppState`:

1. **Surface registration** — after `SurfaceRegistration` message is processed and surfaces are
   added to `SurfaceRegistry`, broadcast `AdminEvent::SurfacesChanged` for the connection's
   `tenant_id`.

2. **Disconnect teardown** — after surfaces for the disconnecting connection are removed from
   `SurfaceRegistry`, broadcast `AdminEvent::SurfacesChanged` for the same `tenant_id`.

No changes to `SurfaceRegistry` itself; no new dependencies introduced.

### Frontend — event type

Add `"SurfacesChanged"` to the `AdminEventType` union/const in `frontend/src/lib/sse.ts`.

Wire the handler in `frontend/src/lib/stores/events.svelte.ts` alongside existing event
handlers. Apply the same debounce already used for other event types (collapse burst of events
within a short window into one refresh).

### Frontend — refresh logic

On receiving a debounced `SurfacesChanged` event:

1. Call `loadSurfaceRegistry()` — re-fetches `GET /api/v1/surfaces`, replaces the local surface
   list. Surfaces that disappeared are removed; new surfaces appear.
2. For each surface currently tracked in the store, call `getSurfaceProviders(surfaceId)` to
   refresh provider availability (disconnected → available or vice-versa).

No component changes required — surface registry is a reactive Svelte store; components
re-render automatically.

## Data Flow

```text
Service connects / disconnects
        │
        ▼
WS handler processes SurfaceRegistration or teardown
        │
        ▼
SurfaceRegistry mutated (add / remove providers)
        │
        ▼
event_broadcaster.broadcast(tenant_id, AdminEvent::SurfacesChanged)
        │  (per-tenant SSE channel, capacity 512)
        ▼
Frontend SSE stream receives "SurfacesChanged"
        │
        ▼
Debounce (collapse burst)
        │
        ▼
loadSurfaceRegistry() + getSurfaceProviders(id) for each tracked surface
        │
        ▼
Reactive store updates → components re-render
```

## Error Handling

- SSE reconnection is already handled with exponential backoff; no extra handling needed.
- `loadSurfaceRegistry()` failure: surface store retains previous state; existing error handling
  in the store applies.
- If `SurfacesChanged` is received while a fetch is in-flight, debounce ensures only one
  refresh runs after the burst settles.

## Testing

- **Backend unit:** emit `SurfacesChanged` on `SurfaceRegistration` and on disconnect teardown;
  assert `EventBroadcaster` received the event with correct `tenant_id`.
- **Frontend unit:** mock SSE delivering `SurfacesChanged`; assert `loadSurfaceRegistry` and
  `getSurfaceProviders` are called; assert debounce collapses rapid events into one call.
- **E2E / manual:** open UI, connect a new agent-ssh service, verify new surfaces appear without
  page refresh; disconnect service, verify surfaces disappear.
