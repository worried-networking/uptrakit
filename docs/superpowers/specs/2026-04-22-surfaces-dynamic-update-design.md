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

Emit also after `SurfaceRegistration` message processing — **on the success path only**, after
`register_surface_provider` returns `Ok`. Do not emit on the two early-return error paths
(validation failure, registration rejection).

**Fourth emission site — `Replaced` session path:** `finalize_authenticated_session`
(in the same file) has a `Replaced` branch that cancels the processor task but skips
`cleanup_authenticated_session` entirely, so `unregister_service` is never called for the
outgoing session. If the replacement session connects without the `UiSurfaces` capability
(e.g., agent rollback or configuration change), the old surface registration lingers in the
registry permanently with no event ever fired. Fix: in the `Replaced` branch, after awaiting
`processor_handle`, check `state.surface_proxy_deps.registry.provider_id_for_service(&session.service_id)`;
if `Some`, call `fail_in_flight_for_provider`, `unregister_service`, and emit `SurfacesChanged`
(with the same `tenant_id` guard as all other sites — `session.service_tenant_id`).
The current branch destructure captures only `processor_cancel` and `processor_handle` via `..`;
`service_id` and `service_tenant_id` must also be explicitly destructured here.

All sites have access to `AppState`. Broadcast via `state.notification.event_broadcaster.send(tenant_id, AdminEvent::SurfacesChanged)`.

**`tenant_id` guard (applies to ALL four sites):** System services can have no `tenant_id`.
If `tenant_id` is `None`, skip the broadcast — there is no tenant SSE channel to target.

**`cleanup_embedded_service_session` requires a signature change:** This function currently has
no `tenant_id` parameter. Add `tenant_id: Option<uuid::Uuid>` to its signature. The call site
already has `session.service_tenant_id: Option<uuid::Uuid>` available to pass through.
`cleanup_authenticated_session` already receives `tenant_id` via `AuthenticatedSessionState`.

No changes to `SurfaceRegistry` itself; no new dependencies introduced.

### Frontend — event type

Add `"surfaces_changed"` to the `AdminEventType` union/const in `frontend/src/lib/sse.ts`.
Note: the existing union is missing `'global_github_provider_misconfigured'` and `'data_reset'`
(pre-existing gap, not introduced here). Do not assume the union is otherwise complete.

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

On success, `loadSurfaceRegistry()` clears `providersBySurface`, `readsBySurface`,
`readRequestedBySurface`, and `readLoadPromises` before repopulating. A burst of provider
changes can therefore cause repeated cache invalidation for any open surface read panels. This
is accepted — the read-model cache is a lazy fetch; panels will re-request on next access.

**In-flight coalescing gap (accepted):** `loadSurfaceRegistry` deduplicates concurrent calls
via a `loadPromise` guard — callers that arrive while a fetch is in-flight await the existing
promise rather than queuing a second fetch. An event that arrives after the debounce fires but
before the fetch completes will be coalesced into the in-flight result even if that result
predates the triggering change. This leaves the UI transiently stale until the next event.
Accepted — the window is short (one `loadSurfaceRegistry` round-trip) and self-corrects on
the next surface event.

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
  Accepted gap — channel saturation is pathological. For connection events, the next reconnect
  emits another `SurfacesChanged`. For disconnect events, no future reconnect occurs; the UI
  stays stale until the next unrelated surface event or page navigation. This is an accepted
  limitation of the dropped-event design.

## Testing

- **Backend unit:** assert `EventBroadcaster` receives `SurfacesChanged` after `SurfaceRegistration`
  message processed and after each `unregister_service` path; assert broadcast fires in the
  `Replaced` path when a surface provider was registered; assert `fail_in_flight_for_provider`
  is called before `unregister_service` in the `Replaced` path; assert no broadcast when
  `tenant_id` is `None` or when `Replaced` path has no provider registered.
- **Frontend unit:** mock SSE delivering `surfaces_changed`; assert `loadSurfaceRegistry` is
  called once; assert burst of three events debounces to one call; assert raw SSE frame
  `event: surfaces_changed\ndata: {}\n\n` is parsed and triggers the handler (not dropped).
- **E2E / manual:** open UI, connect a new agent-ssh service, verify new surfaces appear without
  page refresh; disconnect service, verify surfaces disappear.
