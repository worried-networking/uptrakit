# Access Management API

M1.6a split the legacy `manage_users` permission into two actions: `users:manage` (user
lifecycle -- see [User Management API](user-management.md)) and `access:manage` (grants,
roles, and role assignment -- this page). A third, fine-grained action,
`system.access:manage`, additionally gates any mutation whose request body or stored
row confers **system-plane** authority (a pattern whose resource starts with `system.`,
e.g. `system.access:manage` or `system.*:*`).

This page documents the grant and role CRUD families, the role-assignment endpoint, and
the interim state of the access-preset endpoint. It reflects what M1.6a actually shipped,
not the original design. [Authentication and Authorization](../security/auth-and-authorization.md)
still describes the pre-split model; it will be rewritten in M1.9 to cover the full
`users:manage`/`access:manage`/`system.access:manage` picture.

## Grant endpoints

A grant is a row binding a subject (a user or a role) to a set of action patterns. All
grant endpoints require `access:manage`. Operations whose request body or stored row
reaches the system plane additionally require `system.access:manage`, checked at runtime
against the patterns in play -- this fine-grained requirement is not expressible as a
static OpenAPI scope, so it is documented in each operation's description instead of a
scope list.

| Method   | Path                         | Gate            | Extra gate (runtime)                                                                        |
| -------- | ---------------------------- | --------------- | ------------------------------------------------------------------------------------------- |
| `POST`   | `/api/v1/access/grants`      | `access:manage` | `system.access:manage` if the request's patterns reach the system plane                     |
| `GET`    | `/api/v1/access/grants`      | `access:manage` | --                                                                                          |
| `GET`    | `/api/v1/access/grants/{id}` | `access:manage` | --                                                                                          |
| `PUT`    | `/api/v1/access/grants/{id}` | `access:manage` | `system.access:manage` if the stored row's _or_ the written patterns reach the system plane |
| `DELETE` | `/api/v1/access/grants/{id}` | `access:manage` | `system.access:manage` if the stored row's patterns reach the system plane                  |

`GET /api/v1/access/grants` returns the active tenant's rows plus all global rows
(role-subject and system-plane grants have `tenant_id: null` and are visible tenant-wide).
It accepts `subject_type` + `subject_id` query parameters (`ListAccessGrantsQuery`) to
filter to one subject; the two must be supplied together or not at all, or the request is
rejected with `400`.

### Grant shape

```json
{
  "id": "019505a1-b2c3-7000-8000-000000000001",
  "tenant_id": null,
  "subject_type": "role",
  "subject_id": "019505a1-b2c3-7000-8000-000000000002",
  "patterns": ["hosts:read", "hosts:update"],
  "selector": "all",
  "description": "Read/update access for the ops role"
}
```

`tenant_id` is `null` for global rows -- role-subject grants and any grant whose patterns
reach the system plane are always global, regardless of who created them. A user-subject
grant whose patterns stay in the tenant plane gets the caller's active tenant.

### Create/update request bounds (`CreateAccessGrantRequest` / `UpdateAccessGrantRequest`)

| Field         | Bound                                                     |
| ------------- | --------------------------------------------------------- |
| `patterns`    | 1-16 entries; each 1-64 bytes                             |
| `selector`    | defaults to `"all"`; any other value is rejected until M2 |
| `description` | optional, at most 500 characters                          |

### Immutable encoding

`subject_type`, `subject_id`, and the row's tenant encoding are fixed at creation --
`PUT` only accepts `patterns`, `selector`, and `description`. To re-subject or re-scope a
grant, delete it and create a new one.

## Role endpoints

All role endpoints require `access:manage`.

| Method   | Path                 | Gate            | Extra gate (runtime)                                                                      |
| -------- | -------------------- | --------------- | ----------------------------------------------------------------------------------------- |
| `GET`    | `/api/v1/roles`      | `access:manage` | --                                                                                        |
| `GET`    | `/api/v1/roles/{id}` | `access:manage` | --                                                                                        |
| `POST`   | `/api/v1/roles`      | `access:manage` | --                                                                                        |
| `PUT`    | `/api/v1/roles/{id}` | `access:manage` | --                                                                                        |
| `DELETE` | `/api/v1/roles/{id}` | `access:manage` | `system.access:manage` if the role carries a role-subject grant reaching the system plane |

`GET /api/v1/roles` returns the global built-in roles plus the active tenant's custom
roles. `RoleResponse` no longer carries a `permissions` field -- a role's effective grants
are visible via `GET /api/v1/access/grants?subject_type=role&subject_id={role_id}`.

### Role name bounds (`CreateRoleRequest` / `UpdateRoleRequest`)

`name` must be 1-64 characters: lowercase alphanumeric plus `-`/`_`, starting with a
letter. `description` is optional, at most 500 characters.

### Built-in immutability

Built-in roles (`is_built_in: true`) cannot be renamed or deleted: both `PUT` and
`DELETE` return `409` with reason code `built_in_role_immutable`. They remain
**assignable** -- linking a user to a built-in role via `PUT /api/v1/users/{id}/roles`
is unaffected.

### Name-shadowing rejection

Creating or renaming a custom role rejects a name collision with `409`:

- `role_name_shadows_global` -- the name matches a global built-in role.
- `role_name_taken` -- the name matches another custom role already in this tenant.

### Deletion cascade

`DELETE /api/v1/roles/{id}` cascades: it deletes the role's own grants (role-subject
grants targeting it) and its user assignments in the same transaction as the role row,
then invalidates the authority cache and publishes an `AccessInvalidated` event naming
the affected user and role ids. Invalidation is all-or-nothing -- the ids are carried for
observability, not as a selective flush -- so no holder of the deleted role can retain
cached authority.

## Assignment endpoint

`PUT /api/v1/users/{id}/roles` replaces a user's entire role-assignment set. It gates on
`access:manage` (not `users:manage`). Adding a role whose grants reach the system plane
additionally requires `system.access:manage` -- the check is evaluated only against roles
the request actually _adds_ (the user's current assignments are re-read inside the
transaction so a concurrent unassign cannot make a re-added role look "already held" and
dodge the check). Re-applying a role the user already holds never re-triggers the
system-plane check, even if that role reaches the system plane.

Request body (`UpdateUserRolesRequest`): `role_ids` must contain 1-20 entries, all of
which must resolve to existing roles (unresolved ids -- `400`, no reason code).

## Preset endpoint (interim state)

`POST /api/v1/users/{id}/apply-preset` was **not** converted by this milestone. It still
gates on the legacy `manage_users` permission via `CanManageUsers` (the
`x-required-permission` OpenAPI extension, not a native `security()` scope) rather than
the `users:manage`/`access:manage` split -- a `users:manage`-only caller can currently
apply any preset, including ones that grant `access:manage`. Full conversion is
M1.6b.

One runtime check was added ahead of that conversion: applying a preset whose roles reach
the system plane (currently only the `owner` preset, via `system_administrator`)
additionally requires `system.access:manage`, checked inline against the engine. As with
role assignment, this is evaluated only against roles the preset would newly grant --
re-applying a preset the user already effectively holds does not re-trigger the check.

## Encoding rules for API consumers

- Subject (`subject_type`/`subject_id`) and tenant encoding are immutable after grant
  creation. To change a grant's subject, delete it and create a replacement.
- Role-subject grants are always global rows (`tenant_id: null`), regardless of pattern
  plane.
- Any grant whose patterns reach the system plane is always a global row, regardless of
  subject type.
- A single grant may not mix system-plane and tenant-plane patterns (`400`,
  `access_grant.plane_mixing`).

## Lockout 409 semantics

Guarded (authority-shrinking) mutations -- grant update, grant delete, role delete, user
role-assignment replacement, and user deactivation (`PUT /api/v1/users/{id}/active` --
see [User Management API](user-management.md#put-apiv1usersidactive)) -- run a lockout
guard before applying. A denial returns `409 Conflict` with one of two reason codes,
carried in `ErrorResponse.code`:

- `lockout_access_manage` -- the change would remove the last tenant-wide covering holder
  of `access:manage`.
- `lockout_system_access` -- the change would remove the last remaining holder of
  `system.access:manage`.

The response body carries the reason code only -- **never** holder identities or counts
-- so that a caller who only holds `users:manage` (and can reach the assignment endpoint)
cannot learn access-plane state from a denial. The same reason code is recorded in the
matching `Denied` audit event's `details`.

That audit event is scoped by the plane the guard fired on, matching how the same row's
successful mutations are scoped: `lockout_system_access` denials are system-scoped (routed
to `system_audit_log`, readable via `GET /api/v1/system-audit-logs`), `lockout_access_manage`
denials are tenant-scoped (`audit_log`).

Adding-only mutations -- grant create and role create -- are not guarded (they cannot
remove authority from anyone). Role rename (`PUT /api/v1/roles/{id}`) is not guarded
either, since it never changes what the role grants.

Grant-limit rejection is a separate, unguarded 409: creating a grant for a subject that
already holds 200 grants returns `409` with reason code `too_many_grants`.

### Known gap: OIDC role sync

OIDC login replaces a linked user's entire role set from the identity provider's claim
mapping on every sign-in. That path now runs the same lockout guard as the API surface:
a mapped replace that would strip the sole `access:manage`/`system.access:manage` covering
holder is skipped rather than applied, and the login proceeds with the pre-sync role set
(`RoleSyncOutcome::SkippedLockout`). When the sync does apply, it invalidates the affected
user's cached authority and publishes `AccessInvalidated` after commit, same as the guarded
endpoints above -- there is no separate, unguarded cache path anymore.

A skipped sync is otherwise silent to the signed-in user (they simply keep their prior
roles) -- the only operator-visible signal is the `user_role.sync_lockout_prevented` audit
Event. Alert on that action if you rely on OIDC role mapping to keep authority current.
That Event is the only audit signal the sync ever emits: a sync that _fails_ (a database
error, or a guard evaluation that cannot resolve the default-tenant sentinel) is logged at
`error` -- as is a link-path transaction that cannot be opened -- and then treated by every
call site as "no change", with no audit row at all. A link-path transaction that opens but
fails to commit is the one partial case: the write never landed, so an applied sync
degrades to "no change" (no invalidation, no publish), while a lockout denial -- which
wrote nothing either way -- still emits its Event. A
persistently failing guard therefore looks identical to a login whose roles simply did not
need updating -- watch the controller log, not just the audit trail.

The residual gap is de-provisioning drift, not lockout: the sync has several fail-open
early returns (no `role_claim_path` configured, empty `role_mapping`, claim path missing
from the token, an unmapped or malformed claim value, or the mapped set resolving to zero
local roles -- see `crates/shared/db/src/access_grants.rs`) that leave the user's existing
roles untouched with no signal at all, not even an audit event. Note that a provider's
`role_mapping` targets are resolved against **global** roles only. Targets naming a
tenant-scoped custom role match nothing and are dropped from the resolved set: if every
target is tenant-scoped the sync no-ops silently for that login, and if only some are, the
sync **applies the global subset** -- the user loses the custom role on every sign-in.
`role_mapping` is free-form text and is not validated against the role table at write time,
so a typo or a custom-role target is only visible as a sync that never applies or that
quietly applies less than it names. An IdP-side de-provisioning
that simply stops sending the covering claim never reaches the guard or the sync's write
path -- it silently no-ops. Operators relying on OIDC role mapping should keep at least one
local (non-OIDC) account holding `access:manage`.

## Revocation latency

Every guarded and adding mutation invalidates the affected subjects'
in-process authority cache and publishes `AccessInvalidated` after commit, so holders on
other controller instances pick up the change without re-authenticating. That publish is
backstopped by a 60-second cache TTL: an authority load already in flight elsewhere may
briefly re-observe the pre-mutation state until the backstop expires, even though the
originating instance invalidates and publishes immediately on commit.

## See also

- [User Management API](user-management.md) -- user lifecycle endpoints (`users:manage`).
- [Authentication and Authorization](../security/auth-and-authorization.md) -- full
  permission model; scheduled for a rewrite in M1.9 to cover this split.

## Key files

| File                                               | Purpose                                            |
| -------------------------------------------------- | -------------------------------------------------- |
| `crates/ui/web-api/src/routes/access_grants.rs`    | Grant CRUD route handlers                          |
| `crates/ui/web-api/src/routes/roles.rs`            | Role CRUD route handlers                           |
| `crates/ui/web-api/src/routes/users.rs`            | `update_user_roles` (assignment endpoint)          |
| `crates/ui/web-api/src/routes/access_presets.rs`   | Preset route handlers (interim gate)               |
| `crates/shared/web-api-types/src/access_grants.rs` | Grant request/response types and validation bounds |
| `crates/shared/web-api-types/src/roles.rs`         | Role request/response types and validation bounds  |
| `crates/shared/db/src/access_grants.rs`            | Engine-owned persistence, lockout guard, bounds    |
| `crates/ui/web-api/src/middleware/action.rs`       | `CanManageAccess`, `require_system_access`         |
