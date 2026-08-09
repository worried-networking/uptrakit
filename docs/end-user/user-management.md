---
title: User Management
weight: 240
description: Uptrakit uses an action-based grant authorization system -- roles are named bundles of grant patterns, with seeded built-in roles, per-tenant custom roles, and code-defined role bundles for common use cases.
---

# User Management

Uptrakit uses an action-based grant authorization system. Roles are named bundles of grant
patterns -- seeded built-in roles plus per-tenant custom roles (roles are data, not code).
Users are assigned one or more roles, and each role's grants determine which `resource:verb`
actions the user can perform. Role bundles name convenient role groupings for common use cases.

## First user setup

The first user to register (via password or OIDC) automatically receives all built-in
roles, equivalent to the **owner** role bundle. This ensures the initial administrator
has full control. Subsequent users receive only the **viewer** role by default.

## Built-in roles

| Role                     | Purpose                   | Key actions                                                                            |
| ------------------------ | ------------------------- | -------------------------------------------------------------------------------------- |
| **viewer**               | Read-only access          | `*:read`                                                                               |
| **operator**             | Day-to-day operations     | `services:approve`, `services:reject`, `checks:trigger`, `updates:trigger`             |
| **service_manager**      | Full service lifecycle    | `services:*`                                                                           |
| **software_manager**     | Software management       | `software:*`, `scheduler:manage`, `discovery.ignores:manage`, `plugin-configs:trigger` |
| **host_manager**         | Host management           | `hosts:*`, `hosts.tags:manage`                                                         |
| **settings_manager**     | Tenant administration     | `settings.*:manage`, `notifications:*`, `audit:read`, `users:manage`, `access:manage`  |
| **command_manager**      | Command configuration     | `commands:manage`, `plugin-configs:trigger`                                            |
| **system_administrator** | Infrastructure management | `system.*:*`                                                                           |

## Role bundles

Role bundles name a predefined set of roles. They are advisory metadata served by
`uptrakit-cli api GET /api/v1/access/catalog` -- look up a bundle's roles there, then assign
them with `users set-roles` (see below).

| Bundle            | Roles                                                                                      | Typical use case                |
| ----------------- | ------------------------------------------------------------------------------------------ | ------------------------------- |
| **read_only**     | viewer                                                                                     | Stakeholders, dashboard viewers |
| **operator**      | viewer, operator                                                                           | On-call staff                   |
| **manager**       | viewer, service_manager, software_manager, host_manager                                    | Team leads                      |
| **administrator** | viewer, service_manager, software_manager, host_manager, settings_manager, command_manager | Tenant administrators           |
| **owner**         | All built-in roles                                                                         | System owners                   |

## Custom roles and grants

Role and grant management is API/CLI-only in v1 -- there is no web UI for creating custom
roles or editing grants yet. Assign a user's roles by name with:

```bash
uptrakit-cli users set-roles <user-id> --names <comma-separated-role-names>
```

Grants -- the `resource:verb` action patterns attached to a role or a user -- are managed
through the access API; see [Access Management API](../api/access-management.md) for grant
and role CRUD endpoints. The catalog endpoint (`GET /api/v1/access/catalog`) lists the full
action catalog and the code-defined role bundles.

## Managing users

Users with the **settings_manager** role (or broader) can manage other users through the
REST API or the CLI.

### Viewing users

```bash
# List all users
uptrakit-cli users list

# Get a specific user
uptrakit-cli users get <user-id>
```

### Changing user roles

```bash
# Replace a user's roles (role IDs are positional, space-separated)
uptrakit-cli users set-roles <user-id> <id1> <id2>

# Look up a role bundle's role composition in the catalog, then apply it by name
uptrakit-cli api GET /api/v1/access/catalog
uptrakit-cli users set-roles <user-id> --names viewer,service_manager
```

### Activating and deactivating users

```bash
# Deactivate a user (prevents login, revokes sessions)
uptrakit-cli users set-active <user-id> --active false

# Reactivate a user
uptrakit-cli users set-active <user-id> --active true
```

### Viewing roles and grants

```bash
# List all roles
uptrakit-cli roles list

# List the available action catalog (actions, categories, role bundles)
uptrakit-cli api GET /api/v1/access/catalog
```

## Lockout prevention

To prevent accidental lockout, Uptrakit enforces the following rule: you cannot remove the
last remaining holder of the `access:manage` or `system.access:manage` action -- the actions
that grant role and access-grant administration. This applies to both role changes and user
deactivation. Attempts that would violate this rule are rejected with an error.

## Security considerations

- The **command_manager** role grants the ability to configure arbitrary shell commands that
  run on managed hosts. Assign it with the same care as granting root/sudo access.
- The **system_administrator** role provides access to global infrastructure settings and
  system services. It should be limited to infrastructure operators.
- User deactivation prevents login but does not immediately invalidate existing JWT access
  tokens (which expire within 15 minutes). For immediate revocation, also revoke the user's
  API tokens.

## Related documentation

- [Authentication and Authorization](../security/auth-and-authorization.md#authorization-model) --
  authorization model, role definitions, and security details
- [User Management API](https://github.com/worried-networking/uptrakit/tree/main/docs/api/) -- REST API reference for all
  user management endpoints
