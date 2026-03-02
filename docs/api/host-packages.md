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
