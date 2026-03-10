# Interactive Updates API

Interactive updates provide bidirectional terminal I/O for update sessions via a
dedicated WebSocket endpoint. This document covers the WebSocket protocol, authentication,
and message formats.

For the end-user guide, see [Interactive Updates](../end-user/interactive-updates.md).
For security considerations, see [Interactive Updates Security](../security/interactive-updates.md).

## WebSocket Endpoint

```text
GET /api/v1/update-history/{id}/interactive
```

Upgrades to a WebSocket connection for bidirectional communication with the update
process. This endpoint is feature-gated behind the `interactive` Cargo feature on the
controller.

### Authentication

The endpoint supports two authentication methods (checked in order):

1. **Query parameter**: `?token=<bearer_token>` -- required for browser WebSocket clients
   that cannot set custom headers.
2. **Authorization header**: `Authorization: Bearer <token>` -- standard API token
   authentication.

Both JWT session tokens and API tokens are accepted.

### Permissions

The authenticated user must have the `TriggerUpdates` permission. Stdin forwarding is
equivalent to code execution on the target host.

### Preconditions

The endpoint validates the following before upgrading:

| Check | Error |
| --- | --- |
| Update history record exists | 404 Not Found |
| Record belongs to the authenticated tenant | 404 Not Found |
| Update status is `in_progress` | 409 Conflict ("update is not in progress") |
| Agent is connected to this controller | 409 Conflict ("agent not connected") |
| No other interactive session is active | 409 Conflict ("another interactive session is active") |

### Rate Limits

Standard API rate limits apply to the WebSocket upgrade request. Once connected,
stdin messages are limited to 1000 messages per second per session. Each stdin data
payload is limited to 64 KB.

## Client-to-Server Messages

All messages are JSON objects with a `type` field.

### `stdin`

Send raw terminal input (keystrokes) to the update process.

```json
{
    "type": "stdin",
    "data": "<base64-encoded bytes>"
}
```

| Field | Type | Description |
| --- | --- | --- |
| `data` | string | Base64-encoded raw bytes. Supports binary data including control characters (e.g., `\x03` for Ctrl+C). |

### `signal`

Send a signal to the update process group.

```json
{
    "type": "signal",
    "signal": 2
}
```

| Field | Type | Description |
| --- | --- | --- |
| `signal` | integer | Signal number: `2` = SIGINT, `15` = SIGTERM. |

For SSH agents, signals are translated to the corresponding terminal control character
written to the PTY (e.g., SIGINT becomes `\x03`).

## Server-to-Client Messages

### `output`

A line of update output from the process.

```json
{
    "type": "output",
    "id": "<uuid>",
    "text": "Installing package...\n",
    "stream": "stdout",
    "timestamp": "2026-02-27T12:00:00Z",
    "seq": 42
}
```

The format matches the SSE `output` event from
`GET /api/v1/update-history/{id}/output/stream`.

### `completed`

The update has finished. The WebSocket closes after this message.

```json
{
    "type": "completed",
    "status": "completed",
    "error": null
}
```

### `stdin_attention`

The process appears to be waiting for stdin input (heuristic: no output for 10 seconds
while the process is still running).

```json
{
    "type": "stdin_attention",
    "hint": "Configuration file '/etc/foo.conf' has been modified..."
}
```

| Field | Type | Description |
| --- | --- | --- |
| `hint` | string or null | Optional hint about what the process might be waiting for. |

### `error`

An error occurred (e.g., agent disconnected).

```json
{
    "type": "error",
    "message": "agent disconnected"
}
```

## Connection Lifecycle

1. Client sends HTTP upgrade request with authentication.
2. Server validates preconditions and claims the interactive session.
3. Server replays existing output lines from the database.
4. Server streams new output in real time.
5. Client sends `stdin` and `signal` messages as needed.
6. On `completed`, the server sends the message and closes the WebSocket.
7. On client disconnect, the session is released but the update continues running.

## Wire Protocol Messages

The interactive WebSocket endpoint uses two wire protocol messages to communicate
between the controller and agent:

### `UpdateStdinData` (controller to agent)

Carries stdin data or a signal to the agent for a specific update.

```json
{
    "type": "update_stdin_data",
    "update_history_id": "<uuid>",
    "data": "<base64>",
    "signal": null
}
```

This message is session-targeted (not NATS-publishable) and is only sent to agents
that advertise the `InteractiveUpdates` capability.

### `StdinAttention` (agent to controller)

Sent when the agent detects the process may be waiting for stdin input.

```json
{
    "type": "stdin_attention",
    "update_history_id": "<uuid>",
    "hint": "optional context"
}
```

The controller broadcasts this as a `stdin_attention` event to SSE subscribers and
the interactive WebSocket session.

## Key Files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/interactive_ws.rs` | WebSocket endpoint handler |
| `crates/ui/web-api/src/interactive_sessions.rs` | Single-writer session registry |
| `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` | `StdinAttention` handler |
| `crates/shared/wire/src/payloads.rs` | `UpdateStdinDataPayload`, `StdinAttentionPayload` |
| `crates/shared/wire/src/messages.rs` | `UpdateStdinData`, `StdinAttention` message variants |

## See Also

- [Wire Protocol](wire-protocol.md) -- full message reference
- [SSE Events](sse-events.md) -- `stdin_attention` admin event
- [HTTP Web API](http-web-api.md) -- REST endpoints including update output streaming
