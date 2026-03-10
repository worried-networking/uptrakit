# User Management

Uptrakit uses a granular role-based access control (RBAC) system with 32 permissions grouped
into 8 built-in roles. Users are assigned one or more roles, and each role grants a specific
set of permissions. Access presets provide convenient role bundles for common use cases.

## First user setup

The first user to register (via password or OIDC) automatically receives all 8 built-in
roles, equivalent to the **owner** access preset. This ensures the initial administrator
has full control. Subsequent users receive only the **viewer** role by default.

## Built-in roles

| Role | Purpose | Key permissions |
| --- | --- | --- |
| **viewer** | Read-only access | View services, software, hosts, settings |
| **operator** | Day-to-day operations | Approve/reject services, trigger checks and updates |
| **service_manager** | Full service lifecycle | Approve, reject, remove, update services |
| **software_manager** | Software management | Create, edit, delete software; trigger checks/updates; manage scheduler and ignore rules |
| **host_manager** | Host management | Update host properties/tags, deactivate hosts |
| **settings_manager** | Tenant administration | Auth settings, enrollment tokens, certificates, notifications, audit logs, user management |
| **command_manager** | Command configuration | Modify command-bearing plugin config fields (equivalent to root access on managed hosts) |
| **system_administrator** | Infrastructure management | Global settings, system services, system audit logs |

## Access presets

Presets assign a predefined set of roles in a single operation. They are useful for quickly
setting up user access levels without manually selecting individual roles.

| Preset | Roles | Typical use case |
| --- | --- | --- |
| **read_only** | viewer | Stakeholders, dashboard viewers |
| **operator** | viewer, operator | On-call staff |
| **manager** | viewer, service_manager, software_manager, host_manager | Team leads |
| **administrator** | viewer, service_manager, software_manager, host_manager, settings_manager, command_manager | Tenant administrators |
| **owner** | All 8 roles | System owners |

## Managing users

Users with the `manage_users` permission can manage other users through the REST API
or the CLI.

### Viewing users

```bash
# List all users
uptrakit-cli users list

# Get a specific user
uptrakit-cli users get <user-id>
```

### Changing user roles

```bash
# Replace a user's roles (provide role IDs)
uptrakit-cli users set-roles <user-id> --role-ids <id1>,<id2>

# Apply an access preset
uptrakit-cli users apply-preset <user-id> --preset administrator
```

### Activating and deactivating users

```bash
# Deactivate a user (prevents login, revokes sessions)
uptrakit-cli users set-active <user-id> --active false

# Reactivate a user
uptrakit-cli users set-active <user-id> --active true
```

### Viewing roles and permissions

```bash
# List all roles
uptrakit-cli roles list

# List all permissions
uptrakit-cli permissions list

# List access presets
uptrakit-cli access-presets list
```

## Lockout prevention

To prevent accidental lockout, Uptrakit enforces the following rule: you cannot remove
the `manage_users` permission from the last user who holds it. This applies to both role
changes and user deactivation. Attempts that would violate this rule are rejected with an
error.

## Security considerations

- The **command_manager** role grants the ability to configure arbitrary shell commands that
  run on managed hosts. Assign it with the same care as granting root/sudo access.
- The **system_administrator** role provides access to global infrastructure settings and
  system services. It should be limited to infrastructure operators.
- User deactivation prevents login but does not immediately invalidate existing JWT access
  tokens (which expire within 15 minutes). For immediate revocation, also revoke the user's
  API tokens.

## Related documentation

- [Authentication and Authorization](../security/auth-and-authorization.md) -- full
  permission model, role definitions, and security details
- [User Management API](../api/user-management.md) -- REST API reference for all
  user management endpoints
