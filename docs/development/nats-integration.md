# NATS Integration (Development Guide)

This document covers the NATS JetStream integration from a development perspective. For production deployment
guidance, see [NATS Deployment](../end-user/deployment/nats.md). For the high-level design, see
[Cross-Controller Communication](cross-controller-comm.md).

## NATS URL — DB Persistence and Configuration

The NATS server URL is stored in the global `settings` table under the key `nats.url` (encrypted with
AES-256-GCM). This allows the URL to be configured without modifying startup scripts and persists across
restarts.

### Configuration priority at startup

The `--nats-url` CLI flag is reconciled with the database value using the standard 5-case priority (see
[Settings Runtime](../api/settings-runtime.md#nats-settings-feature-nats)):

1. DB value + CLI provided (different) + `--force-settings-override`: CLI wins, DB updated.
2. DB value + CLI provided (different): DB wins, warning logged.
3. DB value + CLI same or absent: DB value used.
4. No DB value + CLI provided: CLI value encrypted and saved to DB.
5. No DB value + CLI absent: NATS transport disabled.

After the first run with `--nats-url`, the flag is no longer required. The stored URL is re-encrypted using
the controller's master key on any write.

### Runtime API

The NATS URL can be updated at runtime via `PUT /api/v1/settings/nats` or `uptrakit settings nats set`.
**Hot-reload is intentionally not supported** — changing the URL updates the DB and in-memory snapshot,
but does not reconnect the live NATS transport. The controller must be restarted for the change to take
effect.

The `SettingsSnapshot.nats_url` field holds a `MaskedUrl` whose `Display`/`Debug`/`Serialize` automatically
replace the password component with `***`, ensuring credentials are never leaked into logs or API responses.

### `MaskedUrl` type

`MaskedUrl` (`crates/shared/web-api-types/src/masked_url.rs`) is a newtype around a URL string:

- `MaskedUrl::new(raw)` — wraps the raw URL (may contain password).
- `MaskedUrl::as_raw_str()` — returns the raw URL for internal use (connecting, encrypting).
- `MaskedUrl::masked()` — returns the URL with the password replaced by `***`.
- `Display`, `Debug`, `Serialize` all call `masked()` — safe for logs and API responses.
- `Deserialize` accepts a plain string and wraps it.

## Connection URL and TLS

`NatsConnection::connect(url)` accepts any NATS connection URL. The URL scheme determines whether the
connection is encrypted:

| Scheme | Transport | Notes |
| --- | --- | --- |
| `nats://` | Plaintext TCP | **Not recommended for production.** Emits a `tracing::warn!` at startup. |
| `nats-tls://` | TLS | Recommended for production deployments. |
| `nats://` + `tls_required: true` server config | TLS (server-side enforcement) | Accepted, but `nats-tls://` is preferred so the client also validates the requirement. |

When a `nats://` URL is detected, `NatsConnection::connect` emits:

```text
WARN uptrakit_nats::connection: connecting to NATS over plaintext (nats://); use nats-tls:// or
     enable TLS on the server side in production — see docs/security/secrets-and-encryption.md
```

**In production environments always use `nats-tls://`.** NATS carries cross-controller
`ControllerMessage` payloads including software state updates and CA rotation requests. Plaintext
transport exposes these messages to any observer on the network segment between the controller and
the NATS server.

See [Secrets and Encryption — NATS Transport Security](../security/secrets-and-encryption.md#nats-transport-security)
for the full security rationale.

## Crate structure

NATS primitives are split across two crates:

| Crate | Path | Purpose |
| --- | --- | --- |
| `uptrakit-nats` | `crates/shared/nats/` | Shared: envelope, subjects, connection, publish |
| `uptrakit-web-api` | `crates/ui/web-api/` | Controller-specific: `NatsTransport`, consumer, delivery |

The `uptrakit-nats` crate is unconditional — it is always compiled (no feature gate). Both the
controller's `NatsTransport` and the external scheduler's `NatsSchedulerNotifier` depend on it.

### `uptrakit-nats` contents

```text
crates/shared/nats/src/
├── lib.rs               # Re-exports
├── config_protection.rs # Encrypt/decrypt plugin configs for NATS transit
├── envelope.rs          # NatsEventEnvelope
├── subjects.rs          # determine_subject(), STREAM_NAME, SUBJECT_PREFIX, STREAM_MAX_AGE
├── connection.rs        # NatsConnection: connect(), ensure_stream(), publish(), publish_envelope()
└── error.rs             # NatsError enum
```

### Controller NATS modules

```text
crates/ui/web-api/src/
├── notification_service.rs   # NotificationService (local + optional NATS)
├── nats_transport.rs         # NatsTransport (connect, publish, consumer)
└── event_delivery.rs         # Shared delivery routing (used by NATS consumer)
```

## Feature flag

NATS support on the controller is gated behind the `nats` Cargo feature:

- **`uptrakit-web-api`**: `nats = ["dep:uptrakit-nats"]` — enables the `nats_transport` module and NATS-related
  code paths in `NotificationService`.
- **`uptrakit-controller`**: `nats = ["uptrakit-web-api/nats"]` — cascades the feature to the web API crate.

This pattern matches the existing `oidc` feature flag approach. When the `nats` feature is disabled, all
NATS-related code is compiled out via `#[cfg(feature = "nats")]` and the controller operates in single-instance
mode.

The external scheduler (`uptrakit-scheduler`) always depends on `uptrakit-nats` directly — it does not use the
`nats` feature flag since NATS is required for its operation.

### NotificationService

`NotificationService` wraps `ServiceConnectionRegistry` and provides the push API. When the `nats` feature is
enabled and a `NatsTransport` is attached via `with_nats()`, messages are published to NATS JetStream in
addition to local delivery. Without NATS, messages are delivered locally only.

Key methods:

| Method | Behaviour |
| --- | --- |
| `send(service_id, msg)` | Local delivery + NATS publish (unless MQTT credential message) |
| `broadcast(msg)` | Local broadcast + NATS publish (unless MQTT credential message) |
| `publish_controller_event(msg)` | NATS-only publish to controller subject (no-op without NATS) |
| `signal_software_states_changed(tenant_id)` | Sends `SoftwareStatesChanged` signal; controller's event delivery loads and pushes states to `update_tracking` services |

### NatsTransport (controller-specific)

`NatsTransport` handles the controller-side NATS operations: publishing via `NatsConnection` and the
consumer loop.

- **`connect(url, controller_id)`** — Creates a `NatsConnection` and wraps it with controller-specific
  consumer state.
- **`publish(...)`** — Fire-and-forget: serializes a `NatsEventEnvelope` via `NatsConnection::publish()`
  to the appropriate subject. Errors are logged, not propagated.
- **`run_consumer(registry, db, ca_rotation_trigger, cancel)`** — Main consumer loop: pulls messages
  from JetStream, filters self-originated messages, delivers via `event_delivery::deliver_event()`,
  handles `RequestCaRotation` events, and ack/nacks.

### NatsConnection (shared)

`NatsConnection` in `uptrakit-nats` provides the core NATS operations used by both the controller
and the external scheduler:

- **`connect(url)`** — Connects to NATS, creates the JetStream context.
- **`ensure_stream()`** — Creates or updates the `UPTRAKIT_EVENTS` stream (idempotent).
- **`publish(subject, envelope)`** — Publishes a `NatsEventEnvelope` to a subject.
- **`publish_envelope(envelope)`** — Determines the subject from the envelope and publishes.

### Event delivery

The `event_delivery` module contains the shared routing logic extracted from the former `EventPoller`. It is
used by the NATS consumer to deliver messages to locally connected services:

- `deliver_event(registry, db, target_service_id, target_capability, msg)` — Routes a message by service ID,
  capability, or broadcast.
- `deliver_mqtt_event(registry, msg)` — Delivers MQTT-specific events (`SoftwareStates`) via capability
  broadcast.
- `deliver_controller_event(db, registry, msg)` — Handles `MqttClientCreated` by triggering lease attempts.

## Subject scheme

| Routing | Subject | When |
| --- | --- | --- |
| Broadcast | `uptrakit.events.broadcast` | `target_service_id` and `target_capability` are both `None` |
| Service-targeted | `uptrakit.events.service.<uuid>` | `target_service_id` is `Some` |
| Capability-targeted | `uptrakit.events.capability.<cap>` | `target_capability` is `Some` (not `"controller"`) |
| Controller events | `uptrakit.events.controller` | `target_capability` is `Some("controller")` |

Service-targeted routing takes precedence over capability-targeted when both are specified.

## JetStream stream configuration

| Setting | Value |
| --- | --- |
| Stream name | `UPTRAKIT_EVENTS` |
| Subjects | `uptrakit.events.>` |
| Max age | 24 hours |
| Storage | File |
| Retention | Limits |

## Consumer configuration

Each controller creates a durable pull consumer named `controller-<controller_id_hex>`:

| Setting | Value |
| --- | --- |
| Deliver policy | `DeliverNew` (on first creation) |
| Ack policy | Explicit |
| Max deliver | 3 |
| Filter subject | `uptrakit.events.>` |
| Pull batch size | 10 |
| Pull expiry | 5 seconds |

## Testing

### Unit tests (no NATS required)

```bash
# Default features (no NATS)
cargo test -p uptrakit-web-api

# With NATS feature (compiles NATS code, runs non-integration tests)
cargo test -p uptrakit-web-api --features nats
```

### Integration tests (require running NATS)

Start a NATS server with JetStream enabled:

```bash
nats-server -js
```

Run the integration tests:

```bash
cargo test -p uptrakit-web-api --features nats nats -- --ignored
```

Set `NATS_URL` to override the default `nats://localhost:4222`:

```bash
NATS_URL=nats://custom-host:4222 cargo test -p uptrakit-web-api --features nats nats -- --ignored
```

## Plugin Config Protection

Plugin configs (`PluginAssignment.config`, `DiscoveryPluginAssignment.config`)
may contain sensitive credentials (API tokens, registry passwords). These
configs are encrypted before NATS publication and decrypted on receipt.

### Mechanism

The `uptrakit_nats::config_protection` module provides two functions:

- `encrypt_message_configs(msg)` — Called in `NatsConnection::publish()` before
  serialization. Serializes each `serde_json::Value` config to a JSON string,
  encrypts it with `uptrakit_crypto::encrypt_str()` (AES-256-GCM + AAD), and
  replaces the config with `Value::String("ENC:v3:...")`.
- `decrypt_message_configs(msg)` — Called in `NatsTransport::run_consumer()`
  after deserialization. Detects encrypted config strings via
  `uptrakit_crypto::is_encrypted()`, decrypts with `decrypt_str()`, and
  restores the original `serde_json::Value`.

### Affected message types

| Variant | Encrypted fields |
| :--- | :--- |
| `CheckVersions` | `assignments[].detect_version.config`, `assignments[].fetch_releases.config` |
| `ExecuteUpdate` | `detect_version_plugin.config`, `execute_update_plugin.config` |
| `ExecuteBatchUpdate` | `plugin_config` |
| `DiscoverSoftware` | `plugins[].config` |

All other `ControllerMessage` variants pass through unchanged.

### Error handling

Encryption or decryption failures are logged at `warn` level and the config is
left unchanged (graceful degradation). The agent will receive an encrypted
string instead of a JSON object and fail the plugin operation, but no crash or
data loss occurs.

### Backward compatibility

`decrypt_message_configs()` checks each config field: if it is already a
`Value::Object` (not encrypted), it is returned unchanged. This ensures
compatibility during rolling upgrades where one controller publishes
unencrypted messages while another has the new code.

### Prerequisites

Both the publishing controller (or external scheduler) and the consuming
controller must have initialized the master key via
`uptrakit_crypto::init_master_key()`. The external scheduler receives the
master key via `ServiceCredentials` at connection time.

## Edge cases

- **Stream creation race**: `get_or_create_stream` is idempotent — safe for multiple controllers starting
  simultaneously.
- **NATS disconnect/reconnect**: The `async-nats` crate handles reconnection automatically. The consumer
  resumes from the last acknowledged message.
- **Self-filtering**: The consumer skips messages where `source_controller_id == self.controller_id` to avoid
  processing its own messages.
- **MQTT credential messages**: `is_mqtt_tenant_message()` in `NotificationService` prevents credential-bearing
  messages from reaching NATS. See [Secrets and Encryption](../security/secrets-and-encryption.md).
- **Service credential messages**: `is_credential_message()` in `NotificationService` prevents
  `ServiceCredentials` from reaching NATS. Credentials are delivered exclusively via WebSocket.
- **`RequestCaRotation`**: Published by the external scheduler to `uptrakit.events.controller`. The
  controller's NATS consumer handles it by triggering `ca_rotation_trigger.notify_one()`.
- **Plugin config encryption**: NATS-published messages have AES-256-GCM
  encrypted plugin config fields. See [Plugin Config Protection](#plugin-config-protection).

## Related documentation

- [Cross-Controller Communication](cross-controller-comm.md) — high-level design
- [NATS Deployment](../end-user/deployment/nats.md) — production configuration
- [Scheduler Engine](scheduler-engine.md) — scheduler engine crate internals
- [External Scheduler Deployment](../end-user/deployment/external-scheduler.md) — external scheduler setup
- [Secrets and Encryption](../security/secrets-and-encryption.md) — credential filtering rationale
- [Secure Development](../security/secure-development.md) — security requirements for contributors
