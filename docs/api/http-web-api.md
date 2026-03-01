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
- GET/PUT `/api/v1/settings/smtp` — global SMTP settings (requires `CanManageGlobalSettings`)
- GET/PUT `/api/v1/settings/nats` *(feature: `nats`)* — NATS server URL (requires `CanManageGlobalSettings`).
  The URL is stored encrypted at rest. The response returns the masked URL with password replaced by `***`.
  Changes take effect after a controller restart (hot-reload not supported). See
  [Settings Runtime — NATS settings](settings-runtime.md#nats-settings-feature-nats) for full details.

Settings persist in the `settings` table and are reconciled with CLI arguments following priority rules defined in
[docs/api/settings-runtime.md](settings-runtime.md). Runtime changes propagate immediately via a `tokio::sync::watch` channel (`SettingsSnapshot`).

## Services and Software Items

- `GET /api/v1/services`: list services with optional capability/status filters.
- `GET /api/v1/services/{id}`: get a single service by UUID.
- `PUT /api/v1/services/{id}`: update a service's configurable settings. See details below.
- `POST /api/v1/services/{id}/approve`: approve a pending service.
- `POST /api/v1/services/{id}/reject`: reject a pending service.
- `DELETE /api/v1/services/{id}`: deactivate (soft-delete) a service.
- `POST /api/v1/services/{target_id}/merge`: merge a source into a target.
- `/api/v1/enrollment-tokens`: CRUD endpoints for enrollment tokens (create, list, get, revoke).
  See [Enrollment Tokens API](enrollment-tokens.md) for full details.
- `/api/v1/software-items`: CRUD endpoints for software items. A software item is a named catalog
  entry; plugin configs and package identifiers live on role-specific plugin assignments
  (`host_software_item_plugins`), not on the item itself.
- `POST /api/v1/software-items/{id}/approve`: approve a discovered (pending) software item.
  Requires `manage_software`.
- `POST /api/v1/software-items/{id}/hosts`: assign a software item to one or more hosts. Each host
  assignment carries a list of role-specific plugin assignments (`plugins: Vec<HostPluginRoleAssignment>`),
  where each role entry specifies the `role`, `plugin_config_id` (or inline `plugin_config`),
  `package_identifier`, optional `config_override`, and `execution_site`. Requires `manage_software`.
- `PUT /api/v1/software-items/{id}/hosts/{host_id}`: update a specific role assignment for a host --
  change the plugin config, package identifier, config override, or execution site. The request body
  includes `role` to identify which role to update. Requires `manage_software`.
- `DELETE /api/v1/software-items/{id}/hosts/{host_id}[?ignore=true]`: remove a host assignment.
  Pass `?ignore=true` to also create an autodiscovery ignore rule for the assignment's
  `(plugin_config_id, package_identifier)`. Requires `manage_software`.
- `/api/v1/update-history`: read-only history with filters by host, software item, or status.
- `POST /api/v1/hosts/{id}/discover`: trigger software discovery on a specific host. Requires `manage_software`.
- `DELETE /api/v1/hosts/{id}/discovered[?plugin_config_id={uuid}]`: bulk-discard pending discovered items for a host. Requires `manage_software`.
- `POST /api/v1/plugin-configs/{id}/discover`: trigger discovery for a specific plugin config. Requires `manage_software`.
- `DELETE /api/v1/plugin-configs/{id}/discovered`: bulk-discard pending discovered items for a plugin config. Requires `manage_software`.

`PluginConfigResponse` includes a `capabilities: Vec<String>` field listing the snake\_case capability strings
declared by the plugin type (e.g. `["discover_local_software"]`). Clients should use this field to determine
which actions are valid for a given config — for example, only showing **Discover** and **Discard** buttons
when `"discover_local_software"` is present. Discovery-capable plugin types are `releases_docker`,
`package_manager_homebrew`, `package_manager_apt`, and `discovery_proxmox_helper_scripts`; non-discovery types
(`releases_github`, `generic_shell`) return an empty capabilities list for this field.

- `/api/v1/autodiscovery/ignores`: CRUD for permanent suppression rules. See [docs/api/autodiscovery.md](autodiscovery.md) for full details.
- `/api/v1/discovery-allowlist`: tenant-wide list of plugin types permitted to run during host
  discovery. `GET` requires `view_software`; `POST`/`DELETE` require `manage_software`.
- `/api/v1/hosts/{id}/discovery-allowlist`: per-host override of the tenant-wide allowlist.
  Same permission requirements. When a host has its own entries the tenant-wide list is ignored
  entirely for that host. When neither list has entries all discovery-capable plugins run (default).
  See [Discovery Allowlist API](discovery-allowlist.md) for full details.

`ServiceResponse` includes an optional `ping_interval_seconds` field (`Option<u32>`) that reports the per-service
ping interval override. When `null`, the service uses its profile default (300s for agents/SSH agents, 15s for MQTT).

`ServiceResponse` uses `capabilities: Vec<String>` and `service_label: String` instead of the former `service_type`
field. The `service_label` is a human-readable display name derived from the service's capability set via
`ServiceProfile` (e.g. "Agent", "SSH Agent", "MQTT Bridge").

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
  to clear the override and revert to the service-profile default. Set to a positive value to override the default ping
  interval in seconds.

**Response** (`200`): `ServiceResponse`

```json
{
  "id": "019...",
  "capabilities": ["software_discovery", "update_hooks", "graceful_shutdown"],
  "service_label": "Agent",
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
| `ServiceResponse` | `id`, `capabilities`, `service_label`, `hostname`, `friendly_name`, `ip_address`, `status`, `client_version`, `last_seen_at`, `created_at`, `updated_at`, `ping_interval_seconds` |

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/services.rs` | Route handler (`update_service`) |
| `crates/shared/web-api-types/src/services.rs` | Request/response types |

## Multi-Tenancy

- Tenant-aware tables store `tenant_id` (e.g., `services`, `hosts`, `plugin_configs`, `software_items`, `settings`, `mqtt_clients`).
- `TenantContext` middleware extracts `X-Tenant-Id` from the request or falls back to the default tenant (`AppState.default_tenant_id`).
- Global tables like `users`, `roles`, `permissions`, `api_tokens`, and `pending_*` remain unscoped.

## Update Output Streaming (SSE)

`GET /api/v1/update-history/{id}/output/stream` — Server-Sent Events (SSE) endpoint for real-time update
output streaming. Requires the `ViewSoftware` permission (same as the update history endpoints). Supports
both session cookie and API token (Bearer) authentication.

This endpoint is a standard HTTP streaming endpoint using the `text/event-stream` content type — it is
**not** part of the WebSocket wire protocol between services and the controller.

### Request

```http
GET /api/v1/update-history/{id}/output/stream HTTP/1.1
Accept: text/event-stream
Authorization: Bearer <token>
```

### Event format

Two event types are emitted:

**`output`** — a single line of update output:

```text
event: output
data: {"id":"<uuid>","text":"Installing package...\n","stream":"stdout","timestamp":"2026-02-27T12:00:00Z","seq":0}
```

**`completed`** — the update has finished (stream ends after this event):

```text
event: completed
data: {"status":"completed","error":null}
```

### Behavior

1. Subscribes to the in-process broadcast channel **before** loading stored lines (avoids gaps).
2. Replays all existing `update_output_line` rows from the database (ordered by creation time).
3. If the update is already completed/failed: replays stored output, sends a `completed` event, and closes.
4. If the update is in-progress: replays stored output, then streams new lines in real time.
5. Sequence-based deduplication prevents replayed lines from being sent twice.
6. A 15-second keep-alive comment (`: keep-alive`) prevents proxies from closing idle connections.

### Response types

Types are defined in `crates/shared/web-api-types/src/update_history.rs`:

| Type | Fields |
| --- | --- |
| `OutputLineSSE` | `id` (Uuid), `text` (String), `stream` (String), `timestamp` (OffsetDateTime), `seq` (u64) |
| `UpdateCompletedSSE` | `status` (String), `error` (Option&lt;String&gt;) |

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/update_history.rs` | SSE handler (`stream_update_output`) |
| `crates/ui/web-api/src/update_output_broadcaster.rs` | In-process broadcast registry (`UpdateOutputBroadcaster`) |
| `crates/shared/web-api-types/src/update_history.rs` | SSE event types (`OutputLineSSE`, `UpdateCompletedSSE`) |

## Service Operations

- `/api/v1/agents/{id}/version-check`: trigger a version check (requires `ManageSoftware`).
- `/api/v1/agents/{id}/execute-update`: send `execute_update` (requires `ManageSoftware`).
- `/api/v1/mqtt/tenants`: manage MQTT tenant assignments (requires `ManageSettings`).

Update history records each attempt (`status`: `pending`, `in_progress`, `completed`, `failed`) and stores the full command output for auditing.

## Software Item Version Check Endpoints

These endpoints trigger granular per-item version checks. Both require the `ManageSoftware` permission.

### `POST /api/v1/software-items/{id}/check-versions`

Trigger a version check for a specific software item across all assigned hosts. The controller resolves the
`execution_site` for each host's plugin assignments and routes work accordingly:

- **Controller-side** (`execution_site = "controller"`, or `"auto"` with `ControllerSideFetchReleases` capability
  — e.g. GitHub Releases, Docker Registry): `fetch_releases` runs directly on the controller. The
  `host_software_items` table is updated immediately and MQTT states are pushed to connected clients.
- **Agent-side** (`execution_site = "agent"`, or `"auto"` without the controller capability): a `CheckVersions`
  wire message is sent to the host's agent.

**Path parameters**: `id` — software item UUID.

**Response** (`200`): `TriggerVersionCheckResponse`

```json
{
  "agents_notified": 2,
  "controller_checks_run": 1,
  "message": "Version check triggered for 2 agent(s), 1 controller-side check(s) run"
}
```

**Error responses**:

- `404` — software item not found or not active.
- `404` — no hosts assigned to the software item, or no applicable plugin assignments found.

### `POST /api/v1/software-items/{id}/hosts/{host_id}/check-versions`

Trigger a version check for a specific software item on a specific host. Resolves `execution_site` for all
plugin assignments on the host and routes work the same way as the bulk endpoint above.

**Path parameters**: `id` — software item UUID, `host_id` — host UUID.

**Response** (`200`): `TriggerVersionCheckResponse`

```json
{
  "agents_notified": 0,
  "controller_checks_run": 1,
  "message": "Controller-side check completed"
}
```

**Error responses**:

- `404` — software item not found or not active.
- `404` — host not found.
- `404` — host is not assigned to this software item.
- `404` — no agent found for host and no controller-side plugins are configured.

### Response types

Types are defined in `crates/shared/web-api-types/src/software_items.rs`:

| Type | Fields |
| --- | --- |
| `TriggerVersionCheckResponse` | `agents_notified` (u32), `controller_checks_run` (u32, default `0`), `message` (String) |
| `SoftwareItemResponse` | `id`, `name`, `plugins` (Vec&lt;String&gt; -- distinct plugin types), `enabled`, `discovery_state`, `last_checked_at`, `host_count`, `latest_version` (Option), `update_available`, `created_at`, `updated_at` |
| `SoftwareItemDetailResponse` | Extends `SoftwareItemResponse` with `hosts: Vec<SoftwareItemHostSummary>` |
| `SoftwareItemHostSummary` | `host_id`, `hostname`, `friendly_name`, `plugins` (Vec&lt;HostPluginRoleSummary&gt;), `installed_version`, `installed_version_detected_at`, `latest_version` (Option), `latest_release_metadata` (Option), `update_available`, `last_updated_at`, `linked_at` |
| `HostPluginRoleSummary` | `role` (PluginRole), `plugin_config_id`, `plugin_config_name`, `plugin_type`, `package_identifier`, `config_override` (Option), `execution_site` |
| `HostSoftwareAssignment` | `host_id`, `plugins` (Vec&lt;HostPluginRoleAssignment&gt;) |
| `HostPluginRoleAssignment` | `role` (PluginRole), `plugin_config_id` (Option), `plugin_config` (Option -- inline create), `package_identifier`, `config_override` (Option), `execution_site` (default `"auto"`) |
| `UpdateHostAssignmentRequest` | `role` (PluginRole), `plugin_config_id` (Option), `plugin_config` (Option), `package_identifier` (Option), `config_override` (Option), `execution_site` (Option) |
| `TriggerUpdateRequest` | `to_version` (String), `release_info` (Option -- `{ tag, release_url, assets }`) |
| `TriggerUpdateResponse` | `update_history_id` (Uuid), `status` (TriggerUpdateStatus -- `pending`, `queued`) |

**`latest_version` and `update_available`** are populated by the controller at read time:

- `latest_version` is stored per-host on `host_software_items.latest_version`. It is populated by:
  - Agent-side `fetch_releases` plugins (Homebrew, APT) via the `VersionCheckResults` WebSocket handler.
  - The controller scheduler's Phase A for controller-side plugins (GitHub Releases, Docker Registry).
  - The manual `check-versions` endpoints when a plugin's `execution_site` routes to the controller
    (either explicitly `"controller"`, or `"auto"` with `ControllerSideFetchReleases` capability).

  `null` when no upstream version has been resolved yet.
- At the item level (`SoftwareItemResponse`), `latest_version` is derived as the maximum across all
  hosts' per-host `latest_version` values.
- `update_available` at the item level is `true` if any assigned host has an `installed_version`
  that differs from its per-host `latest_version` (and both are known; string equality). At the
  host level (`SoftwareItemHostSummary`), it is `true` when both `installed_version` and
  `latest_version` are non-null and differ.
- `last_checked_at` on `SoftwareItemResponse` is updated in a single batch `UPDATE` after the
  `VersionCheckResults` WebSocket message is processed, covering all items that had at least one
  successful result.

See [Software Item Entity](../architecture/software-item-entity.md) for the full field reference.

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/software_items.rs` | Route handlers (`check_versions`, `check_versions_host`, `trigger_update`) |
| `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` | `VersionCheckResults` handler (updates `last_checked_at`) |
| `crates/shared/web-api-types/src/software_items.rs` | Response and request types |

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

## Notification Endpoints

Full CRUD for notification channels, rules, and a delivery log. See [Notifications API](notifications.md) for
the complete endpoint reference with request/response examples.

- `POST /api/v1/notifications/channels`: create a channel (requires `ManageNotifications`).
- `GET /api/v1/notifications/channels`: list channels, paginated (requires `ViewNotifications`).
- `GET /api/v1/notifications/channels/{id}`: get a channel (requires `ViewNotifications`).
- `PUT /api/v1/notifications/channels/{id}`: update a channel (requires `ManageNotifications`).
- `DELETE /api/v1/notifications/channels/{id}`: delete a channel (requires `ManageNotifications`).
- `POST /api/v1/notifications/channels/{id}/test`: send a test notification (requires `ManageNotifications`).
- `POST /api/v1/notifications/rules`: create a rule (requires `ManageNotifications`).
- `GET /api/v1/notifications/rules`: list rules, paginated and filterable (requires `ViewNotifications`).
- `GET /api/v1/notifications/rules/{id}`: get a rule (requires `ViewNotifications`).
- `PUT /api/v1/notifications/rules/{id}`: update a rule (requires `ManageNotifications`).
- `DELETE /api/v1/notifications/rules/{id}`: delete a rule (requires `ManageNotifications`).
- `GET /api/v1/notifications/log`: list delivery log, paginated (requires `ViewNotifications`).
- `POST /api/v1/notifications/callback/telegram/{channel_id}`: Telegram callback (public,
  verified via `X-Telegram-Bot-Api-Secret-Token` header).

Channel types: `webhook` (always available), `telegram` (feature-gated).

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
- WebSocket endpoints (`service_ws/`) are excluded — they use protocol-level error handling, not JSON responses.

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
`crates/ui/web-api/src/routes/service_ws/mod.rs`.

### Implementation

- **Store**: `crates/ui/web-api/src/auth/rate_limit.rs` — `RateLimitStore` with sliding-window counter algorithm using
  atomic upserts.
- **Middleware**: `crates/ui/web-api/src/middleware/rate_limit.rs` — `rate_limit_auth` middleware with
  `LazyLock<HashMap>` endpoint config. Fails open on store errors.
- **Entity**: `crates/shared/db/src/entity/api_rate_limit.rs` — SeaORM entity for the `api_rate_limits` table (columns:
  `key` TEXT PK, `request_count` INTEGER, `window_start` TIMESTAMP, `expires_at` TIMESTAMP).
- **Migration**: `crates/shared/db/src/migration/m20260209_000001_initial.rs`.
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
| `GET /api/v1/services` | `ListServicesQuery` (includes `page`/`per_page`) | Filterable by `capability`, `status` |
| `GET /api/v1/hosts` | `PaginationParams` | |
| `GET /api/v1/software-items` | `PaginationParams` | |
| `GET /api/v1/update-history` | `UpdateHistoryQuery` (includes `page`/`per_page`) | Filterable by `host_id`, `software_item_id`, `status` |
| `GET /api/v1/plugin-configs` | `PaginationParams` | |
| `GET /api/v1/enrollment-tokens` | `ListEnrollmentTokensQuery` (includes `page`/`per_page`) | |
| `GET /api/v1/notifications/channels` | `PaginationParams` | |
| `GET /api/v1/notifications/rules` | `ListRulesQuery` (includes `page`/`per_page`) | Filterable by `channel_id`, `event_type` |
| `GET /api/v1/notifications/log` | `PaginationParams` | |

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
