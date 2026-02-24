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
| Enrollment tokens | Agent onboarding | One-time tokens with optional expiry/use limit. |
| MQTT enrollment tokens | MQTT service enrollment | Stored separately (`mqtt_enrollment.token_hash`). |

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
| `ViewSoftware` | `view_software` | View software items, provider configs, and update history |
| `ManageSoftware` | `manage_software` | Manage software items, provider configs, version checks, updates, and scheduler |
| `ViewHosts` | `view_hosts` | View hosts |
| `ManageHosts` | `manage_hosts` | Manage hosts (update, deactivate) |

### Roles

| Role | Permissions |
| --- | --- |
| `owner` | All nine permissions |
| `admin` | `view_settings`, `manage_settings`, `view_agents`, `manage_agents`, `view_software`, `manage_software`, `view_hosts`, `manage_hosts` |
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

## Content Security Policy

The admin UI's Content Security Policy (set in `frontend/src/app.html`) includes
`img-src 'self' https:`. This allows images from any HTTPS domain, which is required
to load OIDC provider logos configured by administrators.

**Accepted risk:** The admin who configures the OIDC provider logo URL is a trusted user.
Logo URLs are validated as HTTPS-only via `isValidLogoUrl()` in `frontend/src/lib/utils.ts`
before display. `referrerpolicy="no-referrer"` is applied to logo `<img>` elements,
preventing the Uptrakit URL from leaking to logo hosts via the `Referer` header.
