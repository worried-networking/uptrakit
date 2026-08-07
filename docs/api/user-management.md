# User Management API

User lifecycle endpoints (list, read, activate/deactivate) require the `users:manage` action.
Role assignment and role CRUD require `access:manage` — see
[Access Management](access-management.md). Both families are global (not tenant-scoped).

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
      "permissions": [
        "view_services",
        "view_software",
        "view_hosts",
        "view_settings",
        "approve_services",
        "reject_services",
        "remove_services",
        "update_services"
      ]
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
provided role IDs are assigned. Unlike the other endpoints on this page, this one gates
on `access:manage`, not `users:manage` -- see [Access Management API](access-management.md)
for the full grant/role/assignment gate reference and the M1.6a permission split.

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

**Lockout prevention**: if this change would remove the last remaining `access:manage` or
`system.access:manage` holder, the request is rejected with `409 Conflict` (reason codes
`lockout_access_manage` / `lockout_system_access` -- see
[Access Management API](access-management.md#lockout-409-semantics)).

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

**Lockout prevention**: deactivating the last remaining `access:manage` or
`system.access:manage` holder is rejected with `409 Conflict` (reason codes
`lockout_access_manage` / `lockout_system_access` -- see
[Access Management API](access-management.md#lockout-409-semantics)).

**Response** (`200`): `UserWithRolesResponse`.

Role bundles (`read_only`, `operator`, `manager`, `administrator`, `owner`) are catalog
metadata, not a separate endpoint -- look up a bundle's role composition via
`GET /api/v1/access/catalog` (see [Access Management API](access-management.md#catalog-endpoint))
and apply it via `PUT /api/v1/users/{id}/roles` above.

## Permission endpoints

### `GET /api/v1/permissions`

List all available permissions.

**Response** (`200`): `Vec<Permission>` (array of permission strings).

## Role endpoints

Role CRUD (`GET`/`POST`/`PUT`/`DELETE`) gates on `access:manage`, not `users:manage` --
see [Access Management API](access-management.md#role-endpoints) for the full reference.

### `GET /api/v1/roles`

List all roles for the active tenant plus the global built-ins.

**Response** (`200`): array of role objects. `RoleResponse` no longer carries a
`permissions` field -- a role's effective grants are visible via
`GET /api/v1/access/grants?subject_type=role&subject_id={role_id}`.

### `GET /api/v1/roles/{id}`

Get a single role.

**Path parameters**: `id` -- role UUID.

**Response** (`200`): role object (no `permissions` field; see above).

## Key files

| File                                             | Purpose                                                                                     |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------- |
| `crates/ui/web-api/src/routes/users.rs`          | User lifecycle handlers (`CanManageUsers`) and role assignment (`CanManageAccess`)          |
| `crates/ui/web-api/src/routes/roles.rs`          | Role CRUD handlers (`CanManageAccess`) -- see [Access Management API](access-management.md) |
| `crates/ui/web-api/src/middleware/action.rs`     | `CanManageUsers`, `CanManageAccess` typed extractors                                        |
| `crates/shared/web-api-types/src/users.rs`       | `UserWithRolesResponse`, `UpdateUserRolesRequest`, `UpdateUserActiveRequest`                |
| `crates/shared/web-api-types/src/roles.rs`       | `RoleResponse`, `CreateRoleRequest`, `UpdateRoleRequest`                                    |
| `crates/shared/types/src/role_bundle.rs`         | `RoleBundle` enum (catalog metadata)                                                        |
| `crates/ui/web-api/src/middleware/permission.rs` | `CanManageUsers` extractor                                                                  |
