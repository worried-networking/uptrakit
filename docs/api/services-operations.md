# Services and Operations

## Agents

- `/api/v1/services` lists all tenant services (agents, SSH agents). The MQTT bridge is now a system service
  and is listed at `/api/v1/system-services` instead.
- `/api/v1/services/{id}` retrieves a single service by UUID.
- `/api/v1/services/{id}/approve`, `/api/v1/services/{id}/reject`,
  `DELETE /api/v1/services/{id}`, and `/api/v1/services/{target_id}/merge`
  manage the service lifecycle (approve, reject, deactivate, merge).

### Service deactivation (`DELETE /api/v1/services/{id}`)

Service deactivation is fully transactional. The handler wraps all three mutations in a single database
transaction:

1. Soft-delete the service record.
2. Revoke all certificates associated with the service.
3. Bump the CRL (Certificate Revocation List) revocation version so that the updated list is published
   to connected agents without delay.

If any step fails, the entire transaction is rolled back and the service remains active. This prevents a
partially-deactivated state where the service record is marked deleted but the certificates remain valid
in the CRL — which would allow a deactivated service to continue authenticating via mTLS until the next
CRL refresh.

- `/api/v1/agents/{agent_id}/version-check` instructs the controller to send `check_versions` over WebSocket.
- `/api/v1/agents/{agent_id}/execute-update` triggers `execute_update` with the software item ID(s).
- `/api/v1/update-history` provides audit logs for updates; each row tracks `status`, `output`, `actor_type`, `actor_id`, and `tenant_id`.

## Configuring a Service

`PUT /api/v1/services/{id}` accepts an `UpdateServiceRequest` body with the following optional fields:

| Field                   | Type             | Description                                                                                                                                                                      |
| ----------------------- | ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ping_interval_seconds` | `u32` (optional) | Custom ping interval in seconds. `0` clears the override; minimum positive value is `5`. Omit to keep current value.                                                             |
| `cert_lifetime_hours`   | `u32` (optional) | Per-service certificate lifetime in hours. `0` clears the per-service override and reverts to the global default. Valid positive range: `1`–`17520`. Omit to keep current value. |

When `cert_lifetime_hours` is set, it takes precedence over the global agent certificate lifetime setting
(`PUT /api/v1/settings/agent-certificates`) at certificate signing time — both during initial enrollment
and on renewals.

The `ServiceResponse` includes `cert_lifetime_hours` only when a per-service override is active (the field
is omitted from JSON when the service uses the global default).

See also: [PKI and Certificate Lifecycle](../security/pki-certificates.md) for renewal window details and
the per-service override section.

### Ping interval mechanics

The ping interval is controller-managed and per-service configurable. The `services` DB table has a
nullable `ping_interval_seconds INTEGER` column. The controller reads this value per-service and falls
back to profile-based defaults, derived from `ServiceProfile::default_ping_interval_secs()`, when the
column is `NULL`:

| Profile         | Default ping interval        | Services                |
| --------------- | ---------------------------- | ----------------------- |
| `UpdateTracker` | 15s (MQTT lease heartbeat)   | MQTT bridge             |
| `Scheduler`     | 60s (less latency-sensitive) | External task scheduler |
| `Agent`         | 300s (5 minutes)             | Local agent, SSH agent  |

The `ServiceResponse.ping_interval_seconds` field mirrors the same optionality: `None` means the
service is using the profile-based default rather than a per-service override.

- **Wire protocol**: `ServiceSettingsPayload.ping_interval` is a required `Duration` field serialized as
  a whole-second `u32` via `#[serde(with = "duration_seconds")]`. The `duration_seconds` module in
  `uptrakit-wire` converts between `std::time::Duration` and `u32` seconds on the wire.
  `ServiceSettingsPayload.tenant_id` is an `Option<Uuid>` present for tenant-scoped services (agents, SSH
  agents) and absent for system services.
- **SDK event loop**: The ping timer starts as `None` and is created when the first `ServiceSettings`
  message arrives with the controller-provided `ping_interval`. The `ServiceHandler::ping_interval()`
  trait method has been removed — services no longer declare their own interval.
- **CLI**: `uptrakit services update <id> --ping-interval <seconds>` (and the system-service equivalent,
  `uptrakit system-services update <id> --ping-interval <seconds>`). `0` clears the override.
- **OpenAPI client**: `update_service(&self, id: &Uuid, req: &UpdateServiceRequest) -> Result<ServiceResponse>`
  in `crates/shared/openapi-client/src/services.rs`.
- **Frontend**: Service page context menu includes an "Edit Ping Interval" dialog.

## MQTT Service

> **Note:** The MQTT bridge binary (`uptrakit-mqtt`) now enrolls as a **system service** and is
> managed via `/api/v1/system-services`, not `/api/v1/services`. It no longer appears in the
> `/api/v1/services` listing. See [System Services](#system-services) below for details.

- `/api/v1/enrollment-tokens` manages multiple named enrollment tokens with optional capability scoping,
  usage limits, and TTL. See [Enrollment Tokens API](enrollment-tokens.md) for full details.
- MQTT clients receive `tenant_assignments`, `tenant_config_updated`, and `tenant_revoked` commands after assignment.
- Agent services use the activity tracking fields in `services`: `ip_address` is refreshed on each WebSocket connect, and `last_seen_at`
  is refreshed on connect and heartbeat (`ping`).

## Shared Service Startup Flow

All service types (agents, SSH agents, MQTT) implement the `ServiceHandler` trait from `uptrakit-service-sdk` and delegate
their startup to `run_service_lifecycle()`. Each service declares associated constants (`DIR_NAME`, `SERVICE_LABEL`)
and implements callbacks (`on_connected`, `on_message`, `on_shutdown`, etc.). The SDK owns the event loop
and handles all common plumbing:

1. Parse CLI arguments and resolve application directories.
1. Load identity state; clear if `--force-enroll` is set.
1. Bootstrap the CA certificate (cached, file, PKI endpoint, TOFU, or system trust).
1. If already certified, check certificate expiry. If expired, clear enrollment state and re-enroll.
1. If already certified, enter the authenticated loop with reconnection. On certificate-expired TLS errors, fall back to enrollment.
1. Run enrollment with exponential backoff on disconnects.
1. Enter the authenticated loop with reconnection (exponential backoff on disconnect; immediate reconnect on certificate rotation).

Both services use a shared `ControllerConnection` type for all authenticated WebSocket communication,
which handles envelope serialization, sequence validation, and WebSocket frame processing
(Ping/Pong, Close frames).

For development details on the `ServiceHandler` trait, see [Service Lifecycle](../development/service-lifecycle.md).

## Multi-Tenancy

- Most tables are tenant-scoped (`tenant_id` required). `services`, `hosts`, `plugin_configs`, `software_items`, `settings`, and `mqtt_clients` all
  include the tenant column.
- Tables without tenant scope include `users`, `roles`, `access_grants`, `api_tokens`, and `pending_*` entities.
- `TenantContext` reads `X-Tenant-Id` or defaults to `AppState.default_tenant_id`. API handlers use it to filter data.

## System Services

System services are tenant-agnostic infrastructure components that serve all tenants simultaneously.
The MQTT bridge and external scheduler are the two current system services. They enroll through the
same WebSocket endpoint as tenant services but are routed to the `system_services` table when the
`system_service` capability is present in the enrollment payload.

See [System Services Architecture](../architecture/system-services.md) for the full design including
the credential guard, enrollment token mechanics, and the two-tier service model diagram.

### Endpoints

| Method | Path                                    | Description                                                                                                       |
| ------ | --------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| GET    | `/api/v1/system-services`               | List system services (requires `system.services:read`). Filterable by `capability` and `status`. Paginated.       |
| GET    | `/api/v1/system-services/{id}`          | Get a single system service by UUID (requires `system.services:read`).                                            |
| PUT    | `/api/v1/system-services/{id}`          | Update configurable settings: `ping_interval_seconds`, `cert_lifetime_hours` (requires `system.services:update`). |
| POST   | `/api/v1/system-services/{id}/approve`  | Approve a pending system service (requires `system.services:approve`).                                            |
| POST   | `/api/v1/system-services/{id}/reject`   | Reject a pending system service (requires `system.services:reject`).                                              |
| DELETE | `/api/v1/system-services/{id}`          | Deactivate a system service (requires `system.services:delete`).                                                  |
| GET    | `/api/v1/system-enrollment-tokens`      | List system enrollment tokens (requires `system.settings:manage`).                                                |
| POST   | `/api/v1/system-enrollment-tokens`      | Create a system enrollment token (requires `system.settings:manage`).                                             |
| GET    | `/api/v1/system-enrollment-tokens/{id}` | Get a system enrollment token (requires `system.settings:manage`).                                                |
| DELETE | `/api/v1/system-enrollment-tokens/{id}` | Revoke a system enrollment token (requires `system.settings:manage`).                                             |

### Deactivation

Deactivation follows the same transactional pattern as tenant service deactivation:

1. Soft-delete the `system_services` row.
2. Revoke all associated `system_service_certificates` rows.
3. Bump the CRL revocation version so the updated list is published without delay.

If any step fails, the entire transaction is rolled back and the service remains active.

### No merge support

System services cannot be merged. The `POST /api/v1/system-services/{target_id}/merge` endpoint
does not exist. The two revocation reasons in `system_service_certificates` are
`certificate_renewed` and `service_deactivated` only.

### Contrasting with tenant services

| Property          | Tenant services (`/api/v1/services`) | System services (`/api/v1/system-services`)  |
| ----------------- | ------------------------------------ | -------------------------------------------- |
| Scoped to tenant  | Yes                                  | No                                           |
| Enrollment token  | Per-tenant, Argon2id                 | Single global, plaintext (encrypted at rest) |
| Certificate table | `service_certificates`               | `system_service_certificates`                |
| Merge             | Supported                            | Not supported                                |
| Typical members   | Agents, SSH agents                   | MQTT bridge, external scheduler              |

## Embedded Services

When the controller runs with the `embedded-agent` or `embedded-scheduler` feature, it
auto-provisions in-process services that appear in the normal services/system-services listings.
These rows carry `is_embedded: true` in the response.

### Response fields

| Field         | Type      | Description                                                                                                                                                         |
| ------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `is_embedded` | `bool`    | `true` when the service runs inside the controller process.                                                                                                         |
| `yielded_to`  | `Uuid[]?` | List of external service IDs that caused this embedded service to yield its responsibilities. `null` or absent when not yielded. Refreshed on a 30-second interval. |

### Constraints

- **Deactivation blocked**: `DELETE /api/v1/services/{id}` and
  `DELETE /api/v1/system-services/{id}` return `409 CONFLICT` for embedded services.
  Embedded services are managed by the controller process and cannot be manually removed.
- **Merge blocked**: `POST /api/v1/services/{target_id}/merge` returns `409 CONFLICT`
  when either the target or source is an embedded service.
- **Batch operations**: Batch deactivate/delete skip embedded services with a per-item
  error in the `failed` array. Non-embedded services in the same batch are still processed.

### Merge error responses

The `POST /api/v1/services/{target_id}/merge` endpoint returns the following error codes:

| Status | Code                      | Description                                                                  |
| ------ | ------------------------- | ---------------------------------------------------------------------------- |
| 400    | `service.embedded_target` | Target service is embedded; merging into embedded services is not permitted. |
| 400    | `service.embedded_source` | Source service is embedded; embedded services cannot be merged away.         |
| 500    | `service.merge_invariant` | Redirect-chain invariant violated; service merge state is inconsistent.      |

### Yield state

An embedded service yields when an external service with the same `service_app_name`
connects. While yielded, the embedded service stops processing commands and defers to
the external service. The `yielded_to` field lists the IDs of the external services
that triggered the yield.

The yield state is stored in the `embedded_service_runtime_states` table and refreshed
every 30 seconds by the controller. It is a runtime-only indicator — not a persistent
configuration.

## Batch Actions

The `POST /api/v1/services/batch` endpoint allows performing lifecycle operations (approve,
reject, deactivate, delete) on multiple services in a single request. System services have
a corresponding `POST /api/v1/system-services/batch` endpoint.

Batch actions are also available for hosts, software items, plugin configs,
and software ignores. See [Batch Actions API](batch-actions.md) for the full reference
including request/response schema and supported actions per resource.

## Interactive Updates

When an update is triggered with `interactive: true`, the agent allocates a PTY and keeps stdin
open for bidirectional terminal I/O. The controller provides a WebSocket endpoint at
`GET /api/v1/update-history/{id}/interactive` for admin clients to send keystrokes and receive
output in real time. See [Interactive Updates API](interactive-updates.md) for the full protocol.

## Update Lifecycle Hooks

Update lifecycle hook plugins (`hook.systemd`, `hook.shell`) are standalone plugin assignments
that run before and after software updates. They inject output into `update_output` with
`[pre-hook]` and `[post-hook]` phase markers. See
[Update Lifecycle Plugins](../development/update-hooks.md).
