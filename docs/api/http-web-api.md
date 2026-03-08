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
- Rate limiting applies per-IP via the `api_rate_limits` table (see `crates/ui/web-api-auth/src/auth/rate_limit.rs`). Rate limited endpoints return `429`
  with a message describing the limit window.
- Route handlers enforce permissions via typed Axum extractors (e.g. `CanViewHosts`, `CanManageAgents`) defined in
  `crates/ui/web-api/src/middleware/permission.rs`. The required permission is declared once in the extractor and
  reflected in the OpenAPI spec via the `x-required-permission` extension on every protected endpoint.
  See [Authentication and Authorization](../security/auth-and-authorization.md) for the full permission model.

## Health and Readiness Endpoints

Two unauthenticated probe endpoints are available on both the main HTTPS router and the
PKI HTTP router:

- `GET /healthz` — Liveness probe. Returns `200 OK` with body `ok`. No dependency checks.
- `GET /readyz` — Readiness probe. Checks database connectivity and CA bundle availability.
  Returns `200` when all checks pass, `503 Service Unavailable` when any check fails.

### `/readyz` response format

```json
{
  "status": "ready",
  "checks": {
    "database": "ok",
    "ca": "ok"
  }
}
```

When a check fails, its value is `"unavailable"` and `status` becomes `"unavailable"`.

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/health.rs` | `healthz` and `readyz` handlers |
| `crates/ui/web-api/src/router.rs` | Route registration on both routers |

## Authentication Endpoints

- `POST /api/v1/auth/device`: start a device authorization flow (RFC 8628). Returns `device_code`, `user_code`, `verification_url`, `expires_in`,
  `interval`.
- `POST /api/v1/auth/device/poll`: poll for approval status. The `status` field is a typed `DeviceAuthStatus` enum
  (`pending`, `authorized`, `expired`) defined in `uptrakit-shared-types`. Returns the API token once status is
  `authorized`.
- `POST /api/v1/auth/device/approve`: browser-side approval using Bearer token.
- `GET /api/v1/auth/device/stream?device_code=<code>`: SSE stream for device auth status updates. Returns `authorized` (with
  API token) or `expired` events. See [SSE Events API](sse-events.md#device-auth-sse).
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
- `POST /api/v1/services/{id}/update-freeze`: enable or disable the update freeze on a connected service. Requires `manage_agents`. See [ATK-17](../hackme/17-rce-controller-to-agent.md).
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

## Batch Update Endpoints

Batch updates allow triggering multiple updates in a single request with controller-managed
sequential per-host dispatch. Updates within a batch are dispatched one at a time per host;
after one completes (or fails), the next is dispatched.

### `POST /api/v1/hosts/{host_id}/batch-update`

Trigger a host-wide batch update for all outdated software items on a host. Requires `ManageSoftware`.

**Request body** (`HostBatchUpdateRequest`):

```json
{
  "category_filter": "security",
  "exclude_item_ids": ["<uuid>"]
}
```

- `category_filter`: optional. Only include items with this update category (e.g. `security`). Omit for all outdated.
- `exclude_item_ids`: optional. Exclude these software item UUIDs from the batch.

**Response** (`200`): `BatchUpdateResponse`

```json
{
  "batch_id": "019...",
  "total_created": 5,
  "updates": [
    {
      "update_history_id": "019...",
      "software_item_id": "019...",
      "software_item_name": "nginx",
      "host_id": "019...",
      "host_name": "web-01",
      "to_version": "1.27.0",
      "trigger_status": "pending"
    }
  ],
  "skipped": [
    {
      "software_item_id": "019...",
      "software_item_name": "redis",
      "host_id": "019...",
      "host_name": "web-01",
      "reason": "update already in progress"
    }
  ]
}
```

If no eligible items are found, returns `200` with `batch_id: null` and `total_created: 0`.

### `POST /api/v1/software-items/{id}/batch-update`

Trigger an item-wide batch update to roll out a software item version to hosts. Requires `ManageSoftware`.

**Request body** (`ItemBatchUpdateRequest`):

```json
{
  "to_version": "3.0.0",
  "host_ids": ["<uuid>", "<uuid>"]
}
```

- `to_version`: required. Target version to update to.
- `host_ids`: optional. Limit to these host UUIDs. Omit to include all assigned hosts with outdated versions.

**Response** (`200`): `BatchUpdateResponse` (same format as above).

### `GET /api/v1/update-batches`

List update batches with optional filters and pagination. Requires `ViewSoftware`.

**Query parameters**: `status` (optional), `page`, `per_page`.

**Response** (`200`): `PaginatedResponse<UpdateBatchSummaryResponse>`

```json
{
  "items": [
    {
      "id": "019...",
      "batch_type": "host_update",
      "status": "completed",
      "total_count": 5,
      "completed_count": 5,
      "failed_count": 0,
      "pending_count": 0,
      "actor_type": "user",
      "actor_id": "019...",
      "created_at": "2026-03-01T12:00:00Z",
      "completed_at": "2026-03-01T12:05:00Z"
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 20,
  "total_pages": 1
}
```

### `GET /api/v1/update-batches/{id}`

Get a single update batch with per-item update details. Requires `ViewSoftware`.

**Response** (`200`): `UpdateBatchDetailResponse` (extends summary with `updates` array).

### Batch Progress Streaming (SSE)

`GET /api/v1/update-batches/{id}/stream` — Server-Sent Events endpoint for real-time batch
progress. Requires `ViewSoftware`. Same authentication as the update output SSE endpoint.

Three event types are emitted:

**`update`** — an individual update within the batch changed status:

```text
event: update
data: {"event":"update_completed","update_history_id":"<uuid>","software_item_name":"nginx","host_name":"web-01"}
```

**`progress`** — overall batch progress summary:

```text
event: progress
data: {"completed":3,"failed":0,"pending":2,"total":5}
```

**`batch_completed`** — the batch reached a terminal status (stream ends after this):

```text
event: batch_completed
data: {"status":"completed"}
```

### Response types

Types are defined in `crates/shared/web-api-types/src/update_batches.rs`:

| Type | Fields |
| --- | --- |
| `HostBatchUpdateRequest` | `category_filter?`, `exclude_item_ids?` |
| `ItemBatchUpdateRequest` | `to_version`, `host_ids?` |
| `BatchUpdateResponse` | `batch_id?`, `total_created`, `updates`, `skipped` |
| `UpdateBatchSummaryResponse` | `id`, `batch_type`, `status`, counts, `actor_type`, `actor_id`, timestamps |
| `UpdateBatchDetailResponse` | Extends summary with `updates: Vec<UpdateBatchItemDetail>` |

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/update_batches.rs` | Route handlers and SSE endpoint |
| `crates/ui/web-api-queries/src/queries/update_batches.rs` | Batch query logic |
| `crates/ui/web-api/src/batch_progress_broadcaster.rs` | In-process broadcast registry |
| `crates/shared/web-api-types/src/update_batches.rs` | Request/response types |
| `crates/shared/db/src/entity/update_batch.rs` | SeaORM entity |

## Batch Action Endpoints

Batch actions allow performing the same operation on multiple entities in a single request.
Unlike batch updates (which create tracked update batches with progress streaming), batch
actions are simple multi-ID operations that return per-item success/failure results.

| Method | Path | Supported actions | Permission |
| --- | --- | --- | --- |
| `POST` | `/api/v1/services/batch` | `approve`, `reject`, `deactivate`, `delete` | `ManageAgents` |
| `POST` | `/api/v1/system-services/batch` | `approve`, `reject`, `deactivate`, `delete` | `ManageSystemServices` |
| `POST` | `/api/v1/hosts/batch` | `deactivate`, `delete` | `ManageHosts` |
| `POST` | `/api/v1/hosts/{host_id}/packages/batch` | `delete`, `enable`, `disable` | `ManageSoftware` |
| `POST` | `/api/v1/software-items/batch` | `approve`, `delete` | `ManageSoftware` |
| `POST` | `/api/v1/plugin-configs/batch` | `delete` | `ManageSoftware` |
| `POST` | `/api/v1/autodiscovery/ignores/batch` | `delete` | `ManageSoftware` |

See [Batch Actions API](batch-actions.md) for full request/response schema and error handling.

## Host Package Endpoints

Host packages represent per-host system-level packages tracked through the host packages subsystem.
See [Host Packages API](host-packages.md) for the full endpoint reference with request/response
examples.

- `GET /api/v1/hosts/{host_id}/packages` — list packages with pagination and filters
  (`?enabled=`, `?has_update=`, `?category=`, `?search=`). Requires `ViewSoftware`.
- `GET /api/v1/hosts/{host_id}/packages/{id}` — single package detail with update history.
  Requires `ViewSoftware`.
- `PUT /api/v1/hosts/{host_id}/packages/{id}` — update (enable/disable). Requires
  `ManageSoftware`.
- `DELETE /api/v1/hosts/{host_id}/packages/{id}` — soft-delete (`?ignore=true` to also create
  ignore rule). Requires `ManageSoftware`.
- `POST /api/v1/hosts/{host_id}/packages/{id}/update` — trigger single package update. Requires
  `ManageSoftware`.
- `POST /api/v1/hosts/{host_id}/packages/{id}/promote` — promote a host package into a tracked
  software item. Requires `ManageSoftware`. See [host-packages.md](host-packages.md#promote-a-host-package).
- `POST /api/v1/hosts/{host_id}/packages/update-all` — trigger batch update for outdated
  packages (`?category=security` for security-only). Requires `ManageSoftware`.
- `POST /api/v1/hosts/{host_id}/packages/check-versions` — trigger version check. Requires
  `ManageSoftware`.
- `GET /api/v1/hosts/{host_id}/package-ignores` — list ignore rules. Requires `ViewSoftware`.
- `POST /api/v1/hosts/{host_id}/package-ignores` — create ignore rule. Requires
  `ManageSoftware`.
- `DELETE /api/v1/hosts/{host_id}/package-ignores/{id}` — remove ignore rule. Requires
  `ManageSoftware`.

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/host_packages.rs` | Route handlers |
| `crates/ui/web-api-queries/src/queries/host_packages.rs` | CRUD queries |
| `crates/ui/web-api-queries/src/queries/host_package_ignores.rs` | Ignore list queries |
| `crates/ui/web-api-queries/src/queries/host_package_triggers.rs` | Update trigger pipeline |
| `crates/shared/web-api-types/src/host_packages.rs` | Request/response types |

## System Services Endpoints

System services are tenant-agnostic infrastructure components (MQTT bridge, external scheduler).
They are stored in the `system_services` table and managed independently of tenant services.
See [System Services Architecture](../architecture/system-services.md) for the full design.

- `GET /api/v1/system-services`: list system services (requires `view_system_services`).
  Filterable by `?capability=mqtt_bridge` or `?status=pending`. Paginated.
- `GET /api/v1/system-services/{id}`: get a single system service by UUID
  (requires `view_system_services`).
- `PUT /api/v1/system-services/{id}`: update configurable settings — `ping_interval_seconds` and
  `cert_lifetime_hours` (requires `manage_system_services`). Same field semantics as
  `PUT /api/v1/services/{id}`: `0` clears the override, positive value sets it, omit to keep current.
- `POST /api/v1/system-services/{id}/approve`: approve a pending system service
  (requires `manage_system_services`).
- `POST /api/v1/system-services/{id}/reject`: reject a pending system service
  (requires `manage_system_services`).
- `DELETE /api/v1/system-services/{id}`: deactivate (soft-delete) a system service, revoke its
  certificates, and bump the CRL (requires `manage_system_services`). Returns `204 No Content`.

### System Enrollment Token Endpoints

System enrollment tokens allow infrastructure services to enroll with automatic approval instead of
waiting for manual operator review. They are global (not tenant-scoped), backend-generated,
Argon2id-hashed, and shown only once at creation — matching the security model of tenant enrollment
tokens. All endpoints require `manage_system_services`.

- `POST /api/v1/system-enrollment-tokens`: create a new system enrollment token. Returns
  `201 Created` with `SystemEnrollmentTokenCreatedResponse` (includes the plaintext `token` field —
  store it immediately, it cannot be retrieved later).
- `GET /api/v1/system-enrollment-tokens`: list tokens with pagination. Returns
  `PaginatedResponse<SystemEnrollmentTokenResponse>` (no `token` field).
- `GET /api/v1/system-enrollment-tokens/{id}`: get a single token's metadata by UUID.
  Returns `SystemEnrollmentTokenResponse`.
- `DELETE /api/v1/system-enrollment-tokens/{id}`: soft-revoke a token (sets `revoked_at`).
  Returns `204 No Content`.

**Create request body** (`CreateSystemEnrollmentTokenRequest`):

```json
{
  "name": "MQTT Bridge Token",
  "max_uses": 5,
  "expires_in_seconds": 86400
}
```

`max_uses` and `expires_in_seconds` are optional. Omit for an unlimited, non-expiring token.

**Create response** (`201`): `SystemEnrollmentTokenCreatedResponse`

```json
{
  "id": "...",
  "token": "upt_xxxxxxxxxxxxxxxx",
  "name": "MQTT Bridge Token",
  "max_uses": 5,
  "current_uses": 0,
  "expires_at": "2026-03-05T00:00:00Z",
  "created_at": "2026-03-04T12:00:00Z",
  "created_by_user_id": "..."
}
```

**List/show response**: same fields, without `token`, with `revoked_at` added.

#### Enrollment behaviour

| Scenario | Result |
| --- | --- |
| No token provided | `Pending` — requires manual approval |
| Token matches an active (non-expired, non-revoked, uses remaining) system enrollment token | `Approved` — `current_uses` incremented, `system_enrollment_token_id` recorded |
| Token provided but no match | `403 Forbidden` |

#### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/system_enrollment_tokens.rs` | Route handlers |
| `crates/ui/web-api-queries/src/queries/system_enrollment_tokens.rs` | DB query helpers |
| `crates/shared/web-api-types/src/system_enrollment_tokens.rs` | Request/response types |
| `crates/shared/db/src/entity/system_enrollment_token.rs` | SeaORM entity |
| `crates/shared/openapi-client/src/system_enrollment_tokens.rs` | Typed HTTP client methods |

### Audit Log Endpoints

Read-only access to the audit trail. Both endpoints use the same filter parameters
(`actor_type`, `method`, `status`, `from`, `to`, `actor_id`, `page`, `per_page`).
See [Audit Logs API Reference](audit-logs.md) for the full specification.

- `GET /api/v1/audit-logs`: list tenant-scoped audit log entries
  (requires `view_audit_logs`). Returns `PaginatedResponse<AuditLogResponse>`.
- `GET /api/v1/system-audit-logs`: list system-level audit log entries
  (requires `view_system_audit_logs`, `owner` only). Returns `PaginatedResponse<SystemAuditLogResponse>`.

### `SystemServiceResponse` fields

| Field | Type | Description |
| --- | --- | --- |
| `id` | UUID | System service identifier |
| `capabilities` | `string[]` | Snake-case capability strings (e.g. `["mqtt_bridge","graceful_shutdown"]`) |
| `hostname` | string | Hostname reported at enrollment |
| `friendly_name` | string | Human-readable display name |
| `ip_address` | `string?` | Client IP address |
| `status` | string | `pending`, `approved`, `rejected`, or `deactivated` |
| `client_version` | `string?` | Client software version |
| `last_seen_at` | `datetime?` | Last connect or heartbeat time |
| `created_at` | datetime | Row creation time |
| `updated_at` | datetime | Last modification time |
| `ping_interval_seconds` | `u32?` | Per-service ping interval override (omitted when using the default) |
| `cert_lifetime_hours` | `u32?` | Per-service certificate lifetime override in hours (omitted when using the default) |

### Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/system_services.rs` | Route handlers |
| `crates/ui/web-api-queries/src/queries/system_services.rs` | DB query helpers |
| `crates/shared/web-api-types/src/system_services.rs` | Request/response types |
| `crates/shared/db/src/entity/system_service.rs` | SeaORM entity for `system_services` |
| `crates/shared/db/src/entity/system_service_certificate.rs` | SeaORM entity for `system_service_certificates` |
| `crates/shared/openapi-client/src/system_services.rs` | Typed HTTP client methods |

## Service Operations

- `/api/v1/agents/{id}/version-check`: trigger a version check (requires `ManageSoftware`).
- `/api/v1/agents/{id}/execute-update`: send `execute_update` (requires `ManageSoftware`).
- `/api/v1/mqtt/tenants`: manage MQTT tenant assignments (requires `ManageSettings`).

Update history records each attempt and stores the full command output for auditing.

**`UpdateStatus` values** in history responses:

| Value | Meaning |
| :---- | :------ |
| `queued` | Batch item waiting for a preceding item on the same host to complete. Counts as `update_in_progress` in MQTT state. |
| `pending` | Dispatched to the agent; not yet started. Holds the per-host active lock. |
| `in_progress` | Agent is executing the update. Holds the per-host active lock. |
| `completed` | Update succeeded (terminal). |
| `failed` | Update failed (terminal). |

Triggers return **HTTP 409** if another update (`pending` or `in_progress`) already exists for the target host,
across both software-item updates and host-package batches.

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

- **Store**: `crates/ui/web-api-auth/src/auth/rate_limit.rs` — `RateLimitStore` with sliding-window counter algorithm using
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
| `GET /api/v1/system-services` | `ListSystemServicesQuery` (includes `page`/`per_page`) | Filterable by `capability`, `status` |
| `GET /api/v1/hosts` | `PaginationParams` | |
| `GET /api/v1/software-items` | `PaginationParams` | |
| `GET /api/v1/update-history` | `UpdateHistoryQuery` (includes `page`/`per_page`) | Filterable by `host_id`, `software_item_id`, `status` |
| `GET /api/v1/plugin-configs` | `PaginationParams` | |
| `GET /api/v1/enrollment-tokens` | `ListEnrollmentTokensQuery` (includes `page`/`per_page`) | |
| `GET /api/v1/notifications/channels` | `PaginationParams` | |
| `GET /api/v1/notifications/rules` | `ListRulesQuery` (includes `page`/`per_page`) | Filterable by `channel_id`, `event_type` |
| `GET /api/v1/notifications/log` | `PaginationParams` | |
| `GET /api/v1/update-batches` | `UpdateBatchListQuery` (includes `page`/`per_page`) | Filterable by `status` |
| `GET /api/v1/hosts/{host_id}/packages` | `ListHostPackagesQuery` (includes `page`/`per_page`) | Filterable by `enabled`, `has_update`, `category`, `search` |

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

## SSE Endpoints

Two SSE (Server-Sent Events) endpoints provide real-time push notifications:

- `GET /api/v1/auth/device/stream?device_code=<code>` — Unauthenticated. Pushes `authorized` or `expired` events
  for the device authorization flow.
- `GET /api/v1/events/stream` — Authenticated (requires `ViewAgents` permission). Pushes admin events for the
  user's tenant (host/service/software changes, version checks, updates, discovery).

See [SSE Events API](sse-events.md) for full event format documentation.
