# Authentication and Authorization

| Method | Scope | Details |
| --- | --- | --- |
| Password (Argon2id) | User login | Local accounts with hashed passwords. |
| OIDC | User login | External identity providers with auto-create or account linking. Requires the `oidc` Cargo feature (enabled by default). |
| Device authorization | CLI login | RFC 8628-style flow: device code, browser approval, API token issuance. Status tracked via `DeviceAuthStatus` enum (`pending`, `authorized`, `expired`). |
| JWT access tokens | API requests | Short-lived tokens that carry resolved permissions (never stored). |
| Refresh tokens | API requests | SHA-256 hashed, 7-day expiry, rotated on each use within a DB transaction, revoking the predecessor. |
| API tokens | Programmatic access | Long-lived, revocable bearer tokens stored in the database. |
| mTLS client certs | Agent/MQTT connections | Issued after CSR approval and validated per connection. |
| Forwarded cert headers | Reverse proxy | Trusted proxies forward cert info/PEM; issuer CN verified. |
| Enrollment tokens | Agent onboarding | One-time tokens with optional expiry/use limit. |
| MQTT enrollment tokens | MQTT service enrollment | Stored separately (`mqtt_enrollment.token_hash`). |

## Permissions Model - Detailed

Authorization uses a typed `Permission` enum (defined in `crates/shared/web-api-types/src/permissions.rs`, re-exported
from `crates/ui/web-api/src/auth/permissions.rs`) rather than raw role-name strings. The enum variants are:

| Permission | Serialized name | Purpose |
| --- | --- | --- |
| `ViewSettings` | `view_settings` | Read settings, OIDC providers, auth config |
| `ManageSettings` | `manage_settings` | Modify settings, OIDC providers, auth config |
| `ViewAgents` | `view_agents` | List agents |
| `ManageAgents` | `manage_agents` | Approve, reject, delete, merge agents; manage enrollment tokens |
| `ManageGlobalSettings` | `manage_global_settings` | View and modify global settings (network, CA, TLS, system alerts) |

### Roles

| Role | Permissions |
| --- | --- |
| `owner` | All five (`view_settings`, `manage_settings`, `view_agents`, `manage_agents`, `manage_global_settings`) |
| `admin` | `view_settings`, `manage_settings`, `view_agents`, `manage_agents` |
| `user` | `view_agents` only |

The first registered user gets the `owner` role — whether registered via password or OIDC. Subsequent users (password or
OIDC auto-created) get the `user` role by default. OIDC role mapping can override this.

### How it works

1. `get_user_permissions()` (`routes/auth.rs`) resolves a user's permissions: user → user_roles → role_permissions →
   permissions table.
1. The resolved `Vec<Permission>` is embedded in the JWT access token (`permissions` claim) and returned in
   `UserResponse.permissions`.
1. The `require_auth` middleware injects `AuthenticatedUser` with the `permissions` field decoded from the JWT.
1. Route handlers call `user.has_permission(Permission::...)` — no DB round-trip needed.
1. The frontend receives permissions as `string[]` (e.g. `["view_settings", "manage_agents"]`) and uses the `Permission`
   TypeScript enum for checks.

### Adding a new permission

1. Add a variant to the `Permission` enum in `crates/shared/web-api-types/src/permissions.rs` (with `as_str` / `parse`
   arms).
1. Write a DB migration to insert it into the `permissions` table and assign it to the appropriate roles.
1. Add the check in the relevant route handler(s).
1. Add the variant to the `Permission` TypeScript enum in `frontend/src/lib/types.ts`.
