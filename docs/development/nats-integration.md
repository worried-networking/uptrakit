# NATS Integration (Development Guide)

This document covers the NATS JetStream integration from a development perspective. For production deployment
guidance, see [NATS Deployment](../end-user/deployment/nats.md). For the high-level design, see
[Cross-Controller Communication](cross-controller-comm.md).

## Feature flag

NATS support is gated behind the `nats` Cargo feature:

- **`uptrakit-web-api`**: `nats = ["dep:async-nats"]` — enables the `nats_transport` module and NATS-related
  code paths in `NotificationService`.
- **`uptrakit-controller`**: `nats = ["uptrakit-web-api/nats"]` — cascades the feature to the web API crate.

This pattern matches the existing `oidc` feature flag approach. When the `nats` feature is disabled, all
NATS-related code is compiled out via `#[cfg(feature = "nats")]` and the controller operates in single-instance
mode.

## Crate dependency

The `async-nats` crate (version 0.46+) is declared as a workspace dependency and used as an optional dependency
in `uptrakit-web-api`. It provides the NATS client, JetStream API, and consumer abstractions.

## Architecture

### Module structure

```text
crates/ui/web-api/src/
├── notification_service.rs   # NotificationService (local + optional NATS)
├── nats_transport.rs         # NatsTransport (connect, publish, consumer)
└── event_delivery.rs         # Shared delivery routing (used by NATS consumer)
```

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
| `push_software_states_for_tenant(db, tenant_id)` | Local capability broadcast + NATS publish to `mqtt_bridge` |

### NatsTransport

`NatsTransport` handles the NATS connection, stream setup, message publishing, and the consumer loop.

- **`connect(url, controller_id)`** — Connects to NATS, creates the JetStream context, and ensures the
  `UPTRAKIT_EVENTS` stream exists.
- **`publish(...)`** — Fire-and-forget: serializes a `NatsEventEnvelope` and publishes to the appropriate
  subject. Errors are logged, not propagated.
- **`run_consumer(registry, db, cancel)`** — Main consumer loop: pulls messages from JetStream, filters
  self-originated messages, delivers via `event_delivery::deliver_event()`, and ack/nacks.

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

## Edge cases

- **Stream creation race**: `get_or_create_stream` is idempotent — safe for multiple controllers starting
  simultaneously.
- **NATS disconnect/reconnect**: The `async-nats` crate handles reconnection automatically. The consumer
  resumes from the last acknowledged message.
- **Self-filtering**: The consumer skips messages where `source_controller_id == self.controller_id` to avoid
  processing its own messages.
- **MQTT credential messages**: `is_mqtt_tenant_message()` in `NotificationService` prevents credential-bearing
  messages from reaching NATS. See [Secrets and Encryption](../security/secrets-and-encryption.md).

## Related documentation

- [Cross-Controller Communication](cross-controller-comm.md) — high-level design
- [NATS Deployment](../end-user/deployment/nats.md) — production configuration
- [Secrets and Encryption](../security/secrets-and-encryption.md) — credential filtering rationale
- [Secure Development](../security/secure-development.md) — security requirements for contributors
