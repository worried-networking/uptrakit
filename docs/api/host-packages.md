# Host packages API

REST API for managing host-level package tracking. All endpoints are scoped to a specific host.

See [architecture: host packages](../architecture/host-packages.md) for entity design and
[CLI usage](../end-user/cli-usage.md) for the command-line interface.

## Endpoints

| Method | Path | Permission | Description |
| :----- | :--- | :--------- | :---------- |
| GET | `/api/v1/hosts/{host_id}/packages` | ViewSoftware | List packages with filtering and pagination |
| GET | `/api/v1/hosts/{host_id}/packages/{id}` | ViewSoftware | Get package detail with update history |
| PUT | `/api/v1/hosts/{host_id}/packages/{id}` | ManageSoftware | Update package (enable/disable) |
| DELETE | `/api/v1/hosts/{host_id}/packages/{id}` | ManageSoftware | Soft-delete package |
| POST | `/api/v1/hosts/{host_id}/packages/{id}/promote` | ManageSoftware | Promote package to tracked software item |
| GET | `/api/v1/hosts/{host_id}/package-ignores` | ViewSoftware | List ignore rules |
| POST | `/api/v1/hosts/{host_id}/package-ignores` | ManageSoftware | Create ignore rule |
| DELETE | `/api/v1/hosts/{host_id}/package-ignores/{id}` | ManageSoftware | Remove ignore rule |

## List packages

```text
GET /api/v1/hosts/{host_id}/packages
```

### Query parameters

| Parameter | Type | Description |
| :-------- | :--- | :---------- |
| `page` | integer | Page number (1-indexed) |
| `per_page` | integer | Items per page (default: 25) |
| `enabled` | boolean | Filter by enabled status |
| `has_update` | boolean | Filter by update availability |
| `category` | string | Filter by update category (`security`, `standard`, `unknown`) |
| `search` | string | Search by package name |

### Response

```json
{
  "items": [
    {
      "id": "uuid",
      "host_id": "uuid",
      "plugin_config_id": "uuid",
      "package_identifier": "nginx",
      "name": "nginx",
      "installed_version": "1.22.0",
      "installed_version_detected_at": "2024-01-01T00:00:00Z",
      "latest_version": "1.24.0",
      "latest_version_fetched_at": "2024-01-01T00:00:00Z",
      "update_category": "standard",
      "enabled": true,
      "last_checked_at": "2024-01-01T00:00:00Z",
      "last_updated_at": null,
      "created_at": "2024-01-01T00:00:00Z",
      "has_update": true
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 25,
  "total_pages": 1
}
```

## Get package detail

```text
GET /api/v1/hosts/{host_id}/packages/{id}
```

Returns the package with recent update history entries.

```json
{
  "package": { "...same as list item..." },
  "recent_updates": [
    {
      "id": "uuid",
      "from_version": "1.22.0",
      "to_version": "1.24.0",
      "status": "completed",
      "output": null,
      "created_at": "2024-01-01T00:00:00Z"
    }
  ]
}
```

## Update package

```text
PUT /api/v1/hosts/{host_id}/packages/{id}
```

### Request body

```json
{
  "enabled": false
}
```

Returns the updated `HostPackageResponse`.

## Delete package

```text
DELETE /api/v1/hosts/{host_id}/packages/{id}[?ignore=true]
```

Soft-deletes the package. If `ignore=true`, also creates an ignore rule to prevent re-discovery.

Returns `204 No Content`.

## Promote a host package

```text
POST /api/v1/hosts/{host_id}/packages/{id}/promote
```

Promotes an auto-discovered host package into a fully tracked software item. The original host
package is unchanged (additive operation). The software item is pre-populated with the installed
version, latest version, and all three plugin roles (`DetectVersion`, `FetchReleases`,
`ExecuteUpdate`) pointing to the same plugin config and package identifier as the source package.

### Idempotency

If the host already has a `host_software_item_plugin` row matching the same
`(host_id, plugin_config_id, package_identifier)` triple, the existing software item is returned
without creating duplicates. Providing `software_item_id` explicitly bypasses the auto-detection
and links the package to the specified item instead.

### Request body

All fields are optional.

```json
{
  "name": "Claude Code",
  "software_item_id": "uuid"
}
```

| Field | Type | Description |
| :---- | :--- | :---------- |
| `name` | string | Display name for the new software item. Defaults to the package name. Ignored when `software_item_id` is provided. Must not be blank if present. |
| `software_item_id` | UUID | Promote into an existing software item instead of creating a new one. Must belong to the same tenant. |

### Response

Returns `200 OK` with a `SoftwareItemDetailResponse` — the same structure returned by
`GET /api/v1/software-items/{id}`, including the host assignment and version data.

### Errors

| Status | Condition |
| :----- | :-------- |
| `400 Bad Request` | `name` is blank |
| `404 Not Found` | Package or explicit `software_item_id` not found |
| `500 Internal Server Error` | Database error |

## Ignore rules

### List ignore rules

```text
GET /api/v1/hosts/{host_id}/package-ignores
```

Returns an array of `HostPackageIgnoreResponse`:

```json
[
  {
    "id": "uuid",
    "plugin_config_id": "uuid",
    "package_identifier": "nginx",
    "created_at": "2024-01-01T00:00:00Z"
  }
]
```

### Create ignore rule

```text
POST /api/v1/hosts/{host_id}/package-ignores
```

```json
{
  "plugin_config_id": "uuid",
  "package_identifier": "nginx"
}
```

Returns the created `HostPackageIgnoreResponse`.

### Remove ignore rule

```text
DELETE /api/v1/hosts/{host_id}/package-ignores/{id}
```

Returns `204 No Content`.

## Related documentation

- [Architecture: host packages](../architecture/host-packages.md)
- [CLI: host-packages](../end-user/cli-usage.md)
- [Security: auth and authorization](../security/auth-and-authorization.md) — permission model
