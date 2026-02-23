# HTTP Web API

The controller exposes a REST API under `/api/v1/`. Most endpoints are authenticated with JWT access tokens and permission checks.

## Typed API Client

The `uptrakit-openapi-client` crate provides a typed HTTP client for all API endpoints listed below. The CLI uses
this client exclusively. See [OpenAPI Client](../development/openapi-client.md) for usage details.

Types are imported via `uptrakit_openapi_client::types::*` (re-exported from `uptrakit-web-api-types`).

## Common Patterns

- Responses use JSON envelopes with standard pagination (`limit`, `offset`, `total`).
- All entity ID fields in response types are `Uuid`, not `String`. The only exception is `SystemAlert::id`, which uses
  hardcoded string identifiers. This ensures UUID validation at the serialization boundary.
- Rate limiting applies per-IP via the `api_rate_limits` table (see `crates/ui/web-api/src/auth/rate_limit.rs`). Rate limited endpoints return `429`
  with a message describing the limit window.
- Route handlers enforce permissions via typed Axum extractors (e.g. `CanViewHosts`, `CanManageAgents`) defined in
  `crates/ui/web-api/src/middleware/permission.rs`. The required permission is declared once in the extractor and
  reflected in the OpenAPI spec via the `x-required-permission` extension on every protected endpoint.
  See [Authentication and Authorization](../security/auth-and-authorization.md) for the full permission model.

## Authentication Endpoints

- `POST /api/v1/auth/device`: start a device authorization flow (RFC 8628). Returns `device_code`, `user_code`, `verification_url`, `expires_in`,
  `interval`.
- `POST /api/v1/auth/device/poll`: poll for approval status. The `status` field is a typed `DeviceAuthStatus` enum
  (`pending`, `authorized`, `expired`) defined in `uptrakit-shared-types`. Returns the API token once status is
  `authorized`.
- `POST /api/v1/auth/device/approve`: browser-side approval using Bearer token.
- `POST /api/v1/auth/token`: exchange credentials for tokens (when allowed).

Access tokens are short-lived, refresh tokens rotate on each use, and logout adds entries to the in-memory `TokenDenylist`.

## Settings Endpoints

- GET/PUT `/api/v1/settings/network`
- GET/PUT `/api/v1/settings/mqtt`, `/api/v1/settings/mqtt/{id}`
- GET/PUT `/api/v1/settings/registration`
- GET/PUT `/api/v1/settings/authentication`
- GET/PUT `/api/v1/settings/service-certificates`

Settings persist in the `settings` table and are reconciled with CLI arguments following priority rules defined in
[docs/api/settings-runtime.md](settings-runtime.md). Runtime changes propagate immediately via a `tokio::sync::watch` channel (`SettingsSnapshot`).

## Services and Software Items

- `GET /api/v1/services`: list services with optional type/status filters.
- `GET /api/v1/services/{id}`: get a single service by UUID.
- `PUT /api/v1/services/{id}`: update a service's configurable settings. See details below.
- `POST /api/v1/services/{id}/approve`: approve a pending service.
- `POST /api/v1/services/{id}/reject`: reject a pending service.
- `DELETE /api/v1/services/{id}`: deactivate (soft-delete) a service.
- `POST /api/v1/services/{target_id}/merge`: merge a source into a target.
- `/api/v1/services/enrollment-token`: manage enrollment tokens for agents or MQTT services.
- `/api/v1/software-items`: CRUD endpoints for software items tied to provider configs.
- `POST /api/v1/software-items/{id}/approve`: approve a discovered (pending) software item. Requires `manage_software`.
- `/api/v1/update-history`: read-only history with filters by host, software item, or status.
- `POST /api/v1/hosts/{id}/discover`: trigger software discovery on a specific host. Requires `manage_software`.
- `DELETE /api/v1/hosts/{id}/discovered[?provider_config_id={uuid}]`: bulk-discard pending discovered items for a host. Requires `manage_software`.
- `POST /api/v1/provider-configs/{id}/discover`: trigger discovery for a specific provider config. Requires `manage_software`.
- `DELETE /api/v1/provider-configs/{id}/discovered`: bulk-discard pending discovered items for a provider config. Requires `manage_software`.
- `/api/v1/autodiscovery/ignores`: CRUD for permanent suppression rules. See [docs/api/autodiscovery.md](autodiscovery.md) for full details.

Software items link to `provider_config`s and host associations via `host_software_item`.

`ServiceResponse` includes an optional `ping_interval_seconds` field (`Option<u32>`) that reports the per-service
ping interval override. When `null`, the service uses its type default (300s for agents/SSH agents, 15s for MQTT).

### `PUT /api/v1/services/{id}`

Update a service's configurable settings. Requires the `ManageAgents` permission.

**Path parameters**: `id` -- service UUID.

**Request body** (`UpdateServiceRequest`):

```json
{
  "ping_interval_seconds": 60
}
```

- `ping_interval_seconds`: optional `u32`. Omit the field (or set to `null`) to keep the current value. Set to `0`
  to clear the override and revert to the service-type default. Set to a positive value to override the default ping
  interval in seconds.

**Response** (`200`): `ServiceResponse`

```json
{
  "id": "019...",
  "service_type": "agent",
  "hostname": "host-1.local",
  "friendly_name": "My Agent",
  "ip_address": "10.0.0.1",
  "status": "approved",
  "client_version": "1.2.3",
  "last_seen_at": "2026-02-15T12:00:00Z",
  "created_at": "2026-01-01T00:00:00Z",
  "updated_at": "2026-02-15T12:00:00Z",
  "ping_interval_seconds": 60
}
```

**Error responses**:

- `404` -- service not found.

### Response types

Types are defined in `crates/shared/web-api-types/src/services.rs`:

| Type | Fields |
| --- | --- |
| `UpdateServiceRequest` | `ping_interval_seconds` (`Option<u32>`) |
| `ServiceResponse` | `id`, `service_type`, `hostname`, `friendly_name`, `ip_address`, `status`, `client_version`, `last_seen_at`, `created_at`, `updated_at`, `ping_interval_seconds` |

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/services.rs` | Route handler (`update_service`) |
| `crates/shared/web-api-types/src/services.rs` | Request/response types |

## Multi-Tenancy

- Tenant-aware tables store `tenant_id` (e.g., `services`, `hosts`, `provider_configs`, `software_items`, `settings`, `mqtt_clients`).
- `TenantContext` middleware extracts `X-Tenant-Id` from the request or falls back to the default tenant (`AppState.default_tenant_id`).
- Global tables like `users`, `roles`, `permissions`, `api_tokens`, and `pending_*` remain unscoped.

## Service Operations

- `/api/v1/agents/{id}/version-check`: trigger a version check (requires `ManageSoftware`).
- `/api/v1/agents/{id}/execute-update`: send `execute_update` (requires `ManageSoftware`).
- `/api/v1/mqtt/tenants`: manage MQTT tenant assignments (requires `ManageSettings`).

Update history records each attempt (`status`: `pending`, `in_progress`, `completed`, `failed`) and stores the full command output for auditing.

## Software Item Version Check Endpoints

These endpoints trigger granular per-item version checks. Both require the `ManageSoftware` permission.

### `POST /api/v1/software-items/{id}/check-versions`

Trigger a version check for a specific software item across all assigned hosts. The controller identifies all
hosts linked to the item, resolves their agents, and sends `CheckVersions` wire messages.

**Path parameters**: `id` — software item UUID.

**Response** (`200`): `TriggerVersionCheckResponse`

```json
{
  "agents_notified": 3,
  "message": "Version check triggered for 3 agent(s)"
}
```

**Error responses**:

- `404` — software item not found or not active.
- `404` — no hosts assigned to the software item.

### `POST /api/v1/software-items/{id}/hosts/{host_id}/check-versions`

Trigger a version check for a specific software item on a specific host. Validates the item-host link and sends
a `CheckVersions` message to the host's agent.

**Path parameters**: `id` — software item UUID, `host_id` — host UUID.

**Response** (`200`): `TriggerVersionCheckResponse`

```json
{
  "agents_notified": 1,
  "message": "Version check triggered for 1 agent(s)"
}
```

**Error responses**:

- `404` — software item not found or not active.
- `404` — host not found.
- `404` — host is not assigned to this software item.
- `404` — no agent found for this host.

### Response types

Types are defined in `crates/shared/web-api-types/src/software_items.rs`:

| Type | Fields |
| --- | --- |
| `TriggerVersionCheckResponse` | `agents_notified` (u32), `message` (String) |

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/software_items.rs` | Route handlers (`check_versions`, `check_versions_host`) |
| `crates/shared/web-api-types/src/software_items.rs` | Response type |

## Scheduler Endpoints

All scheduler endpoints require the `ManageSoftware` permission.

### `GET /api/v1/scheduler/tasks`

List all scheduled tasks for the tenant.

**Response** (`200`): `Vec<ScheduledTaskResponse>`

```json
[
  {
    "id": "019...",
    "task_type": "auth_cleanup",
    "label": "Auth Cleanup",
    "cron_expression": "*/5 * * * *",
    "enabled": true,
    "task_config": null,
    "last_run_at": "2026-02-15T12:00:00Z",
    "next_run_at": "2026-02-15T12:05:00Z",
    "is_running": false,
    "last_error": null,
    "run_count": 42,
    "created_at": "2026-02-15T00:00:00Z",
    "updated_at": "2026-02-15T12:00:00Z"
  }
]
```

### `GET /api/v1/scheduler/tasks/{id}`

Get a single scheduled task by UUID.

**Response** (`200`): `ScheduledTaskResponse`

### `PUT /api/v1/scheduler/tasks/{id}`

Update a scheduled task. All fields are optional.

**Request body** (`UpdateScheduledTaskRequest`):

```json
{
  "cron_expression": "0 */12 * * *",
  "enabled": true,
  "task_config": { "custom_key": "value" }
}
```

- `cron_expression`: validated before persistence. Standard 5-field or extended 6/7-field cron. Updating the expression recomputes `next_run_at`.
- `enabled`: toggle task on/off.
- `task_config`: JSON value. Send `null` to clear.

**Response** (`200`): `ScheduledTaskResponse`

### `POST /api/v1/scheduler/tasks/{id}/trigger`

Trigger immediate execution. Sets `next_run_at = now` so the task becomes eligible on the next scheduler poll cycle.

**Response** (`200`): `TriggerScheduledTaskResponse`

```json
{
  "triggered": true,
  "message": "Task will execute on next scheduler poll"
}
```

### Response types

Types are defined in `crates/shared/web-api-types/src/scheduler.rs`:

| Type | Fields |
| --- | --- |
| `ScheduledTaskResponse` | `id`, `task_type`, `label`, `cron_expression`, `enabled`, `task_config`, `last_run_at`, `next_run_at`, `is_running`, `last_error`, `run_count`, `created_at`, `updated_at` |
| `UpdateScheduledTaskRequest` | `cron_expression?`, `enabled?`, `task_config?` |
| `TriggerScheduledTaskResponse` | `triggered`, `message` |

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/scheduler.rs` | Route handlers |
| `crates/shared/web-api-types/src/scheduler.rs` | Request/response types |
| `crates/shared/db/src/entity/scheduled_task.rs` | SeaORM entity |

## API Error Responses - Detailed

All web API error responses use a consistent JSON format defined by `ErrorResponse`
(`crates/shared/web-api-types/src/error.rs`):

```json
{
  "error": "Human-readable error message",
  "code": "optional_machine_readable_code"
}
```

The `code` field is optional and serializes as `null` when not set. It is used where a machine-readable code adds
value (e.g. `"not_found"` for the 404 fallback, `"agent_version_too_old"` for version checks).

### Helper functions

`crates/ui/web-api/src/error_response.rs` provides two helpers:

- `error_response(status, message) -> Response` — most common case, no code field
- `error_response_with_code(status, message, code) -> Response` — includes a machine-readable code

All route handlers, middleware rejections, and custom `IntoResponse` impls use these helpers instead of constructing raw
tuples.

### Convention

- **Do**: `error_response(StatusCode::BAD_REQUEST, "Invalid input")`
- **Do not**: `(StatusCode::BAD_REQUEST, "Invalid input").into_response()`
- **Do not**: construct `Json(serde_json::json!({"error": "..."}))` manually
- The 404 fallback uses `error_response_with_code` for the JSON path; the HTML path returns a plain-text "Not Found".
- WebSocket endpoints (`service_ws.rs`) are excluded — they use protocol-level error handling, not JSON responses.

### Frontend integration

The frontend (`frontend/src/lib/api.ts`) uses `extractErrorMessage(res)` to parse error responses. It tries to parse
JSON and extract the `error` field; falls back to the raw response text if parsing fails. The `ErrorResponse` TypeScript
interface is defined in `frontend/src/lib/types.ts`.

## API Rate Limiting - Detailed

Database-backed per-IP rate limiting protects public authentication endpoints from brute-force attacks, credential
stuffing, and abuse. All rate limit state is in the `api_rate_limits` table, making it HA-safe across multiple
controller instances.

### Rate-limited endpoints

| Endpoint | Limit | Key format |
| --- | --- | --- |
| `POST /api/v1/auth/login` | 10 req/min/IP | `/api/v1/auth/login:{ip}` |
| `POST /api/v1/auth/register` | 10 req/min/IP | `/api/v1/auth/register:{ip}` |
| `POST /api/v1/auth/refresh` | 10 req/min/IP | `/api/v1/auth/refresh:{ip}` |
| `POST /api/v1/auth/device` | 10 req/min/IP | `/api/v1/auth/device:{ip}` |
| `POST /api/v1/auth/device/poll` | 12 req/min/IP | `/api/v1/auth/device/poll:{ip}` |

Endpoints **not** rate-limited: logout (requires valid refresh token), device/approve (requires auth), OIDC (external
IdP interaction), all authenticated endpoints (require valid JWT/API token).

### WebSocket rate limiting

The `/api/v1/ws/service` WebSocket endpoint has its own per-IP rate limiting, applied **before** the WebSocket upgrade:

| Key format | Limit | Trigger | Fail mode |
| --- | --- | --- | --- |
| `ws_connect:{ip}` | 30 req/60s | Every connection attempt | Fail-closed (503 on DB error) |
| `ws_auth_fail:{ip}` | 10 req/300s | After failed bearer lookup | Fail-closed (503 on DB error) |

Unlike the HTTP rate limiter middleware (which fails open), the WebSocket rate limiter **fails closed** on DB errors.
This prevents bypass under database pressure. The check is in `service_ws()` in
`crates/ui/web-api/src/routes/service_ws.rs`.

### Implementation

- **Store**: `crates/ui/web-api/src/auth/rate_limit.rs` — `RateLimitStore` with sliding-window counter algorithm using
  atomic upserts.
- **Middleware**: `crates/ui/web-api/src/middleware/rate_limit.rs` — `rate_limit_auth` middleware with
  `LazyLock<HashMap>` endpoint config. Fails open on store errors.
- **Entity**: `crates/shared/db/src/entity/api_rate_limit.rs` — SeaORM entity for the `api_rate_limits` table (columns:
  `key` TEXT PK, `request_count` INTEGER, `window_start` TIMESTAMP, `expires_at` TIMESTAMP).
- **Migration**: `crates/core/controller/src/migration/m20260209_000001_initial.rs`.
- **Cleanup**: expired entries are pruned every 5 minutes by the controller's periodic cleanup task.

### Response format

When rate-limited, the API returns HTTP 429 with a JSON `ErrorResponse` body and a `Retry-After` header:

```json
{ "error": "Too many requests, please try again later" }
```

### Adding a new rate-limited endpoint

1. Add an entry to the `RATE_LIMITS` HashMap in `crates/ui/web-api/src/middleware/rate_limit.rs`.
1. Update the table in this section.

## Pagination - Detailed

All list endpoints that can grow unboundedly use server-side pagination with a consistent response envelope. Shared
types are defined in `crates/shared/web-api-types/src/pagination.rs`.

### Query parameters

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `page` | u64 | 1 | Page number (1-indexed) |
| `per_page` | u64 | 20 | Items per page (clamped to 1–1000) |

### Response envelope

All paginated endpoints return a `PaginatedResponse<T>`:

```json
{
  "items": [...],
  "total": 42,
  "page": 1,
  "per_page": 20,
  "total_pages": 3
}
```

### Paginated endpoints

| Endpoint | Query struct | Notes |
| --- | --- | --- |
| `GET /api/v1/services` | `ListServicesQuery` (includes `page`/`per_page`) | Filterable by `type`, `status` |
| `GET /api/v1/hosts` | `PaginationParams` | |
| `GET /api/v1/software-items` | `PaginationParams` | |
| `GET /api/v1/update-history` | `UpdateHistoryQuery` (includes `page`/`per_page`) | Filterable by `host_id`, `software_item_id`, `status` |
| `GET /api/v1/provider-configs` | `PaginationParams` | |

### Endpoints NOT paginated (already bounded)

| Endpoint | Reason |
| --- | --- |
| `GET /api/v1/settings/mqtt` | Bounded by `MqttMaxClientsPerTenant` (default 10) |
| `GET /api/v1/auth/api-tokens` | Per-user, typically small |

### Adding pagination to a new endpoint

For endpoints with an existing query struct, add `page: Option<u64>` and `per_page: Option<u64>` fields directly
(serde_urlencoded does not support `#[serde(flatten)]`) and provide a `pagination()` helper method. For endpoints
without an existing query struct, use `Query<PaginationParams>` as a new extractor.

### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/web-api-types/src/pagination.rs` | `PaginationParams`, `ResolvedPagination`, `PaginatedResponse<T>` |
| `crates/shared/web-api-types/src/prelude.rs` | Convenience re-exports of ~35 commonly used request/response types (`use uptrakit_web_api_types::prelude::*`) |
