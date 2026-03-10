# Host Tags API

Host tags are user-defined labels for organizing hosts. All endpoints are scoped to the
authenticated user's tenant.

## Endpoints

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| `GET` | `/api/v1/host-tags` | `ViewHosts` | List host tags (paginated, searchable) |
| `POST` | `/api/v1/host-tags` | `UpdateHosts` | Create a host tag |
| `GET` | `/api/v1/host-tags/{id}` | `ViewHosts` | Get a single host tag |
| `PUT` | `/api/v1/host-tags/{id}` | `UpdateHosts` | Update a host tag |
| `DELETE` | `/api/v1/host-tags/{id}` | `DeactivateHosts` | Soft-delete a host tag |
| `POST` | `/api/v1/host-tags/batch` | `DeactivateHosts` | Batch delete host tags |
| `PUT` | `/api/v1/hosts/{id}/tags` | `UpdateHosts` | Set (replace-all) tags on a host |

## List host tags

```http
GET /api/v1/host-tags?page=1&per_page=20&search=prod
```

### Query parameters

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `page` | integer | 1 | Page number (1-indexed) |
| `per_page` | integer | 20 | Items per page (max 1000) |
| `search` | string | -- | Filter by name (case-insensitive contains) |

### Response `200`

```json
{
  "items": [
    {
      "id": "019505a1-b2c3-7000-8000-000000000001",
      "name": "production",
      "color": "#3B82F6",
      "description": "Production environment hosts",
      "created_at": "2026-03-09T10:00:00Z",
      "updated_at": "2026-03-09T10:00:00Z",
      "host_count": 5
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 20,
  "total_pages": 1
}
```

Results are ordered alphabetically by name. Only active (non-deleted) tags are returned.

## Create a host tag

```http
POST /api/v1/host-tags
Content-Type: application/json

{
  "name": "production",
  "color": "#3B82F6",
  "description": "Production environment hosts"
}
```

### Request body

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | string | Yes | Tag name (1--100 characters, unique per tenant) |
| `color` | string | No | Hex color code (e.g. `#3B82F6`). Auto-assigned from palette if omitted. |
| `description` | string | No | Optional description (max 500 characters) |

### Response `201`

Returns the created `HostTagResponse` (same schema as the list item above) with `host_count: 0`.

### Errors

| Status | Condition |
| --- | --- |
| `400` | Validation error (empty name, name too long, invalid color, description too long) |
| `409` | A tag with this name already exists in the tenant |

## Get a host tag

```http
GET /api/v1/host-tags/{id}
```

### Response `200`

Returns a single `HostTagResponse`.

### Errors

| Status | Condition |
| --- | --- |
| `404` | Tag not found or has been deleted |

## Update a host tag

```http
PUT /api/v1/host-tags/{id}
Content-Type: application/json

{
  "name": "staging",
  "color": "#10B981",
  "description": "Staging environment"
}
```

### Request body

All fields are optional. Omitted fields are left unchanged.

| Field | Type | Description |
| --- | --- | --- |
| `name` | string | New name (1--100 characters) |
| `color` | string | New hex color code |
| `description` | JSON | String to set, `null` to clear, omit to keep |

The `description` field follows the nullable update pattern: send a JSON string value to set it,
send `null` to clear it, or omit the field entirely to keep the current value.

### Response `200`

Returns the updated `HostTagResponse`.

### Errors

| Status | Condition |
| --- | --- |
| `400` | Validation error |
| `404` | Tag not found |
| `409` | Another tag with this name already exists |

## Delete a host tag

```http
DELETE /api/v1/host-tags/{id}
```

Performs a soft-delete (sets `deactivated_at`). All host assignments for this tag are hard-deleted
within the same transaction.

### Response `204`

No content.

### Errors

| Status | Condition |
| --- | --- |
| `404` | Tag not found or already deleted |

## Batch delete host tags

```http
POST /api/v1/host-tags/batch
Content-Type: application/json

{
  "action": "delete",
  "ids": [
    "019505a1-b2c3-7000-8000-000000000001",
    "019505a1-b2c3-7000-8000-000000000002"
  ]
}
```

### Request body

| Field | Type | Description |
| --- | --- | --- |
| `action` | string | Action to perform. Currently only `delete` is supported. |
| `ids` | UUID[] | List of host tag UUIDs (max 100) |

### Response `200`

```json
{
  "succeeded": [
    { "id": "019505a1-b2c3-7000-8000-000000000001" }
  ],
  "failed": [
    { "id": "019505a1-b2c3-7000-8000-000000000002", "error": "not found" }
  ]
}
```

Partial success is possible. Items that fail do not block successful items.

## Set tags on a host

```http
PUT /api/v1/hosts/{id}/tags
Content-Type: application/json

{
  "tag_ids": [
    "019505a1-b2c3-7000-8000-000000000001",
    "019505a1-b2c3-7000-8000-000000000003"
  ]
}
```

This is a **replace-all** operation: the provided list of tag IDs replaces all existing tag
assignments for the host. Send an empty array to remove all tags.

### Request body

| Field | Type | Description |
| --- | --- | --- |
| `tag_ids` | UUID[] | Tag IDs to assign (max 50). Invalid or cross-tenant IDs are silently ignored. |

### Response `200`

Returns the resulting `Vec<HostTagSummary>`:

```json
[
  {
    "id": "019505a1-b2c3-7000-8000-000000000001",
    "name": "production",
    "color": "#3B82F6"
  },
  {
    "id": "019505a1-b2c3-7000-8000-000000000003",
    "name": "critical",
    "color": "#EF4444"
  }
]
```

### Errors

| Status | Condition |
| --- | --- |
| `400` | Validation error (more than 50 tags) |
| `404` | Host not found or deactivated |

## Response types

### HostTagResponse

| Field | Type | Description |
| --- | --- | --- |
| `id` | UUID | Tag identifier |
| `name` | string | Tag name |
| `color` | string | Hex color code |
| `description` | string? | Optional description |
| `created_at` | datetime | RFC 3339 creation timestamp |
| `updated_at` | datetime | RFC 3339 last modification timestamp |
| `host_count` | integer | Number of hosts currently assigned to this tag |

### HostTagSummary

Slim representation included in `HostResponse.tags`:

| Field | Type | Description |
| --- | --- | --- |
| `id` | UUID | Tag identifier |
| `name` | string | Tag name |
| `color` | string | Hex color code |

## Admin events (SSE)

The following events are emitted via `GET /api/v1/events/stream`:

| Event type | Payload | Trigger |
| --- | --- | --- |
| `host_tag_created` | `{ id }` | Tag created |
| `host_tag_updated` | `{ id }` | Tag updated |
| `host_tag_deleted` | `{ id }` | Tag deleted |
| `host_tags_changed` | `{ host_id }` | Tags assigned/unassigned on a host |

## Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/host_tags.rs` | Route handlers |
| `crates/ui/web-api-queries/src/queries/host_tags.rs` | Query functions |
| `crates/shared/web-api-types/src/host_tags.rs` | Request/response types |
| `crates/shared/openapi-client/src/host_tags.rs` | Typed API client |

## See also

- [Host Tags Architecture](../architecture/host-tags.md) -- database schema and design
- [Host Entity](../architecture/host-entity.md) -- host data model
- [Batch Actions API](batch-actions.md) -- general batch action pattern
- [CLI Usage Guide](../end-user/cli-usage.md#host-tags) -- CLI commands
