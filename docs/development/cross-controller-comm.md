# Cross-controller notification delivery

In a multi-controller deployment (multiple controller instances behind a load balancer sharing a database), the
in-memory `ServiceConnectionRegistry` cannot deliver push messages across controllers. When NATS JetStream is
configured, the **NATS transport** solves this: push messages are published to NATS alongside local delivery, and
each controller runs a consumer that delivers messages originating from other controllers to its locally connected
services.

Without NATS, the controller operates in **single-instance mode** — all push messages are delivered locally only.
This is the default and is sufficient for single-controller deployments.

## How it works

1. Each controller generates a unique `controller_id` (UUIDv7) at startup (stored in `AppState.controller_id`).
2. `NotificationService` wraps `ServiceConnectionRegistry` and, when NATS is configured, publishes messages to
   NATS JetStream on every `send()` and `broadcast()` call. MQTT credential-bearing messages
   (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`) are delivered locally but **not** published to
   NATS to prevent credential leakage. The MQTT service reconciles its state from the DB on reconnect.
3. Each controller runs a NATS consumer that pulls messages from JetStream, filters out self-originated messages
   (`source_controller_id == self`), and delivers them to locally connected services using the shared
   `event_delivery` routing logic.
4. Messages are routed based on `target_service_id` (specific service) and `target_capability` (a `Capability`
   string such as `"software_discovery"`, `"update_tracking"`, or broadcast to all services).
5. Failed deliveries are nacked and retried up to 3 times. Messages older than 24 hours are automatically
   discarded by the JetStream stream retention policy.

## Design rules

- `ServerRestarting` is local-only — it stays on `ServiceConnectionRegistry.broadcast_server_restarting_scattered()` and
  is NOT sent through `NotificationService`.
- MQTT credential-bearing messages (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`) are local-only —
  filtered by `is_mqtt_tenant_message()` in `NotificationService`. `MqttLeaseCoordinator` delivers these directly via
  `ServiceConnectionRegistry` without NATS publication.
- `ServiceCredentials` is local-only — filtered by `is_credential_message()` in `NotificationService`. Infrastructure
  credentials are delivered exclusively via WebSocket to services with credential capabilities.
- `RequestCaRotation` flows in the opposite direction: published by the external scheduler to
  `uptrakit.events.controller`, consumed by controllers to trigger `ca_rotation_trigger.notify_one()`.
- `RequestCrlRenewal` is published by controllers (after revocation) and by the `CrlRenewal` scheduler task
  to `uptrakit.events.controller`; each consuming controller fires `revocation_notify.notify_one()` to rebuild
  its CRL. See [PKI — CRLs](../security/pki-certificates.md#crls) for the full rebuild model.
- NATS publishes are fire-and-forget (errors are logged, not propagated to the caller).
- Without NATS, there is zero cross-controller overhead — `NotificationService` delivers locally only.

## NATS subject scheme

| Routing | Subject |
| --- | --- |
| Broadcast (no filter) | `uptrakit.events.broadcast` |
| Service-targeted | `uptrakit.events.service.<uuid>` |
| Capability-targeted | `uptrakit.events.capability.<cap>` |
| Controller events | `uptrakit.events.controller` |

## JetStream configuration

- **Stream name**: `UPTRAKIT_EVENTS`
- **Subjects**: `uptrakit.events.>`
- **Max age**: 24 hours
- **Storage**: File
- **Retention**: Limits-based
- **Consumer per controller**: `controller-<controller_id_hex>` (durable, pull-based, explicit ack, max 3 deliveries)

The stream is created via `get_or_create_stream` which is idempotent and safe for multi-controller startup races.

## Wire envelope

Messages are serialized as JSON `NatsEventEnvelope`:

```json
{
  "source_controller_id": "01234567-89ab-cdef-0123-456789abcdef",
  "target_service_id": null,
  "target_capability": "update_tracking",
  "message": { "type": "software_states", "..." : "..." },
  "created_at": "2026-02-27T12:00:00Z"
}
```

## Enabling NATS

NATS support requires the `nats` Cargo feature on `uptrakit-web-api` (cascaded via `uptrakit-controller`'s
`nats` feature). At runtime, pass `--nats-url` or set `UPTRAKIT_NATS_URL`:

```bash
uptrakit-controller --nats-url nats://localhost:4222
```

See [NATS Integration](nats-integration.md) for development details and
[NATS Deployment](../end-user/deployment/nats.md) for production guidance.

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/nats/src/` | `uptrakit-nats` — shared envelope, subjects, connection, publish |
| `crates/ui/web-api/src/notification_service.rs` | `NotificationService` — local delivery + optional NATS publish |
| `crates/ui/web-api/src/nats_transport.rs` | `NatsTransport` — NATS connect, publish, and consumer loop |
| `crates/ui/web-api/src/event_delivery.rs` | Shared delivery routing logic (used by NATS consumer) |
| `crates/core/controller/src/cli.rs` | `--nats-url` CLI argument |
| `crates/core/controller/src/main.rs` | NATS transport wiring and consumer spawn |
| `crates/core/scheduler/src/nats_notifier.rs` | `NatsSchedulerNotifier` — external scheduler NATS publisher |
