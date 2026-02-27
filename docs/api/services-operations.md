# Services and Operations

## Agents

- `/api/v1/services` lists all agents and MQTT services.
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

## MQTT Service

- `/api/v1/enrollment-tokens` manages multiple named enrollment tokens with optional capability scoping,
  usage limits, and TTL. See [Enrollment Tokens API](enrollment-tokens.md) for full details.
- MQTT clients receive `tenant_assignments`, `tenant_config_updated`, and `tenant_revoked` commands after assignment.
- MQTT services share the same enrollment, certificate, and PKI flows as agents.
- Agent and MQTT services use the same activity tracking fields in `services`: `ip_address` is refreshed on each WebSocket connect, and `last_seen_at`
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

## Update Hooks

Update hooks (systemd, Docker Compose, custom commands) inject metadata in `update_output`. Document any new hook configuration in
[docs/development/plugin-guidelines.md](../development/plugin-guidelines.md).
