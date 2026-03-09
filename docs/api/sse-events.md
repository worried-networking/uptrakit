# SSE Events API Reference

This document covers the two Server-Sent Events (SSE) endpoints for real-time event streaming.
For update output streaming and batch progress streaming, see [HTTP Web API](http-web-api.md).

## Device Auth SSE

Real-time stream for device authorization flow status. The CLI connects to this endpoint
after initiating a device flow to receive the API token as soon as the user approves
the request in the browser.

### Endpoint

```text
GET /api/v1/auth/device/stream?device_code=<raw_code>
```

### Authentication

None. This is a public endpoint.

### Query Parameters

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `device_code` | string | Yes | The `device_code` returned by `POST /api/v1/auth/device` |

### Keep-Alive

A `: keep-alive` comment is sent every 15 seconds to prevent proxies from closing
idle connections.

### Events

**`authorized`** -- the device code was approved. The stream closes after this event.

```text
event: authorized
data: {"token":"...","token_name":"..."}
```

| Field | Type | Description |
| --- | --- | --- |
| `token` | string | The API token |
| `token_name` | string | The token name |

**`expired`** -- the device flow expired (10-minute TTL reached). The stream closes
after this event.

```text
event: expired
data: {"message":"..."}
```

| Field | Type | Description |
| --- | --- | --- |
| `message` | string | Human-readable expiration message |

### Timeout

The stream has a 600-second timeout, matching the device flow TTL.

### Error Responses

| Status | Condition |
| --- | --- |
| 404 | `device_code` not found or already consumed |

## Admin Events SSE

Real-time stream of system-wide events for the admin dashboard. The frontend connects
to this endpoint to receive lightweight invalidation signals, then refetches the
relevant data from the corresponding REST endpoints.

### Endpoint

```text
GET /api/v1/events/stream
```

### Authentication

Bearer token required. The authenticated user must have the `ViewAgents` permission.

### Keep-Alive

A `: keep-alive` comment is sent every 15 seconds to prevent proxies from closing
idle connections.

### Events

All events use the SSE `event:` field to identify the event type and the `data:` field
to carry a JSON payload.

| Event Type | Data Fields | Description |
| --- | --- | --- |
| `host_updated` | `{"id":"<uuid>"}` | Host metadata updated |
| `host_created` | `{"id":"<uuid>"}` | New host created |
| `host_deleted` | `{"id":"<uuid>"}` | Host deactivated |
| `service_status_changed` | `{"id":"<uuid>","status":"<string>"}` | Service approved/rejected/deactivated |
| `software_item_updated` | `{"id":"<uuid>"}` | Software item updated |
| `software_item_created` | `{"id":"<uuid>"}` | New software item created |
| `version_check_completed` | `{"host_id":"<uuid>","software_item_id":"<uuid>"}` | Version check done |
| `update_started` | `{"update_history_id":"<uuid>","host_id":"<uuid>","software_item_id":"<uuid>"}` | Update execution started |
| `update_completed` | `{"update_history_id":"<uuid>","host_id":"<uuid>","software_item_id":"<uuid>","status":"<string>"}` | Update done |
| `discovery_completed` | `{"host_id":"<uuid>"}` | Autodiscovery done for host |
| `host_software_changed` | `{"host_id":"<uuid>"}` | Host software items updated |
| `batch_update_completed` | `{"host_id":"<uuid>"}` | Batch update done |
| `system_service_status_changed` | `{"id":"<uuid>","status":"<string>"}` | System service status changed |
| `scheduler_task_completed` | `{"task_id":"<uuid>"}` | Scheduled task done |
| `stdin_attention` | `{"update_history_id":"<uuid>","host_id":"<uuid>","software_item_id":"<uuid>","hint":"<string or null>"}` | Interactive update waiting for stdin input |

## Event Format Example

```text
event: host_updated
data: {"id":"550e8400-e29b-41d4-a716-446655440000"}

event: service_status_changed
data: {"id":"550e8400-e29b-41d4-a716-446655440001","status":"approved"}
```

## Reconnection Guidance

- **Events are invalidation signals.** Clients should refetch the full data from the
  relevant REST endpoint after receiving an event. The event payload contains only the
  minimum identifiers needed to know what changed.
- **No replay on reconnect.** The stream does not replay missed events. Clients must
  refetch current state from the REST API when they reconnect.
- **Exponential backoff.** Use exponential backoff for reconnection attempts: 1s, 2s,
  4s, 8s, 16s, capped at 30s.
- **Polling fallback.** The CLI falls back to periodic polling if the SSE connection
  fails.

## See Also

- [SSE Events Developer Guide](../development/sse-events.md) -- implementation details
  and architecture
- [Authentication Flows](auth-flows.md) -- device authorization flow details
- [HTTP Web API](http-web-api.md) -- REST API endpoints, including update output
  streaming and batch progress streaming SSE endpoints
