# Wire Protocol

Agent, SSH agent, and MQTT service communication with the controller happens over a secure WebSocket (`/api/v1/ws/service`). All
service types share the same `ServiceMessage`/`ControllerMessage` enums, with variants enabled per service type.

## Connection Types

| Type | Authentication | Purpose |
| --- | --- | --- |
| Anonymous | None | Enrollment (initial handshake). |
| Enrolled | Bearer token | Certificate request (`request_certificate`). |
| Authenticated | mTLS client cert | Normal operation (heartbeats, commands, data). |

Agents and SSH agents initiate outbound-only connections and never accept inbound traffic. MQTT services and SSH agents
use the same enrollment model (shared service abstraction).

## Service Activity Tracking

For Agent, SSH agent, and MQTT services, the controller updates `services.last_seen_at` on every successful WebSocket connect
and on heartbeat ping/pong exchanges. The controller updates `services.ip_address` on each connect when a client IP is
resolved by the network middleware.

Accurate client IP tracking depends on trusted-proxy configuration; see
[docs/security/reverse-proxy-security.md](../security/reverse-proxy-security.md).

## Agent Lifecycle

1. Connect anonymously and send `enroll` with hostname, service type, and optional enrollment token.
1. Controller assigns UUIDv7 `service_id` and responds with `enrolled` (includes enrollment secret).
1. Agent generates an ECDSA P-256 keypair locally and submits a CSR (`request_certificate`).
1. Controller validates the CSR, signs it, and returns `certificate`.
1. Agent reconnects with mTLS and enters authenticated state.
1. Normal operation: `ping`/`pong`, `version_check_results`, `update_output`, etc.
1. On shutdown, agent sends `disconnecting` and waits for in-flight updates to finish.

## Message Types

### Shared (service -> controller)

`ping`, `enroll`, `request_certificate`, `renew_certificate`, `disconnecting`, `error`

### Agent-specific (service -> controller)

`report_hosts`, `version_check_results`, `update_started`, `update_output`, `update_result`

### MQTT-specific (service -> controller)

`register`, `release_tenants`, `mqtt_client_status`

### Shared (controller -> service)

`pong`, `enrolled`, `approved`, `rejected`, `certificate`, `error`, `service_settings`, `ca_bundle_updated`,
`request_cert_renewal`, `server_restarting`

### Agent-specific (controller -> service)

`check_versions`, `execute_update`

### MQTT-specific (controller -> service)

`registered`, `tenant_assignments`, `tenant_config_updated`, `tenant_revoked`

## Replay Protection

Every envelope (`ServiceEnvelope` / `ControllerEnvelope`) carries a monotonically increasing `seq` starting at `1`.
Each connection tracks per-direction counters; mismatched sequences cause the connection to close with
`ErrorCode::SequenceError`.

## Connection Limits

| Limit | Value | Description |
| --- | --- | --- |
| Maximum incoming message size | 1 MB (1,048,576 bytes) | Messages exceeding this limit are rejected and the connection is closed. |
| Anonymous connection timeout | 30 seconds | An anonymous connection that does not send `Enroll` within 30 seconds is closed. |
| Update output cap | 1 MB | The controller caps accumulated `update_history.output`. Further `UpdateOutput` messages are silently dropped. |
| Approval polling interval | 5 seconds | The controller polls the database for approval status changes at a fixed 5-second interval. |
| TCP connect timeout (client) | 30 seconds | The enrollment client aborts the TCP connection if it cannot be established within 30 seconds. |
| Response timeout (client) | 60 seconds | The `Enroll` and `RequestCertificate` request-response exchanges time out after 60 seconds. |
| Approval timeout (client) | 30 minutes | The `wait_for_approval` loop times out after 30 minutes. The caller retries the enrollment flow on timeout. |

## Error Codes

| Code | Description |
| --- | --- |
| `bad_request` | Malformed or invalid message. |
| `enrollment_failed` | Enrollment could not be completed. |
| `not_approved` | Service has not been approved yet. |
| `forbidden` | Service is not authorized for this action. |
| `certificate_error` | CSR validation or certificate issuance failed. |
| `internal_error` | Unexpected server-side error. |
| `agent_version_too_old` | Agent protocol version is below the minimum supported by the controller. |
| `sequence_error` | Incoming sequence number does not match the expected value. Connection is closed. |

## WebSocket Close Reasons

When the controller closes a WebSocket connection, it includes a reason string in the close frame. These
reasons are defined as constants in the `uptrakit_internal_wire::close_reason` module to prevent
mismatches between sender and receiver.

| Constant | Value | Description |
| --- | --- | --- |
| `CERTIFICATE_ROTATED` | `"certificate rotated"` | Service certificate was rotated; reconnect with new cert. |
| `CERTIFICATE_REVOKED` | `"certificate revoked"` | Service certificate was revoked; re-enrollment required. |
| `NO_VALID_CERTIFICATE` | `"no valid certificate"` | No valid client certificate presented. |
| `INTERNAL_ERROR` | `"internal error"` | Unexpected server-side error. |
| `CERTIFICATE_NOT_RECOGNIZED` | `"certificate not recognized"` | Client certificate not recognized by the controller. |
| `SERVICE_DEACTIVATED` | `"service deactivated"` | Service has been deactivated by an administrator. |
| `SERVICE_NOT_APPROVED` | `"service not approved"` | Service has not been approved for connection. |
| `SERVICE_NOT_FOUND` | `"service not found"` | Service ID not found in the database. |
| `ENROLLMENT_TIMEOUT` | `"enrollment timeout"` | Enrollment did not complete within the allowed time. |
| `AGENT_VERSION_TOO_OLD` | `"agent version too old"` | Agent protocol version is below the minimum supported. |
| `SUPERSEDED` | `"superseded by new connection"` | Another instance connected with the same service ID. |
| `RATE_LIMIT_EXCEEDED` | `"rate limit exceeded"` | Connection rate limit exceeded. |

Services should match on these constants (not raw strings) to determine reconnection behavior:

- `CERTIFICATE_ROTATED` → reconnect immediately with new certificate.
- `CERTIFICATE_REVOKED` → stop; re-enrollment needed.
- Other reasons → reconnect with backoff or terminate depending on severity.

## AsyncAPI Specification

The full message schema and payload definitions are published in `crates/shared/wire/asyncapi.yaml`. Use this document
to generate clients or validate payload structures. Ensure protobuf/JSON serializers conform to the spec before
releasing.
