# Interactive Updates Security

This document covers the security model for interactive update sessions.

For the API reference, see [Interactive Updates API](../api/interactive-updates.md).
For the development guide, see [Interactive Updates Development](../development/interactive-updates.md).

## Threat Model

Interactive updates grant stdin access to a running process on a remote host. This is
equivalent to shell access and must be treated with the same level of trust as code
execution.

### Attack Surface

| Surface | Mitigation |
| --- | --- |
| Unauthorized stdin access | `ManageSoftware` permission required |
| Session hijacking | Single-writer enforcement; authenticated WebSocket |
| Stdin data interception | TLS encryption on all WebSocket connections |
| Denial of service via stdin flooding | Rate limit: 1000 messages/sec; 64 KB per message |
| Cross-tenant access | Tenant-scoped update history validation |
| NATS message injection | `UpdateStdinData` is not NATS-publishable |

## Permission Model

- **Viewing output**: `ViewSoftware` (unchanged from non-interactive mode).
- **Sending stdin / connecting interactively**: `ManageSoftware`.
- **Triggering interactive updates**: same permissions as non-interactive triggers.

No new `Permission` variant was introduced. The `ManageSoftware` permission already
implies code execution trust (it allows triggering updates that run arbitrary commands
on hosts).

## Authentication

The interactive WebSocket endpoint accepts authentication via:

1. Bearer token in the `Authorization` header.
2. Bearer token in the `token` query parameter (for browser WebSocket clients).

Both JWT session tokens and API tokens are supported. The token is validated using the
same authentication functions as all other API endpoints.

## Single-Writer Enforcement

Only one admin can have write access (stdin forwarding) to a given update at a time.
The `InteractiveSessionRegistry` tracks active sessions and rejects concurrent
connections with HTTP 409 Conflict.

Other admins can observe the update output via the read-only SSE endpoint
(`GET /api/v1/update-history/{id}/output/stream`).

## Audit Logging

Interactive session lifecycle events are audit-logged:

- Session connection (who connected, when).
- Session disconnection (who disconnected, when).

**Stdin content is NOT logged.** Stdin data may contain passwords, passphrases, or
other sensitive information entered at prompts. Logging raw stdin would create a
credential exposure risk.

## Wire Protocol Security

### `UpdateStdinData` Is Not NATS-Publishable

The `UpdateStdinData` message is session-targeted and is explicitly excluded from
NATS publication. This prevents stdin data from being broadcast to unintended
recipients in a multi-controller deployment. The message is only sent over the
direct WebSocket connection between the controller and the specific agent.

### Data Limits

| Limit | Value | Purpose |
| --- | --- | --- |
| `MAX_STDIN_DATA_LEN` | 64 KB | Maximum base64-encoded stdin data per message |
| Rate limit | 1000 messages/sec | Prevents stdin flooding |
| Signal values | 2 (SIGINT), 15 (SIGTERM) | Only safe signals allowed |

### Capability Gating

The controller only sends `UpdateStdinData` to agents that advertise the
`InteractiveUpdates` capability. A non-interactive agent receiving an unknown message
type hits the catch-all handler and logs a warning (existing forward-compatibility
pattern).

## PTY Security Considerations

### Local Agent

- The child process runs in its own session (`setsid()`) to isolate signal delivery.
- PTY allocation failures fall back to non-interactive mode (no crash, no data loss).
- The PTY slave fd is closed in the parent process after spawning the child.

### SSH Agent

- PTY allocation requires the `no-pty` restriction to be absent from the
  `authorized_keys` entry. When the `interactive` feature is enabled, this restriction
  is omitted.
- The SSH connection itself is already authenticated via mTLS between the controller
  and the SSH agent, and the SSH agent authenticates to the target host using its
  enrolled key pair.
- Running `host sync` with an interactive-enabled build updates existing
  `authorized_keys` entries.

## Graceful Degradation

| Failure | Behavior |
| --- | --- |
| PTY allocation fails | Update runs non-interactive; warning logged |
| Agent lacks `InteractiveUpdates` | Controller returns HTTP 409 |
| Admin disconnects mid-update | Update continues; stdin channel closes (process sees EOF) |
| Agent disconnects mid-session | Controller broadcasts error; frontend shows disconnect |

## See Also

- [Secure Development](secure-development.md) -- general security practices
- [PKI and Certificates](pki-certificates.md) -- mTLS authentication
- [Wire Protocol Security](../api/wire-protocol.md) -- message security
