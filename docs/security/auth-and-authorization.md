# Authentication and Authorization

| Method | Scope | Details |
| --- | --- | --- |
| Password (Argon2id) | User login | Local accounts with hashed passwords. |
| OIDC | User login | External identity providers with auto-create or account linking. Requires the `oidc` Cargo feature (enabled by default). |
| Device authorization | CLI login | RFC 8628-style flow: device code, browser approval, API token issuance. Status tracked via `DeviceAuthStatus` enum (`pending`, `authorized`, `expired`). |
| JWT access tokens | API requests | Short-lived tokens that carry resolved permissions (never stored). |
| Refresh tokens | API requests | SHA-256 hashed, 7-day expiry, rotated on each use within a DB transaction, revoking the predecessor. Session integrity validated on every use (see below). |
| API tokens | Programmatic access | Long-lived, revocable bearer tokens stored in the database. |
| mTLS client certs | Agent/MQTT connections | Issued after CSR approval and validated per connection. |
| Forwarded cert headers | Reverse proxy | Trusted proxies forward cert info/PEM; issuer CN verified. |
| Enrollment tokens | Service onboarding | Multiple named tokens stored in the `enrollment_tokens` table (Argon2id hashed). Each token supports capability scoping, usage limits, and TTL. See [Enrollment Tokens API](../api/enrollment-tokens.md). |

## JWT Access Token Claims Contract

Every access token minted by `JwtManager::create_access_token` includes:

| Claim | Value | Purpose |
| --- | --- | --- |
| `iss` | `"uptrakit"` | Identifies the issuing deployment. |
| `aud` | `["uptrakit"]` | Restricts token acceptance to Uptrakit instances. |
| `sub` | User UUID | Identifies the subject user. |
| `exp` | Unix timestamp | Token expiry (15 minutes from issuance). |
| `jti` | UUID | Per-token unique identifier used for denylist lookups. |
| `permissions` | `string[]` | Resolved permissions embedded at issuance time. |
| `auth_method` | `"password"` \| `"oidc"` \| `"api_token"` | How the user authenticated. |

`decode_access_token` validates **all three** of `exp`, `iss`, and `aud`. Tokens that lack any of
these claims, or that carry the wrong values (e.g. tokens issued by a different deployment sharing the
same signing key), are rejected with `AuthError::JwtDecode`. This prevents cross-deployment token
replay attacks in scenarios where the signing key file is accidentally shared or restored from backup.

## JWT Signing Key Storage

The JWT signing key is stored in the `settings` table under `auth.jwt_signing_key` (global scope, base64
encoded) and is encrypted at rest using AES-256-GCM via `encrypt_str()` — the same algorithm used for all
other sensitive fields (`EncryptedString`). On every read the value is decrypted transparently before use.

The JWT key uses the context-bound `ENC:v2:` format with AAD `"uptrakit:settings:jwt_signing_key"`,
preventing this ciphertext from being reused in any other encrypted column even if the master key is
compromised and an attacker attempts a relocation attack.

Legacy unencrypted keys (base64 only, written before encryption was introduced) are transparently
re-encrypted on the next read using `ENC:v2:`. Legacy `ENC:v1:` keys are accepted during decryption
for backward compatibility with existing installations. No operator intervention is required.

See [Secrets Handling and Encryption](secrets-and-encryption.md) for the encryption format and master key
requirements.

## Session Integrity Validation

Refresh token verification and rotation validate session data integrity before proceeding:

- **OIDC sessions** must have a valid `oidc_provider_id`. If the provider ID is missing or corrupt
  (e.g., `auth_method = 'oidc'` but `oidc_provider_id IS NULL`), the session is rejected with
  `AuthError::InvalidSession` and a warning is logged. The session is **never** silently downgraded
  to password authentication.
- **JWT access tokens** with `auth_method = "oidc"` are similarly rejected if `oidc_provider_id`
  is missing or unparseable, returning HTTP 401 instead of falling back to a different auth method.
- **Database constraint**: The sessions table enforces
  `CHECK(auth_method != 'oidc' OR oidc_provider_id IS NOT NULL)` to prevent invalid state at the
  storage layer.

See also: [Secrets Handling and Encryption](secrets-and-encryption.md) for encryption-at-rest details.

## OIDC Email Verification Enforcement

`resolve_oidc_user` checks the `email_verified` claim from the OIDC ID token before performing any database
lookup. This prevents account creation or matching for addresses that the identity provider has not confirmed.

| `email_verified` claim | Behavior |
| --- | --- |
| `true` | Accepted — proceeds to user resolution |
| `false` | **Rejected** — returns `OidcUserResolution::EmailNotVerified`; user is redirected to the login page with `error=email_not_verified` |
| absent / `null` | Accepted conservatively — many legitimate providers (GitHub, Google) omit the claim for confirmed accounts |

The check occurs at the entry point of `resolve_oidc_user` before any DB query, ensuring no account is created
or linked for an unverified email regardless of `auto_create` or role-mapping configuration.

See also: [`docs/development/coding-standards.md`](../development/coding-standards.md) for the security guard
placement convention.

## Database Error Propagation in Auth Handlers

Database errors in authentication and authorization handlers must always propagate as HTTP 500
Internal Server Error. **Silently defaulting on DB failure is a security defect, not a
graceful fallback.**

### Why defaults are dangerous

| Site | Anti-pattern | Effect |
| --- | --- | --- |
| `require_auth.rs` — load user permissions | `.unwrap_or_default()` | DB outage → empty permission set → 403 Forbidden; legitimate requests blocked silently |
| `oidc_auth.rs` — count existing users | `.unwrap_or(false)` | DB outage → assume zero users → unintended first-admin OIDC registration allowed |
| `oidc_auth.rs` — list OIDC providers | `.unwrap_or_default()` | DB outage → empty provider list → correct behavior obscured, outage masked |

### Required pattern

All auth-path DB queries must be propagated with `?` after mapping to an appropriate error:

```rust
// ✓ Correct — DB outage surfaces as 500; access never silently granted or denied
let permissions = get_user_permissions(db, user_id)
    .await
    .map_err(|e| {
        tracing::error!(err = %e, user_id = %user_id, "failed to load user permissions");
        AuthFailure::InternalError
    })?;
```

See [Error Handling — Pattern 20](../development/error-handling.md#pattern-20-db-errors-in-authentication-and-authorization-handlers)
for the full pattern with examples.

## OIDC Link Token URL Exposure

**Risk level:** Low (accepted)

When OIDC account linking is required the backend redirects to
`/login?link_required=true&link_token=<token>&email=<email>`. The `link_token` is therefore
visible in:

- The browser address bar
- Browser history
- Server access logs

### Mitigations in place

1. **Single-use, short-lived**: the token is consumed atomically from
   `pending_account_links` on first use and expires server-side within a short window.
2. **Same-origin redirect**: the redirect is to the same origin (`/login`), so there
   is no cross-origin referrer leakage.
3. **`Referrer-Policy: no-referrer`** is set on the redirect response in
   `crates/ui/web-api/src/routes/oidc_auth.rs` so the token URL is not forwarded in
   `Referer` headers when the browser subsequently loads third-party resources.
4. **User already authenticated**: the token only exists after the user has successfully
   completed OIDC authentication; it does not grant initial access.

### Residual risk

The token remains in browser history and server access logs for the lifetime of those
logs. Operators should ensure access-log retention policies are appropriate for their
environment.

## Permissions Model - Detailed

Authorization uses a typed `Permission` enum (defined in `crates/shared/web-api-types/src/permissions.rs`, re-exported
from `crates/ui/web-api/src/auth/permissions.rs`) rather than raw role-name strings. The enum variants are:

| Permission | Serialized name | Purpose |
| --- | --- | --- |
| `ViewSettings` | `view_settings` | Read settings, OIDC providers, auth config |
| `ManageSettings` | `manage_settings` | Modify settings, OIDC providers, auth config |
| `ViewAgents` | `view_agents` | List agents/services |
| `ManageAgents` | `manage_agents` | Approve, reject, delete, merge agents; manage enrollment tokens |
| `ManageGlobalSettings` | `manage_global_settings` | View and modify global settings (network, CA, TLS, system alerts) |
| `ViewSoftware` | `view_software` | View software items, plugin configs, and update history |
| `ManageSoftware` | `manage_software` | Manage software items, version checks, update scheduling, and non-command plugin config fields |
| `ManageCommands` | `manage_commands` | Create and modify plugin configs containing command-bearing fields (shell commands, `post_pull_command`, custom hook `commands` arrays). **Treat as equivalent to root access on all managed hosts.** |
| `ViewHosts` | `view_hosts` | View hosts |
| `ManageHosts` | `manage_hosts` | Manage hosts (update, deactivate) |

> **Security note:** `ManageCommands` grants effective code-execution authority on all managed hosts
> assigned to the affected software items. Users with this permission can configure arbitrary shell
> commands that execute on managed hosts. Assign with the same care as granting `root` access.

### Roles

| Role | Permissions |
| --- | --- |
| `owner` | All permissions (including `manage_commands`) |
| `admin` | All except `manage_global_settings` (including `manage_commands`) |
| `user` | `view_settings`, `view_agents`, `view_software`, `view_hosts` |

The first registered user gets the `owner` role — whether registered via password or OIDC. Subsequent users (password or
OIDC auto-created) get the `user` role by default. OIDC role mapping can override this.

### How it works

1. `get_user_permissions()` (`routes/auth.rs`) resolves a user's permissions: user → user_roles → role_permissions →
   permissions table.
1. The resolved `Vec<Permission>` is embedded in the JWT access token (`permissions` claim) and returned in
   `UserResponse.permissions`.
1. The `require_auth` middleware injects `AuthenticatedUser` with the `permissions` field decoded from the JWT.
1. Route handlers declare their permission requirement via a **typed Axum extractor** (e.g.
   `CanViewHosts(_user): CanViewHosts`). The extractor is defined in
   `crates/ui/web-api/src/middleware/permission.rs` using a macro that generates one concrete struct per permission.
   If the user lacks the permission the extractor short-circuits with `403 Forbidden` before the handler body runs.
   No DB round-trip is needed.
1. Every protected endpoint also carries an `x-required-permission` OpenAPI extension (set in the `#[utoipa::path]`
   annotation, e.g. `extensions(("x-required-permission" = json!("view_hosts")))`). This makes the required
   permission machine-readable in the generated OpenAPI spec.
1. The frontend receives permissions as `string[]` (e.g. `["view_settings", "manage_agents"]`) and uses the `Permission`
   TypeScript enum for checks.

### Self-authenticated endpoints and the `"self"` sentinel

Some endpoints — `create_api_token`, `list_api_tokens`, `revoke_api_token`, `logout`, and `me` — are
authenticated (require a valid Bearer token) but not governed by the RBAC permission model. Any authenticated
user may call them regardless of their assigned roles. These endpoints use `Extension<AuthenticatedUser>`
directly rather than a typed permission extractor, and carry:

```rust
extensions(("x-required-permission" = json!("self")))
```

The sentinel value `"self"` is distinct from the named `Permission` variants. It signals to automated
permission-audit tooling that the endpoint requires only **authentication** (a valid token), not any specific
RBAC permission. Tools must treat `"self"` as "any authenticated user is authorized".

### Permission extractor reference

| Extractor | Permission checked |
| --- | --- |
| `CanViewSettings` | `Permission::ViewSettings` |
| `CanManageSettings` | `Permission::ManageSettings` |
| `CanViewAgents` | `Permission::ViewAgents` |
| `CanManageAgents` | `Permission::ManageAgents` |
| `CanManageGlobalSettings` | `Permission::ManageGlobalSettings` |
| `CanViewSoftware` | `Permission::ViewSoftware` |
| `CanManageSoftware` | `Permission::ManageSoftware` |
| `CanManageCommands` | `Permission::ManageCommands` |
| `CanViewHosts` | `Permission::ViewHosts` |
| `CanManageHosts` | `Permission::ManageHosts` |

All extractors derive `Debug`, expose `pub AuthenticatedUser` as field 0 for handler use, and provide a
`::new(user)` constructor for use in unit tests that call handlers directly (bypassing the HTTP layer).

See also: [`docs/development/coding-standards.md`](../development/coding-standards.md) for the permission pattern conventions.

### Adding a new permission

1. Add a variant to the `Permission` enum in `crates/shared/web-api-types/src/permissions.rs` (with `as_str` / `parse`
   arms).
1. Write a DB migration to insert it into the `permissions` table and assign it explicitly to the appropriate
   roles by name: `owner` always gets all permissions; `admin` gets all except `manage_global_settings`-level
   permissions; `user` gets all `view_*` permissions.
1. Add a `CanXxx => Permission::Xxx` entry to the `permission_extractor!` macro call in
   `crates/ui/web-api/src/middleware/permission.rs`.
1. Use `CanXxx(_user): CanXxx` (or `CanXxx(user): CanXxx` if you need the user) in the relevant route handler(s),
   and add `extensions(("x-required-permission" = json!("xxx")))` to the corresponding `#[utoipa::path]` annotation.
1. Add the variant to the `Permission` TypeScript enum in `frontend/src/lib/types.ts`.

## WebSocket Enrollment Secret Lookup

Services connect to the controller WebSocket endpoint at `/api/v1/ws/service`. Three authentication
paths exist:

| Path | When | How |
| --- | --- | --- |
| mTLS | Post-enrollment | Client certificate issued after CSR approval; identity is cryptographically tied to the certificate serial number and service ID. |
| Bearer token | Enrollment window | `Authorization: Bearer <enrollment_secret>` header; secret is SHA-256 hashed and compared against the database. |
| Anonymous | Pre-enrollment | No credentials; only `enroll` messages are accepted. |

### Bearer token lookup and the `service_id` query parameter

During the enrollment window (between `save_enrollment()` completing and the CA issuing and
delivering the certificate), services authenticate via their enrollment secret. The controller
resolves the service by querying:

```sql
SELECT * FROM services
WHERE enrollment_secret_hash = $1
  AND deactivated_at IS NULL
  -- optionally:
  AND id = $2
```

As a defence-in-depth measure, the service appends its known `service_id` as a URL query parameter:

```text
wss://controller:3000/api/v1/ws/service?service_id=<uuid>
```

When `service_id` is present, the DB query is narrowed to that specific service. If the secret hash
matches a different service's row (a practically impossible collision for 256-bit random secrets, but
architecturally undesirable), the controller returns `InvalidSecret` — the same error as no match —
so the caller cannot tell whether a collision occurred.

**The `service_id` filter is enforced by the service-sdk.** `connect_ws()` in
`crates/shared/service-sdk/src/ws.rs` appends `?service_id=<uuid>` to the WebSocket URL whenever
the local identity file contains a service ID. During the first enrollment (no identity yet) the
parameter is omitted and the lookup falls back to hash-only matching.

**mTLS connections do not use the `service_id` parameter.** Their identity is embedded in the client
certificate; no bearer secret lookup is performed.

See also: [Wire Protocol](../api/wire-protocol.md) for connection sequencing.

## Content Security Policy

The admin UI's Content Security Policy (set in `frontend/src/app.html`) includes
`img-src 'self' https:`. This allows images from any HTTPS domain, which is required
to load OIDC provider logos configured by administrators.

**Accepted risk:** The admin who configures the OIDC provider logo URL is a trusted user.
Logo URLs are validated as HTTPS-only via `isValidLogoUrl()` in `frontend/src/lib/utils.ts`
before display. `referrerpolicy="no-referrer"` is applied to logo `<img>` elements,
preventing the Uptrakit URL from leaking to logo hosts via the `Referer` header.
