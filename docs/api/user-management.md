# User Management API

All user management endpoints require the `ManageUsers` permission. They are global
(not tenant-scoped).

See [Authentication and Authorization](../security/auth-and-authorization.md) for the full
permission model, built-in roles, and access presets.

## User endpoints

### `GET /api/v1/users`

List all users with their assigned roles.

**Query parameters**: standard pagination (`limit`, `offset`).

**Response** (`200`): `PaginatedResponse<UserWithRolesResponse>`

```json
{
  "data": [
    {
      "id": "019...",
      "email": "admin@example.com",
      "first_name": "Admin",
      "last_name": "User",
      "is_active": true,
      "roles": [
        { "id": "019...", "name": "viewer" },
        { "id": "019...", "name": "service_manager" }
      ],
      "permissions": ["view_services", "view_software", "view_hosts", "view_settings",
                       "approve_services", "reject_services", "remove_services", "update_services"]
    }
  ],
  "total": 1,
  "limit": 50,
  "offset": 0
}
```

### `GET /api/v1/users/{id}`

Get a single user with their roles and resolved permissions.

**Path parameters**: `id` -- user UUID.

**Response** (`200`): `UserWithRolesResponse`

### `PUT /api/v1/users/{id}/roles`

Replace all role assignments for a user. The previous assignments are removed and the
provided role IDs are assigned.

**Path parameters**: `id` -- user UUID.

**Request body** (`UpdateUserRolesRequest`):

```json
{
  "role_ids": ["019...", "019..."]
}
```

**Validation**:

- `role_ids` must not be empty (at least one role required).
- `role_ids` must contain at most 20 entries.

**Lockout prevention**: if this change would remove `manage_users` from the last user who
has it, the request is rejected with `409 Conflict`.

**Response** (`200`): `UserWithRolesResponse` with updated roles and permissions.

### `PUT /api/v1/users/{id}/active`

Activate or deactivate a user account.

**Path parameters**: `id` -- user UUID.

**Request body** (`UpdateUserActiveRequest`):

```json
{
  "is_active": false
}
```

**Lockout prevention**: deactivating the last user with `manage_users` is rejected with
`409 Conflict`.

**Response** (`200`): `UserWithRolesResponse`.

### `POST /api/v1/users/{id}/apply-preset`

Apply an access preset to a user, replacing their current roles with those defined by
the preset.

**Path parameters**: `id` -- user UUID.

**Request body** (`ApplyPresetRequest`):

```json
{
  "preset": "administrator"
}
```

**Valid preset values**: `read_only`, `operator`, `manager`, `administrator`, `owner`.
See [Access Presets](../security/auth-and-authorization.md#access-presets) for the role
composition of each preset.

**Response** (`200`): `UserWithRolesResponse` with the preset's roles applied.

## Permission endpoints

### `GET /api/v1/permissions`

List all available permissions.

**Response** (`200`): `Vec<Permission>` (array of permission strings).

## Role endpoints

### `GET /api/v1/roles`

List all roles with their permissions.

**Response** (`200`): array of role objects with permissions.

### `GET /api/v1/roles/{id}`

Get a single role with its permissions.

**Path parameters**: `id` -- role UUID.

**Response** (`200`): role object with permissions.

## Access preset endpoints

### `GET /api/v1/access-presets`

List all access presets with their role compositions.

**Response** (`200`): `Vec<AccessPresetResponse>`

```json
[
  {
    "name": "read_only",
    "description": "Dashboard viewers, stakeholders",
    "roles": ["viewer"]
  },
  {
    "name": "operator",
    "description": "On-call staff, trigger checks/updates, approve agents",
    "roles": ["viewer", "operator"]
  },
  {
    "name": "manager",
    "description": "Team leads with full CRUD on services, software, hosts",
    "roles": ["viewer", "service_manager", "software_manager", "host_manager"]
  },
  {
    "name": "administrator",
    "description": "Tenant administrators with full management",
    "roles": ["viewer", "service_manager", "software_manager", "host_manager",
              "settings_manager", "command_manager"]
  },
  {
    "name": "owner",
    "description": "System owner with full control",
    "roles": ["viewer", "operator", "service_manager", "software_manager",
              "host_manager", "settings_manager", "command_manager",
              "system_administrator"]
  }
]
```

## Key files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/users.rs` | User and role route handlers |
| `crates/ui/web-api/src/routes/access_presets.rs` | Preset route handlers |
| `crates/shared/web-api-types/src/users.rs` | `UserWithRolesResponse`, `UpdateUserRolesRequest`, `UpdateUserActiveRequest`, `ApplyPresetRequest` |
| `crates/shared/web-api-types/src/access_presets.rs` | `AccessPresetResponse` |
| `crates/shared/types/src/permissions.rs` | `Permission` enum (32 variants) |
| `crates/shared/types/src/access_preset.rs` | `AccessPreset` enum (5 variants) |
| `crates/ui/web-api/src/middleware/permission.rs` | `CanManageUsers` extractor |
