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

| Field | Type | Description |
| --- | --- | --- |
| `ping_interval_seconds` | `u32` (optional) | Custom ping interval in seconds. `0` clears the override; minimum positive value is `5`. Omit to keep current value. |
| `cert_lifetime_hours` | `u32` (optional) | Per-service certificate lifetime in hours. `0` clears the per-service override and reverts to the global default. Valid positive range: `1`–`17520`. Omit to keep current value. |

When `cert_lifetime_hours` is set, it takes precedence over the global agent certificate lifetime setting
(`PUT /api/v1/settings/agent-certificates`) at certificate signing time — both during initial enrollment
and on renewals.

The `ServiceResponse` includes `cert_lifetime_hours` only when a per-service override is active (the field
is omitted from JSON when the service uses the global default).

See also: [PKI and Certificate Lifecycle](../security/pki-certificates.md) for renewal window details and
the per-service override section.

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
- Tables without tenant scope include `users`, `roles`, `permissions`, `api_tokens`, and `pending_*` entities.
- `TenantContext` reads `X-Tenant-Id` or defaults to `AppState.default_tenant_id`. API handlers use it to filter data.

## System Services

System services are tenant-agnostic infrastructure components that serve all tenants simultaneously.
The MQTT bridge and external scheduler are the two current system services. They enroll through the
same WebSocket endpoint as tenant services but are routed to the `system_services` table when the
`system_service` capability is present in the enrollment payload.

See [System Services Architecture](../architecture/system-services.md) for the full design including
the credential guard, enrollment token mechanics, and the two-tier service model diagram.

### Endpoints

| Method | Path | Description |
| --- | --- | --- |
| GET | `/api/v1/system-services` | List system services (requires `view_system_services`). Filterable by `capability` and `status`. Paginated. |
| GET | `/api/v1/system-services/{id}` | Get a single system service by UUID (requires `view_system_services`). |
| PUT | `/api/v1/system-services/{id}` | Update configurable settings: `ping_interval_seconds`, `cert_lifetime_hours` (requires `manage_system_services`). |
| POST | `/api/v1/system-services/{id}/approve` | Approve a pending system service (requires `manage_system_services`). |
| POST | `/api/v1/system-services/{id}/reject` | Reject a pending system service (requires `manage_system_services`). |
| DELETE | `/api/v1/system-services/{id}` | Deactivate a system service (requires `manage_system_services`). |
| GET | `/api/v1/settings/system-services` | Get the global enrollment token (requires `manage_system_services`). |
| PUT | `/api/v1/settings/system-services` | Set or clear the global enrollment token (requires `manage_system_services`). |

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

| Property | Tenant services (`/api/v1/services`) | System services (`/api/v1/system-services`) |
| --- | --- | --- |
| Scoped to tenant | Yes | No |
| Enrollment token | Per-tenant, Argon2id | Single global, plaintext (encrypted at rest) |
| Certificate table | `service_certificates` | `system_service_certificates` |
| Merge | Supported | Not supported |
| Typical members | Agents, SSH agents | MQTT bridge, external scheduler |

## Update Hooks

Update hooks (systemd, Docker Compose, custom commands) inject metadata in `update_output`. Document any new hook configuration in
[docs/development/plugin-guidelines.md](../development/plugin-guidelines.md).
