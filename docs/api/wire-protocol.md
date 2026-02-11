# Wire Protocol

Agent and MQTT service communication with the controller happens over a secure WebSocket (`/api/v1/ws/service`). Both service types share the same
`ServiceMessage`/`ControllerMessage` enums, with variants enabled per service type.

## Connection Types

| Type | Authentication | Purpose |
| --- | --- | --- |
| Anonymous | None | Enrollment (initial handshake). |
| Enrolled | Bearer token | Certificate request (`request_certificate`). |
| Authenticated | mTLS client cert | Normal operation (heartbeats, commands, data). |

Agents initiate outbound-only connections and never accept inbound traffic. MQTT services use the same enrollment model (shared service abstraction).

## Agent Lifecycle

1. Connect anonymously and send `enroll` with host info + optional enrollment token.
2. Controller assigns UUIDv7 `agent_id` and responds with `enrolled` (includes enrollment secret).
3. Agent generates an ECDSA P-256 keypair locally and submits a CSR (`request_certificate`).
4. Controller validates the CSR, signs it, and returns `certificate`.
5. Agent reconnects with mTLS and enters authenticated state.
6. Normal operation: `ping`/`pong`, `version_check_results`, `update_output`, etc.
7. On shutdown, agent sends `disconnecting` and waits for in-flight updates to finish.

## Message Types

### Shared (service → controller)

`ping`, `enroll`, `request_certificate`, `renew_certificate`, `disconnecting`

### Agent-specific (service → controller)

`report_host_info`, `version_check_results`, `update_started`, `update_output`, `update_result`

### MQTT-specific (service → controller)

`register`, `release_tenants`

### Shared (controller → service)

`pong`, `enrolled`, `approved`, `rejected`, `certificate`, `error`, `service_settings`, `ca_bundle_updated`, `request_cert_renewal`,
`server_restarting`

### Agent-specific (controller → service)

`check_versions`, `execute_update`

### MQTT-specific (controller → service)

`registered`, `tenant_assignments`, `tenant_config_updated`, `tenant_revoked`

## Replay Protection

Every envelope (`ServiceEnvelope` / `ControllerEnvelope`) carries a monotonically increasing `seq` starting at `1`. Each connection tracks
per-direction counters; mismatched sequences cause the connection to close with `ErrorCode::SequenceError`.

## AsyncAPI Specification

The full message schema and payload definitions are published in `crates/shared/wire/asyncapi.yaml`. Use this document to generate clients or validate
payload structures. Ensure protobuf/JSON serializers conform to the spec before releasing.
