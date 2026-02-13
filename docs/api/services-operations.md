# Services and Operations

## Agents

- `/api/v1/services` lists all agents and MQTT services (approve, reject, delete, merge).
- `/api/v1/agents/{agent_id}/version-check` instructs the controller to send `check_versions` over WebSocket.
- `/api/v1/agents/{agent_id}/execute-update` triggers `execute_update` with the software item ID(s).
- `/api/v1/update-history` provides audit logs for updates; each row tracks `status`, `output`, `initiated_by`, and `tenant_id`.

## MQTT Service

- `/api/v1/services/enrollment-token?type=mqtt` issues tokens for MQTT instances.
- MQTT clients receive `tenant_assignments`, `tenant_config_updated`, and `tenant_revoked` commands after assignment.
- MQTT services share the same enrollment, certificate, and PKI flows as agents.
- Agent and MQTT services use the same activity tracking fields in `services`: `ip_address` is refreshed on each WebSocket connect, and `last_seen_at`
  is refreshed on connect and heartbeat (`ping`).

## Shared Service Startup Flow

Both agents and MQTT services follow a unified startup sequence provided by the `uptrakit-service-sdk` crate:

1. Initialize tracing with a crate-specific directive (e.g. `uptrakit_agent=info`).
1. Install the `aws-lc-rs` crypto provider for rustls.
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

## Multi-Tenancy

- Most tables are tenant-scoped (`tenant_id` required). `services`, `hosts`, `provider_configs`, `software_items`, `settings`, and `mqtt_clients` all
  include the tenant column.
- Tables without tenant scope include `users`, `roles`, `permissions`, `api_tokens`, and `pending_*` entities.
- `TenantContext` reads `X-Tenant-Id` or defaults to `AppState.default_tenant_id`. API handlers use it to filter data.

## Update Hooks

Update hooks (systemd, Docker Compose, custom commands) inject metadata in `update_output`. Document any new hook configuration in
[docs/development/provider-guidelines.md](../development/provider-guidelines.md).
