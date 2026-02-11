# HTTP Web API

The controller exposes a REST API under `/api/v1/`. Most endpoints are authenticated with JWT access tokens and permission checks.

## Common Patterns

- Responses use JSON envelopes with standard pagination (`limit`, `offset`, `total`).
- Rate limiting applies per-IP via the `api_rate_limits` table (see `crates/ui/web-api/src/auth/rate_limit.rs`). Rate limited endpoints return `429` with a message describing the limit window.
- Route handlers require permissions (typed `Permission` enum) obtained from the JWT; never rely on raw role strings.

## Authentication Endpoints

- `POST /api/v1/auth/device`: start a device authorization flow (RFC 8628). Returns `device_code`, `user_code`, `verification_url`, `expires_in`, `interval`.
- `POST /api/v1/auth/device/poll`: poll for approval status; returns the API token once approved.
- `POST /api/v1/auth/device/approve`: browser-side approval using Bearer token.
- `POST /api/v1/auth/token`: exchange credentials for tokens (when allowed).

Access tokens are short-lived, refresh tokens rotate on each use, and logout adds entries to the in-memory `TokenDenylist`.

## Settings Endpoints

- GET/PUT `/api/v1/settings/network`
- GET/PUT `/api/v1/settings/mqtt`, `/api/v1/settings/mqtt/{id}`
- GET/PUT `/api/v1/settings/registration`
- GET/PUT `/api/v1/settings/authentication`
- GET/PUT `/api/v1/settings/service-certificates`

Settings persist in the `settings` table and are reconciled with CLI arguments following priority rules defined in `docs/api/settings-runtime.md`. Runtime changes propagate immediately via a `tokio::sync::watch` channel (`SettingsSnapshot`).

## Services and Software Items

- `/api/v1/services`: manage agents/MQTT services (list, approve, reject, merge).
- `/api/v1/services/enrollment-token`: manage enrollment tokens for agents or MQTT services.
- `/api/v1/software-items`: CRUD endpoints for software items tied to provider configs.
- `/api/v1/update-history`: read-only history with filters by host, software item, or status.

Software items link to `provider_config`s and host associations via `host_software_item`.

## Multi-Tenancy

- Tenant-aware tables store `tenant_id` (e.g., `services`, `hosts`, `provider_configs`, `software_items`, `settings`, `mqtt_clients`).
- `TenantContext` middleware extracts `X-Tenant-Id` from the request or falls back to the default tenant (`AppState.default_tenant_id`).
- Global tables like `users`, `roles`, `permissions`, `api_tokens`, and `pending_*` remain unscoped.

## Service Operations

- `/api/v1/agents/{id}/version-check`: trigger a version check (requires `ManageAgents`).
- `/api/v1/agents/{id}/execute-update`: send `execute_update` (requires `ManageAgents`).
- `/api/v1/mqtt/tenants`: manage MQTT tenant assignments (requires `ManageSettings`).

Update history records each attempt (`status`: `pending`, `in_progress`, `completed`, `failed`) and stores the full command output for auditing.
