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
- Startup-time surface registrations (`bootstrap_builtin` / `bootstrap_plugin`) — these run before
  any tenant session exists, so no `tenant_id` is available and no SSE broadcast is needed.

## Architecture

### Backend — new `AdminEvent` variant

Add to `crates/shared/web-api-types/src/events.rs`:

```rust
SurfacesChanged,
```

No fields. `EventBroadcaster` is already per-tenant, so the signal is automatically scoped.

**Required maintenance in the same file:**

- Add arm `Self::SurfacesChanged => "surfaces_changed"` to `event_name()`.
- Add `AdminEvent::SurfacesChanged` to the `all_variants()` array and increment the hardcoded
  count assertion by 1.

`AdminEvent` is `#[non_exhaustive]`. External-crate match sites must add a wildcard arm
(`_ => …`). Within-crate match sites may add the specific arm or a wildcard.

**SSE wire format:** `SurfacesChanged` is a unit variant with no payload. The server must still
emit a `data:` line to satisfy the SSE parser (which drops events with no data). Emit
`data: {}`. This matches the pattern used for other payload-less admin events.

### Backend — emission sites

Emit at every `surface_registry.unregister_service(…)` call site in
`crates/ui/web-api/src/routes/service_ws/handler/mod.rs` (do not rely on line numbers —
grep for `unregister_service` to find all paths including any future additions).

Emit also after `SurfaceRegistration` message processing (surface add path).

All sites already hold `AppState`.

**`tenant_id` guard (applies to ALL three sites):** System services can have no `tenant_id`.
If `tenant_id` is `None`, skip the broadcast — there is no tenant SSE channel to target.

No changes to `SurfaceRegistry` itself; no new dependencies introduced.

### Frontend — event type

Add `"surfaces_changed"` to the `AdminEventType` union/const in `frontend/src/lib/sse.ts`.

Wire the handler in `frontend/src/lib/stores/events.svelte.ts` alongside existing event
handlers. `SurfacesChanged` carries no entity ID; the debounce key will be
`"surfaces_changed:"` (empty entity suffix). This correctly collapses all bursts into a single
refresh, which is the intended behavior. Use the same debounce window already applied to other
event types in that file — do not introduce a new window value.

### Frontend — refresh logic

On receiving a debounced `surfaces_changed` event, call `loadSurfaceRegistry()`. This single
call re-fetches `GET /api/v1/surfaces` and internally refreshes provider availability for all
targeted surfaces — no separate `getSurfaceProviders()` loop needed.

No component changes required — surface registry is a reactive Svelte store; components
re-render automatically.

On `loadSurfaceRegistry()` failure the store retains its previous state. Existing console-level
error logging in the store applies; no additional UI feedback is required.

## Data Flow

```text
Service connects / disconnects
        │
        ▼
WS handler processes SurfaceRegistration or calls unregister_service
        │
        ▼
SurfaceRegistry mutated (add / remove providers)
        │
        ▼
tenant_id present?
  No  → skip broadcast
  Yes → event_broadcaster.broadcast(tenant_id, AdminEvent::SurfacesChanged)
             (per-tenant SSE channel, capacity 512)
        │
        ▼
Frontend SSE stream receives "surfaces_changed"
        │
        ▼
Debounce collapses burst (key "surfaces_changed:")
        │
        ▼
loadSurfaceRegistry() — re-fetches surfaces + provider availability
        │
        ▼
Reactive store updates → components re-render
```

## Error Handling

- SSE reconnection is already handled with exponential backoff; no extra handling needed.
- `loadSurfaceRegistry()` failure: store retains previous state; existing error handling applies.
- Broadcast channel full (capacity 512 exceeded): event is dropped, frontend does not refresh.
  Accepted gap — channel saturation is pathological and self-corrects on next agent reconnect
  which will emit another `SurfacesChanged`.

## Testing

- **Backend unit:** assert `EventBroadcaster` receives `SurfacesChanged` after `SurfaceRegistration`
  message processed and after each `unregister_service` path; assert no broadcast when
  `tenant_id` is `None`.
- **Frontend unit:** mock SSE delivering `surfaces_changed`; assert `loadSurfaceRegistry` is
  called once; assert burst of three events debounces to one call.
- **E2E / manual:** open UI, connect a new agent-ssh service, verify new surfaces appear without
  page refresh; disconnect service, verify surfaces disappear.
