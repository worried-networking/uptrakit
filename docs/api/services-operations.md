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

## Multi-Tenancy

- Most tables are tenant-scoped (`tenant_id` required). `services`, `hosts`, `provider_configs`, `software_items`, `settings`, and `mqtt_clients` all include the tenant column.
- Tables without tenant scope include `users`, `roles`, `permissions`, `api_tokens`, and `pending_*` entities.
- `TenantContext` reads `X-Tenant-Id` or defaults to `AppState.default_tenant_id`. API handlers use it to filter data.

## Update Hooks

Update hooks (systemd, Docker Compose, custom commands) inject metadata in `update_output`. Document any new hook configuration in [docs/development/provider-guidelines.md](../development/provider-guidelines.md).
