# Service Config Store

The service config store is a generic mechanism for services to persist named key/value config
entries on the controller. It provides multi-instance fanout so that all connected instances of
the same service receive config changes in real time, without requiring a shared database or
external coordination.

## Overview

- Services call `StoreServiceConfig` or `DeleteServiceConfig` via the wire protocol.
- The controller persists entries to the `tenant_service_config` or `global_service_config` DB
  tables (depending on whether a `tenant_id` is provided).
- Sensitive values are encrypted at rest using `EncryptedString` with per-entry AAD.
- On connect: the controller delivers all stored entries via `ServiceConfigDelivery` once per
  service `app_name` after mTLS authentication.
- On change: all connected instances of the same `service_app_name` receive
  `ServiceConfigUpdated` so they can apply the new or deleted entry immediately.

## Wire Messages

| Message | Direction | Description |
| --- | --- | --- |
| `store_service_config` | service → controller | Store a named config entry (tenant-scoped or global). The controller persists to DB, ACKs, and broadcasts `service_config_updated` to all other connected instances of the same `service_app_name`. |
| `delete_service_config` | service → controller | Delete a config entry by key. The controller removes from DB, ACKs, and broadcasts `service_config_updated` with `deleted: true`. |
| `service_config_ack` | controller → service | Acknowledges a store or delete operation. Carries `request_id` for correlation, a `success` flag, and an optional `error` message. Sent only to the requesting instance. |
| `service_config_delivery` | controller → service | Sent once after mTLS authentication. Contains all stored config entries (tenant-scoped and global) for the connecting service's `app_name`. |
| `service_config_updated` | controller → service | Pushed to all connected instances when any instance changes a config entry. Contains the updated key, value (absent when deleted), `tenant_id` (absent for global entries), and a `deleted` flag. |

See [Wire Protocol — Generic Service Config Messages](../api/wire-protocol.md#generic-service-config-messages)
for full payload schemas.

## Database Schema

### `tenant_service_config`

Stores service config entries scoped to a specific tenant.

| Column | Type | Description |
| --- | --- | --- |
| `id` | UUID (PK) | Row identifier |
| `service_app_name` | TEXT NOT NULL | Binary/crate name of the service (from `env!("CARGO_PKG_NAME")`) |
| `tenant_id` | UUID NOT NULL (FK → tenants) | Tenant that owns this entry |
| `key` | TEXT NOT NULL | Config entry key (max 512 chars, namespaced by convention) |
| `value` | TEXT NOT NULL | JSON config value, stored as `EncryptedString` |
| `created_at` | TIMESTAMPTZ NOT NULL | Creation time |
| `updated_at` | TIMESTAMPTZ NOT NULL | Last modification time |

UNIQUE constraint: `(service_app_name, tenant_id, key)`.

### `global_service_config`

Stores service config entries not scoped to any tenant (e.g. system-wide defaults).

| Column | Type | Description |
| --- | --- | --- |
| `id` | UUID (PK) | Row identifier |
| `service_app_name` | TEXT NOT NULL | Binary/crate name of the service |
| `key` | TEXT NOT NULL | Config entry key |
| `value` | TEXT NOT NULL | JSON config value, stored as `EncryptedString` |
| `created_at` | TIMESTAMPTZ NOT NULL | Creation time |
| `updated_at` | TIMESTAMPTZ NOT NULL | Last modification time |

UNIQUE constraint: `(service_app_name, key)`.

## ServiceConfigProxy (SDK)

`uptrakit-service-sdk` provides `ServiceConfigProxy` for request/response-correlated store and
delete operations. It abstracts the correlation between outgoing requests and incoming ACKs.

### Usage pattern

```rust
// 1. Create a pending request with a correlation ID
let (payload, pending) = config_proxy.store(key, value, tenant_id)?;

// 2. Send the wire message via the background task channel
bg_tx.send(ServiceMessage::StoreServiceConfig(payload)).await?;

// 3. Await the ACK (with timeout)
pending.wait(&config_proxy, Duration::from_secs(10)).await?;
```

- `ServiceConfigProxy::store(key, value, tenant_id)` — returns a `(StoreServiceConfigPayload,
  PendingServiceConfigRequest)` pair. The payload is ready to send; the pending handle waits
  for the matching ACK.
- `ServiceConfigProxy::delete(key, tenant_id)` — same pattern with
  `DeleteServiceConfigPayload`.
- `PendingServiceConfigRequest::wait(proxy, timeout)` — parks the current task until the
  controller sends a `ServiceConfigAck` with the matching `request_id`, or until the timeout
  elapses.
- `ServiceHandler::on_service_config_ack(ack)` — called by the SDK event loop when a
  `ServiceConfigAck` arrives. Routes the ACK to the waiting `PendingServiceConfigRequest` via
  an internal `DashMap<Uuid, oneshot::Sender<...>>`.

The proxy is safe to clone and share across tasks; all state is behind `Arc`.

## Usage Example: MQTT Service

The MQTT service uses the service config store as its source of truth for MQTT client
configurations. Here is the end-to-end flow:

### 1. On connect: receive stored config and start clients

The controller delivers `ServiceConfigDelivery` after mTLS authentication. The MQTT service
iterates all entries, deserializes each value as `ParsedMqttClientConfig`, and starts or
updates the corresponding `MqttClientHandle`. This is equivalent to the old
`TenantAssignments` handshake but driven by the generic mechanism.

### 2. Extension action: `create-client`

When a user submits the Create Client form in the MQTT Clients extension tab:

1. The extension handler deserializes the form params into `ParsedMqttClientConfig`.
2. It calls `config_proxy.store(key, serialized_config, tenant_id)` and sends the message
   via `bg_tx`.
3. It awaits `pending.wait(...)` to confirm the controller persisted the entry.
4. On success, it calls `start_client(config)` to establish the MQTT connection.

### 3. Extension action: `delete-client`

When a user deletes a client:

1. The extension handler calls `config_proxy.delete(key, tenant_id)` and sends via `bg_tx`.
2. It awaits `pending.wait(...)` to confirm deletion.
3. On success, it calls `stop_client(key)` to disconnect and clean up.

### 4. `ServiceConfigUpdated` from another instance

When a second MQTT service instance is running:

- On store: the service receives `ServiceConfigUpdated` with the new key/value. It parses
  the config and starts or updates the client.
- On delete: the service receives `ServiceConfigUpdated` with `deleted: true`. It looks up
  the client by key and stops it.

This replaces the old `TenantConfigUpdated` and `TenantRevoked` MQTT-specific messages.

## Security

- **Encryption at rest:** All config values are stored as `EncryptedString` (AES-256-GCM,
  envelope encryption via the master key ring). See
  [Secrets and Encryption](../security/secrets-and-encryption.md).
- **AAD format:** Each encrypted value uses the additional authenticated data (AAD) string
  `"uptrakit:service_config:{service_app_name}:{key}"` to bind the ciphertext to its entry.
  Swapping an encrypted value between keys or services is cryptographically detected.
- **Decryption before delivery:** The controller decrypts values before including them in
  `ServiceConfigDelivery` and `ServiceConfigUpdated` messages. The service receives plaintext
  JSON on the mTLS-secured WebSocket.
- **mTLS channel:** Config delivery and updates travel only over the authenticated mTLS
  WebSocket connection. The messages are not published to NATS.
