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

### SSH agent-specific (service -> controller)

`report_hosts`, `version_check_results`, `update_started`, `update_output`, `update_result`

### MQTT-specific (service -> controller)

`register`, `release_tenants`, `mqtt_client_status`

### Shared (controller -> service)

`pong`, `enrolled`, `approved`, `rejected`, `certificate`, `error`, `service_settings`, `ca_bundle_updated`,
`request_cert_renewal`, `server_restarting`

### Agent-specific (controller -> service)

`check_versions`, `execute_update`

Both the regular agent and the SSH agent receive `check_versions` and `execute_update` messages. The `host_machine_id`
field in each payload determines which host the operation targets (see [`host_machine_id` Field](#host_machine_id-field)).

### MQTT-specific (controller -> service)

`registered`, `tenant_assignments`, `tenant_config_updated`, `tenant_revoked`

## `host_machine_id` Field

`CheckVersionsPayload` and `ExecuteUpdatePayload` both carry a required `host_machine_id: String` field as their first
field. The controller groups package assignments by `(service_id, host_machine_id)` and sends one message per host so
that each agent instance receives only the checks and updates relevant to its managed hosts.

### Regular agent behavior

The regular agent validates that the received `host_machine_id` matches the local machine ID collected at startup. A
mismatch is logged as a warning and the message is silently ignored (returns `Ok(None)`). This is a defensive sanity
check; under normal operation the controller only routes messages to the correct agent.

### SSH agent behavior

The SSH agent uses `host_machine_id` to look up which SSH host to connect to by calling `find_host_by_machine_id()`
against its local `ssh_hosts` database. If no matching host is found the message is rejected with an error. When
`ReportHosts` completes, `update_host_machine_id()` persists each remote host's `machine_id` so that subsequent
routing lookups succeed.

See [SSH Agent Architecture — Version Check and Update Execution](../architecture/ssh-agent.md#version-check-and-update-execution)
for the full dispatch flow.

## Replay Protection

Every envelope (`ServiceEnvelope` / `ControllerEnvelope`) carries a monotonically increasing `seq` starting at `1`.
Each connection tracks per-direction counters; mismatched sequences cause the connection to close with
`ErrorCode::SequenceError`.

Sequence validation is performed before full message deserialization. When a message has a valid sequence number but
an unrecognized type (e.g., from a newer protocol version), the sequence counter is correctly advanced and the message
is silently skipped. This ensures that unknown message types do not cause sequence mismatches on subsequent messages.

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

The `ErrorCode` enum is marked `#[non_exhaustive]` — new codes may be added in future protocol versions. Consumers should include a wildcard match arm.

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
reasons are defined as variants of the `CloseReason` enum in the `uptrakit_internal_wire::close_reason`
module. The enum provides `Display` (for sending) and `FromStr` (for receiving) implementations that
produce and parse the same wire strings. An `Unknown(String)` variant provides forward compatibility
for strings not yet recognized by the receiver.

| Variant | Wire String | Description |
| --- | --- | --- |
| `CloseReason::CertificateRotated` | `"certificate rotated"` | Service certificate was rotated; reconnect with new cert. |
| `CloseReason::CertificateRevoked` | `"certificate revoked"` | Service certificate was revoked; re-enrollment required. |
| `CloseReason::NoValidCertificate` | `"no valid certificate"` | No valid client certificate presented. |
| `CloseReason::InternalError` | `"internal error"` | Unexpected server-side error. |
| `CloseReason::CertificateNotRecognized` | `"certificate not recognized"` | Client certificate not recognized by the controller. |
| `CloseReason::ServiceDeactivated` | `"service deactivated"` | Service has been deactivated by an administrator. |
| `CloseReason::ServiceNotApproved` | `"service not approved"` | Service has not been approved for connection. |
| `CloseReason::ServiceNotFound` | `"service not found"` | Service ID not found in the database. |
| `CloseReason::EnrollmentTimeout` | `"enrollment timeout"` | Enrollment did not complete within the allowed time. |
| `CloseReason::VersionTooOld` | `"agent version too old"` | Agent protocol version is below the minimum supported. |
| `CloseReason::Superseded` | `"superseded by new connection"` | Another instance connected with the same service ID. |
| `CloseReason::RateLimitExceeded` | `"rate limit exceeded"` | Connection rate limit exceeded. |
| `CloseReason::Unknown(String)` | *(any other string)* | Forward-compatible catch-all for unrecognized reasons. |

Services should match on enum variants (not raw strings) to determine reconnection behavior:

- `CloseReason::CertificateRotated` → reconnect immediately with new certificate.
- `CloseReason::CertificateRevoked` → stop; re-enrollment needed.
- Other variants → reconnect with backoff or terminate depending on severity.

## `ServiceSettingsPayload` Fields

The `ServiceSettingsPayload` struct is sent by the controller as a `service_settings` message after an
authenticated connection is established. It carries runtime configuration for the connected service.

| Field | Type | Serde | Description |
| --- | --- | --- | --- |
| `renewal_window_hours` | `u16` | required | Hours before certificate expiry to initiate renewal |
| `ca_bundle_hash` | `String` | `#[serde(default)]` | Hash of the current CA bundle for staleness detection |
| `protocol_version` | `u16` | `#[serde(default = "protocol_version_default")]` | Wire protocol version used by the controller |
| `shutdown_timeout_seconds` | `Option<u32>` | `#[serde(default, skip_serializing_if)]` | Max seconds to wait during shutdown; present for agents, absent for MQTT |
| `ping_interval` | `Duration` | `#[serde(with = "duration_seconds")]` | Controller-managed ping interval; derived from per-service DB override or service-type default (300s agent/SSH agent, 15s MQTT) |

The `ping_interval` field is serialized as a `u32` number of seconds on the wire (e.g. `"ping_interval": 300`)
using the `duration_seconds` serde module. The controller reads `ping_interval_seconds` from the `services` table
for each service and falls back to service-type defaults when no override is set.

### `duration_seconds` serde module

The `duration_seconds` module (`uptrakit_internal_wire::duration_seconds`) provides `serialize` and `deserialize`
functions for converting `std::time::Duration` to/from a `u32` number of seconds in JSON. Use it with
`#[serde(with = "duration_seconds")]` on `Duration` fields that should appear as plain integer seconds on the wire.

## `HostInfo` Fields

The `HostInfo` struct (used inside `ReportHostsPayload.hosts`) contains:

| Field | Type | Description |
| --- | --- | --- |
| `machine_id` | `String` | Persistent system identifier (required) |
| `os_type` | `Option<String>` | Operating system type (e.g. `linux`, `macos`) |
| `os_version` | `Option<String>` | OS version or pretty name |
| `architecture` | `Option<String>` | CPU architecture (e.g. `x86_64`, `aarch64`) |
| `hostname` | `Option<String>` | Machine hostname |
| `ip_address` | `Option<String>` | IP address or hostname used to reach the host |

The `hostname` and `ip_address` fields were added as part of the SSH agent host reporting feature.
They use `#[serde(default)]` for backward compatibility -- agents that do not send these fields
will have them default to `None` on the controller side. No protocol version bump is required.

Both the regular agent and the SSH agent send `report_hosts`. The regular agent sets `hostname`
from local system calls. The SSH agent sets `hostname` from the remote host's `hostname` command
and `ip_address` from the SSH target address.

## Forward Compatibility

Several wire protocol enums are marked `#[non_exhaustive]` to allow adding new variants without breaking downstream
consumers:

- `ErrorCode` — new error codes may be added.
- `EnrollmentStatus` — new enrollment statuses may be added.
- `UpdateFinalStatus` — new update result statuses may be added.
- `DisconnectReason` — new disconnect reasons may be added.

Consumers matching on these enums must include a wildcard (`_`) arm to handle unknown variants gracefully.

## `MqttTenantConfig` Fields

The `MqttTenantConfig` struct is used in `tenant_assignments` and `tenant_config_updated` messages. Key fields:

| Field | Type | Description |
| --- | --- | --- |
| `mqtt_client_id` | UUID | Primary identifier from the `mqtt_clients` table |
| `tenant_id` | UUID | Tenant UUID |
| `enabled` | bool | Whether this MQTT client is enabled |
| `transport` | `tcp` / `tls` | Transport protocol |
| `host` | String | MQTT broker hostname |
| `port` | u16 | MQTT broker port |
| `client_id` | String | MQTT client ID for broker connection |
| `username` | Option&lt;SecretString&gt; | Broker authentication username |
| `password` | Option&lt;SecretString&gt; | Broker authentication password |
| `ca_pem` | Option&lt;SecretString&gt; | Custom CA certificate in PEM format for private brokers |
| `topic_prefix` | String | MQTT topic prefix |
| `updated_at` | i64 | Last update timestamp in milliseconds |

The `ca_pem` field is optional and uses `#[serde(default, skip_serializing_if = "Option::is_none")]` for
backward compatibility. When present, the MQTT service uses the PEM bytes as the trusted CA for TLS
connections instead of the system trust store. Credentials (`username`, `password`, `ca_pem`) use
`SecretString` for zeroize-on-drop and redacted debug output. The `ca_pem` field is included in the
config hash computation for change detection.

## AsyncAPI Specification

The full message schema and payload definitions are published in `crates/shared/wire/asyncapi.yaml`. Use this document
to generate clients or validate payload structures. Ensure protobuf/JSON serializers conform to the spec before
releasing.
