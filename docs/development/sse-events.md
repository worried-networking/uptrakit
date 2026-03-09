# SSE Admin Events Developer Guide

This guide covers the server-sent events (SSE) system used to push real-time
state-change notifications to the admin frontend. It explains the broadcaster
architecture, how to add new event types end-to-end, and how the frontend
consumes events.

For the HTTP API reference, see [docs/api/sse-events.md](../api/sse-events.md).
For the generated client stream methods, see
[docs/development/openapi-client.md](openapi-client.md). For authentication
requirements on the SSE endpoint, see
[docs/security/auth-and-authorization.md](../security/auth-and-authorization.md).

## Architecture overview

### EventBroadcaster (admin events)

`EventBroadcaster` lives in `crates/ui/web-api/src/event_broadcaster.rs` and is
stored in `AppState`. It maintains a `HashMap<Uuid, ChannelEntry>` (tenant ID to
broadcast channel) behind an `Arc<RwLock<...>>`.

Key properties:

- **Lazy creation** -- a `tokio::sync::broadcast` channel (capacity 512) is
  created on the first `subscribe()` call for a given tenant.
- **Auto-cleanup** -- when a subscriber disconnects and the `subscriber_count`
  drops to zero, the channel entry is removed from the map.
- **Fire-and-forget** -- `send()` and `send_global()` silently discard events
  when no subscribers are connected (`let _ = tx.send(event)`). Event emission
  never blocks the producing handler.
- **Tenant isolation** -- `send(tenant_id, event)` delivers only to subscribers
  of that tenant. `send_global(event)` iterates all active tenant channels (used
  for system-wide events like `SystemServiceStatusChanged`).
- **Thread-safe** -- the broadcaster is `Clone` (wraps `Arc`) and safe to pass
  across async tasks.

### DeviceFlowBroadcaster (device auth flow)

`DeviceFlowBroadcaster` in `crates/ui/web-api/src/device_flow_broadcaster.rs`
follows the same pattern but is keyed by `device_code_hash: String` instead of
tenant ID. It has a much smaller channel capacity (4) since only a few events
occur per flow (`StatusChanged`, `Expired`). Channels are explicitly created
with `create_channel()` and removed with `remove_channel()` after the flow is
consumed or expires.

## Adding a new event type

Adding a new event requires changes across four layers: shared types, openapi
client, backend producer, and frontend consumer.

### 1. Add the variant to `AdminEvent`

File: `crates/shared/web-api-types/src/events.rs`

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AdminEvent {
    // ... existing variants ...

    /// Description of the new event.
    NewVariant { id: Uuid },
}
```

Then add the corresponding arm to `event_name()`:

```rust
impl AdminEvent {
    pub fn event_name(&self) -> &'static str {
        match self {
            // ... existing arms ...
            Self::NewVariant { .. } => "new_variant",
        }
    }
}
```

Update the test helpers in the same file:

- Add the variant to `all_variants()`.
- Bump the count assertion in `event_name_count_matches_variant_count`.

### 2. Add the variant to `AdminSseEvent` (openapi client)

File: `crates/shared/openapi-client/src/events_stream.rs`

```rust
pub enum AdminSseEvent {
    // ... existing variants ...

    /// Description of the new event.
    NewVariant { id: Uuid },

    /// Catch-all for forward compatibility.
    Unknown { event_type: String, data: String },
}
```

Add a parse arm in `parse_typed_event()`:

```rust
fn parse_typed_event(event: RawSseEvent) -> std::result::Result<AdminSseEvent, StreamError> {
    match event.event_type.as_str() {
        // ... existing arms ...
        "new_variant" => Ok(AdminSseEvent::NewVariant {
            id: parse_id(&event.data)?,
        }),
        _ => Ok(AdminSseEvent::Unknown {
            event_type: event.event_type,
            data: event.data,
        }),
    }
}
```

Add a test for parsing the new event type.

### 3. Emit the event from a backend handler

In the route handler or service function where the state change occurs:

```rust
state
    .event_broadcaster
    .send(tenant_id, AdminEvent::NewVariant { id: entity_id })
    .await;
```

For system-wide events that should reach all connected tenants:

```rust
state
    .event_broadcaster
    .send_global(AdminEvent::NewVariant { id: entity_id })
    .await;
```

The `send()` call is fire-and-forget. Place it after the database write succeeds
but do not await it in a way that would block the HTTP response -- the current
API already handles this correctly since `send()` only acquires a read lock.

### 4. Add the event type to the frontend

File: `frontend/src/lib/sse.ts`

Add the new event name to the `AdminEventType` union:

```typescript
export type AdminEventType =
    | 'host_updated'
    // ... existing types ...
    | 'new_variant';
```

### 5. Subscribe in the relevant page

In the Svelte component that needs to react to the event:

```typescript
import { subscribeToEvent } from '$lib/stores/events.svelte';
import { onMount, onDestroy } from 'svelte';

let unsubscribe: (() => void) | undefined;

onMount(() => {
    unsubscribe = subscribeToEvent('new_variant', (data) => {
        // data.id contains the entity UUID
        // Refetch or invalidate the relevant data here
    });
});

onDestroy(() => {
    unsubscribe?.();
});
```

### 6. Update tests

- **Serde round-trip test** -- already covered if you added the variant to
  `all_variants()` and bumped the count.
- **`parse_typed_event` test** -- add a test in `events_stream.rs` that parses a
  raw SSE event into the new `AdminSseEvent` variant.

## Frontend integration

### Centralized events store

`frontend/src/lib/stores/events.svelte.ts` manages a single SSE connection
shared across all pages.

- **Lazy connect** -- the SSE connection opens when the first subscriber calls
  `subscribeToEvent()`.
- **Auto disconnect** -- when the last subscriber unsubscribes, the connection is
  closed and all debounce timers are cleared.
- **Reconnection** -- uses `connectEventStream()` from `$lib/sse.ts` with
  `maxReconnectAttempts: Infinity` and exponential backoff (1s to 30s).

### Debouncing

Rapid duplicate events (same event type + entity ID within 200ms) are collapsed
into a single callback invocation. The entity ID is extracted from `data.id`,
`data.host_id`, or `data.task_id`, whichever is present.

This prevents UI flicker when a burst of identical events arrives (for example,
multiple `host_software_changed` events for the same host during a batch
operation).

### Safety-net fallback polling

Pages that depend on SSE for freshness should keep a fallback polling interval of
5 minutes. This catches any events lost during reconnection windows or if SSE is
unavailable. The 5-minute interval (reduced from 30-60s in earlier designs) is
sufficient because SSE handles the common case.

### SSE transport

The frontend uses `fetch()` with `ReadableStream` instead of the browser
`EventSource` API. This is required because `EventSource` does not support custom
headers, and the SSE endpoint requires `Authorization: Bearer <token>`.

## Memory management

Both broadcasters follow the same lifecycle:

1. **Channel creation** -- on first `subscribe()` (EventBroadcaster) or explicit
   `create_channel()` (DeviceFlowBroadcaster).
2. **Active** -- events are delivered to all subscribers via
   `tokio::sync::broadcast`.
3. **Channel removal** -- EventBroadcaster removes the entry when
   `subscriber_count` reaches 0. DeviceFlowBroadcaster requires an explicit
   `remove_channel()` call.

If a subscriber falls behind, `tokio::sync::broadcast` returns a `Lagged` error
with the number of skipped messages. The SSE handler should log the lag and
continue receiving -- the frontend will refetch stale data on the next event.

## Fire-and-forget pattern

Event emission is designed to never interfere with the producing handler:

```rust
// In a route handler, after a successful DB write:
state.event_broadcaster.send(tenant_id, AdminEvent::HostUpdated { id }).await;
// Returns immediately -- no error to handle, no subscriber acknowledgement.
```

- `send()` acquires only a read lock on the channel map.
- `let _ = tx.send(event)` discards the `Result` (error means zero receivers).
- No logging on send failure -- this is expected when no clients are connected.
- The handler's HTTP response is never delayed by event delivery.
