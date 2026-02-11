# Cross-controller notification delivery (notification outbox)

In a multi-controller deployment (multiple controller instances behind a load balancer sharing a database), the
in-memory `ServiceConnectionRegistry` cannot deliver push messages across controllers. The **notification outbox**
pattern solves this: push messages are written to a `controller_events` DB table alongside local delivery. A background
`EventPoller` on each controller picks up events from other controllers and delivers them to locally connected services.

**How it works:**

1. Each controller generates a unique `controller_id` (UUIDv7) at startup (stored in `AppState.controller_id`).
1. `NotificationService` wraps `ServiceConnectionRegistry` and writes outbox events on every `send()` and `broadcast()`
   call. MQTT credential-bearing messages (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`) are delivered
   locally but **not** written to the outbox to prevent plaintext credential persistence in the database. The MQTT
   service reconciles its state from the DB on reconnect.
1. `EventPoller` runs as a background task, polling every 1 second for new events from other controllers
   (`source_controller_id != self`), using a cursor-based approach (`id > last_seen_id`). The cursor only advances past
   events that were successfully delivered (or permanently skipped after 3 failed retries), providing at-least-once
   delivery semantics under backpressure.
1. Events are routed based on `target_service_id` (specific service) and `target_service_type` (`ServiceType` enum
   serialized as `"agent"`, `"mqtt"`, or `null` for broadcast).
1. Old events (>1 hour) are cleaned up every 5 minutes.

**Design rules:**

- `ServerRestarting` is local-only — it stays on `ServiceConnectionRegistry.broadcast_server_restarting_scattered()` and
  is NOT sent through `NotificationService`.
- MQTT credential-bearing messages (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`) are local-only —
  filtered by `is_mqtt_tenant_message()` in `NotificationService`. `MqttLeaseCoordinator` delivers these directly via
  `ServiceConnectionRegistry` without outbox writes.
- Outbox writes are fire-and-forget (errors are logged, not propagated to the caller).
- Single-controller overhead is negligible: one extra INSERT per push event, plus one SELECT/second returning 0 rows.

**Database table (`controller_events`):**

| Column | Type | Notes |
| --- | --- | --- |
| `id` | BIGINT AUTO_INCREMENT PK | Cursor for polling |
| `source_controller_id` | UUID NOT NULL | Controller that wrote the event |
| `target_service_id` | UUID NULL | NULL = broadcast |
| `target_service_type` | TEXT NULL | `"agent"`, `"mqtt"`, or NULL = all |
| `message_json` | TEXT NOT NULL | Serialized `ControllerMessage` |
| `created_at` | TIMESTAMP NOT NULL | For cleanup |

**Key files:**

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/notification_service.rs` | `NotificationService` — send + outbox write |
| `crates/ui/web-api/src/event_poller.rs` | `EventPoller` — background polling + delivery |
| `crates/shared/db/src/entity/controller_event.rs` | SeaORM entity |
| `crates/core/controller/src/migration/m20260209_000001_initial.rs` | Single consolidated migration |
