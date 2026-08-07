---
title: Authentication and Authorization
weight: 40
description: Authentication methods, JWT access token claims, role and permission model, and auth middleware behavior in Uptrakit.
---

# Authentication and Authorization

| Method                   | Scope                     | Details                                                                                                                                                                                                                                          |
| ------------------------ | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Password (Argon2id)      | User login                | Local accounts with hashed passwords.                                                                                                                                                                                                            |
| OIDC                     | User login                | External identity providers with auto-create or account linking. Requires the `oidc` Cargo feature (enabled by default).                                                                                                                         |
| Device authorization     | CLI login                 | RFC 8628-style flow: device code, browser approval, API token issuance. Status tracked via `DeviceAuthStatus` enum (`pending`, `authorized`, `expired`).                                                                                         |
| JWT access tokens        | API requests              | Short-lived tokens that carry no authorization data; the `AccessEngine` resolves authority per request (never stored).                                                                                                                           |
| Refresh tokens           | API requests              | SHA-256 hashed, 7-day expiry, rotated on each use within a DB transaction, revoking the predecessor. Session integrity validated on every use (see below).                                                                                       |
| API tokens               | Programmatic access       | Long-lived, revocable bearer tokens stored in the database.                                                                                                                                                                                      |
| mTLS client certs        | Agent/MQTT connections    | Issued after CSR approval and validated per connection.                                                                                                                                                                                          |
| Forwarded cert headers   | Reverse proxy             | Trusted proxies forward cert info/PEM; issuer CN verified.                                                                                                                                                                                       |
| Enrollment tokens        | Service onboarding        | Multiple named tokens stored in the `enrollment_tokens` table (Argon2id hashed). Each token supports capability scoping, usage limits, and TTL. See [Enrollment Tokens API](https://github.com/worried-networking/uptrakit/tree/main/docs/api/). |
| System enrollment tokens | System service onboarding | Global (non-tenant) named tokens stored in the `system_enrollment_tokens` table (Argon2id hashed). Supports usage limits and TTL. See [System Enrollment Tokens API](https://github.com/worried-networking/uptrakit/tree/main/docs/api/).        |

## JWT Access Token Claims Contract

Every access token minted by `JwtManager::create_access_token` includes:

| Claim         | Value                                     | Purpose                                                |
| ------------- | ----------------------------------------- | ------------------------------------------------------ |
| `iss`         | `"uptrakit"`                              | Identifies the issuing deployment.                     |
| `aud`         | `["uptrakit"]`                            | Restricts token acceptance to Uptrakit instances.      |
| `sub`         | User UUID                                 | Identifies the subject user.                           |
| `exp`         | Unix timestamp                            | Token expiry (15 minutes from issuance).               |
| `jti`         | UUID                                      | Per-token unique identifier used for denylist lookups. |
| `auth_method` | `"password"` \| `"oidc"` \| `"api_token"` | How the user authenticated.                            |

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
- **Expiry cleanup**: The scheduler's `AuthCleanupExecutor` purges expired `sessions` rows
  (`expires_at < now`, no grace window) alongside other expired auth state, so stale session rows do
  not accumulate unbounded.

See also: [Secrets Handling and Encryption](secrets-and-encryption.md) for encryption-at-rest details.

## OIDC Email Verification Enforcement

`resolve_oidc_user` checks the `email_verified` claim from the OIDC ID token before performing any database
lookup. This prevents account creation or matching for addresses that the identity provider has not confirmed.

| `email_verified` claim | Behavior                                                                                                                                                                                                                   |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `true`                 | Accepted — proceeds to user resolution                                                                                                                                                                                     |
| `false`                | **Rejected** — returns `OidcUserResolution::EmailNotVerified`; user is redirected to the login page with `error=email_not_verified`                                                                                        |
| absent / `null`        | **Rejected** — treated the same as `false`. A rogue IdP that omits the claim cannot bypass verification. Providers that omit the claim for confirmed accounts must be configured to always include `email_verified: true`. |

The check occurs at the entry point of `resolve_oidc_user` before any DB query, ensuring no account is created
or linked for an unverified email regardless of `auto_create` or role-mapping configuration.

See also: [`docs/development/coding-standards.md`](https://github.com/worried-networking/uptrakit/tree/main/docs/development/) for the security guard
placement convention.

## Database Error Propagation in Auth Handlers

Database errors in authentication and authorization handlers must always propagate as HTTP 500
Internal Server Error. **Silently defaulting on DB failure is a security defect, not a
graceful fallback.**

### Why defaults are dangerous

| Site                                  | Anti-pattern           | Effect                                                                           |
| ------------------------------------- | ---------------------- | -------------------------------------------------------------------------------- |
| `oidc_auth.rs` — count existing users | `.unwrap_or(false)`    | DB outage → assume zero users → unintended first-admin OIDC registration allowed |
| `oidc_auth.rs` — list OIDC providers  | `.unwrap_or_default()` | DB outage → empty provider list → correct behavior obscured, outage masked       |

`users.rs`'s `build_user_response` (the site formerly listed above) now propagates role-lookup failures with `?`
instead of defaulting; it mints no token and the response carries no permission/action data, so a DB outage there
surfaces as an error rather than a silently degraded response.

`register`, `login`, `refresh`, the OIDC mint, `me`, and the post-MFA/2FA session builders all resolve their
response's `actions` list through `effective_actions()` / `AccessEngine::allowed_actions()` (see [Transition:
action extractors](#transition-action-extractors-m14am14b)). None of them propagate a 500 for an `AccessEngine`
failure on this path: the response still returns its normal success status with `actions: []` and
`authority: "unavailable"`. This is deliberate and uniform across the family — the SPA treats any non-2xx from `me`
as an unconditional logout, so a 500 there would eject an already-logged-in user on a transient DB blip, and the
same carve-out now covers the other mint paths so a blip degrades authority instead of failing the request.

### Required pattern

All auth-path DB queries must be propagated with `?` after mapping to an appropriate error:

```rust
// ✓ Correct — DB outage surfaces as 500; access never silently granted or denied
let roles = get_user_role_summaries(state, user_id)
    .await
    .map_err(|e| {
        tracing::error!(err = %e, user_id = %user_id, "failed to load user roles");
        AuthFailure::InternalError
    })?;
```

See [Error Handling — Pattern 20](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
for the full pattern with examples.

## OIDC Link Token URL Handling

**Risk level:** Low

When OIDC account linking is required the backend redirects to
`/login?link_required=true&email=<email>#link_token=<token>`. The sensitive `link_token`
is placed in the URL fragment instead of the query string, so it is not sent to the server
and does not appear in controller access logs or `Referer` headers.

### Mitigations in place

1. **Single-use, short-lived**: the token is consumed atomically from
   `pending_account_links` on first use and expires server-side within a short window.
2. **Fragment transport**: the browser keeps the token client-side in the fragment, removing
   it from server-side request logs and request targets.
3. **Same-origin redirect**: the redirect is to the same origin (`/login`), so there
   is no cross-origin referrer leakage.
4. **`Referrer-Policy: no-referrer`** is set on the redirect response in
   `crates/ui/web-api/src/routes/oidc_auth.rs` so the token URL is not forwarded in
   `Referer` headers when the browser subsequently loads third-party resources.
5. **User already authenticated**: the token only exists after the user has successfully
   completed OIDC authentication; it does not grant initial access.

### Residual risk

The token still appears in the browser address bar and browser history until the user
leaves or rewrites the page URL. Fragment transport removes server-side logging
exposure, but a compromised or observed client can still capture the token during its
validity window.

## Permissions Model - Detailed

Authorization uses a typed `Permission` enum (defined in `crates/shared/types/src/permissions.rs`, re-exported
via `crates/shared/web-api-types/src/permissions.rs`) rather than raw role-name strings. There are 33 granular
permissions organized by domain:

> **Historical model.** The `permission_extractor!` mechanism this section documents
> (`crates/ui/web-api/src/middleware/permission.rs`) was deleted in M1.7; no route family enforces through it any
> more, and the JWT/response fields it relied on (the `permissions` claim, `UserResponse.permissions`) no longer
> exist. The `Permission` enum and its backing tables are removed in M1.8. See [Transition: action
> extractors](#transition-action-extractors-m14am14b) below for the live mechanism. The reference tables in this
> section remain for migration/historical context.

### Permissions reference

#### Services

| Permission        | Serialized name    | Purpose                                                |
| ----------------- | ------------------ | ------------------------------------------------------ |
| `ViewServices`    | `view_services`    | View tenant services and their status                  |
| `ApproveServices` | `approve_services` | Approve pending service enrollments                    |
| `RejectServices`  | `reject_services`  | Reject pending service enrollments                     |
| `RemoveServices`  | `remove_services`  | Deactivate/remove services                             |
| `UpdateServices`  | `update_services`  | Update service settings (ping interval, freeze, merge) |

#### System services

| Permission              | Serialized name           | Purpose                                                |
| ----------------------- | ------------------------- | ------------------------------------------------------ |
| `ViewSystemServices`    | `view_system_services`    | View system services (MQTT bridge, external scheduler) |
| `ApproveSystemServices` | `approve_system_services` | Approve pending system services                        |
| `RejectSystemServices`  | `reject_system_services`  | Reject pending system services                         |
| `RemoveSystemServices`  | `remove_system_services`  | Deactivate system services                             |
| `UpdateSystemServices`  | `update_system_services`  | Update system service settings                         |

#### Software

| Permission        | Serialized name    | Purpose                                      |
| ----------------- | ------------------ | -------------------------------------------- |
| `ViewSoftware`    | `view_software`    | View software items, plugin configs, history |
| `CreateSoftware`  | `create_software`  | Create software items and plugin configs     |
| `UpdateSoftware`  | `update_software`  | Edit software items and plugin configs       |
| `DeleteSoftware`  | `delete_software`  | Delete software items and plugin configs     |
| `TriggerChecks`   | `trigger_checks`   | Trigger version checks and autodiscovery     |
| `TriggerUpdates`  | `trigger_updates`  | Trigger update execution (single and batch)  |
| `ManageScheduler` | `manage_scheduler` | Manage scheduled tasks                       |

#### Hosts

| Permission        | Serialized name    | Purpose                         |
| ----------------- | ------------------ | ------------------------------- |
| `ViewHosts`       | `view_hosts`       | View hosts                      |
| `UpdateHosts`     | `update_hosts`     | Update host properties and tags |
| `DeactivateHosts` | `deactivate_hosts` | Deactivate hosts                |

#### Settings

| Permission               | Serialized name            | Purpose                                             |
| ------------------------ | -------------------------- | --------------------------------------------------- |
| `ViewSettings`           | `view_settings`            | View all tenant settings (unified read)             |
| `ManageAuthSettings`     | `manage_auth_settings`     | Manage registration, authentication, OIDC providers |
| `ManageEnrollmentTokens` | `manage_enrollment_tokens` | Manage tenant enrollment tokens                     |
| `ManageAgentCerts`       | `manage_agent_certs`       | Manage agent certificate settings                   |
| `ManageGlobalSettings`   | `manage_global_settings`   | Manage global infrastructure settings               |

#### Commands

| Permission          | Serialized name       | Purpose                                                                |
| ------------------- | --------------------- | ---------------------------------------------------------------------- |
| `ManageCommands`    | `manage_commands`     | Modify command-bearing plugin config fields (code execution authority) |
| `TestPluginConfigs` | `test_plugin_configs` | Test plugin configurations against hosts (dry-run validation)          |

> [!CAUTION]
> `ManageCommands` grants effective code-execution authority on all managed hosts
> assigned to the affected software items. Users with this permission can configure arbitrary shell
> commands that execute on managed hosts. Assign with the same care as granting `root` access.

#### Notifications

| Permission            | Serialized name        | Purpose                                                      |
| --------------------- | ---------------------- | ------------------------------------------------------------ |
| `ViewNotifications`   | `view_notifications`   | View notification channels, rules, log                       |
| `ManageNotifications` | `manage_notifications` | Create/modify notification channels and rules; SMTP settings |

#### Audit logs

| Permission            | Serialized name          | Purpose                              |
| --------------------- | ------------------------ | ------------------------------------ |
| `ViewAuditLogs`       | `view_audit_logs`        | View tenant-scoped audit log entries |
| `ViewSystemAuditLogs` | `view_system_audit_logs` | View system-level audit log entries  |

#### User management

| Permission    | Serialized name | Purpose                      |
| ------------- | --------------- | ---------------------------- |
| `ManageUsers` | `manage_users`  | Manage user roles and access |

#### Autodiscovery

| Permission      | Serialized name  | Purpose                           |
| --------------- | ---------------- | --------------------------------- |
| `ManageIgnores` | `manage_ignores` | Manage autodiscovery ignore rules |

### Built-in roles

Eight built-in roles group permissions into logical responsibilities:

| Role                   | Permissions                                                                                                                                                                         |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `viewer`               | `view_services`, `view_software`, `view_hosts`, `view_settings`                                                                                                                     |
| `operator`             | `approve_services`, `reject_services`, `trigger_checks`, `trigger_updates`                                                                                                          |
| `service_manager`      | `approve_services`, `reject_services`, `remove_services`, `update_services`                                                                                                         |
| `software_manager`     | `create_software`, `update_software`, `delete_software`, `trigger_checks`, `trigger_updates`, `manage_scheduler`, `manage_ignores`, `test_plugin_configs`                           |
| `host_manager`         | `update_hosts`, `deactivate_hosts`                                                                                                                                                  |
| `settings_manager`     | `manage_auth_settings`, `manage_enrollment_tokens`, `manage_agent_certs`, `view_notifications`, `manage_notifications`, `view_audit_logs`, `manage_users`                           |
| `command_manager`      | `manage_commands`, `test_plugin_configs`                                                                                                                                            |
| `system_administrator` | `manage_global_settings`, `view_system_services`, `approve_system_services`, `reject_system_services`, `remove_system_services`, `update_system_services`, `view_system_audit_logs` |

Built-in roles are marked with `is_built_in = true` in the `roles` table.

### Role bundles

Role bundles are code-defined (not stored in the database) and group one or more roles under a
single name. They are exposed as advisory metadata via `GET /api/v1/access/catalog`; apply one to
a user by assigning its roles through `PUT /api/v1/users/{id}/roles`.

| Bundle          | Roles assigned                                                                                         | Use case                        |
| --------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------- |
| `read_only`     | `viewer`                                                                                               | Dashboard viewers, stakeholders |
| `operator`      | `viewer`, `operator`                                                                                   | On-call staff                   |
| `manager`       | `viewer`, `service_manager`, `software_manager`, `host_manager`                                        | Team leads                      |
| `administrator` | `viewer`, `service_manager`, `software_manager`, `host_manager`, `settings_manager`, `command_manager` | Tenant administrators           |
| `owner`         | All 8 roles                                                                                            | System owner                    |

See [User Management API](https://github.com/worried-networking/uptrakit/tree/main/docs/api/) for the full endpoint reference and
[User Management Guide](../end-user/user-management.md) for the end-user documentation.

### First user setup

The first registered user -- whether via password or OIDC -- receives all 8 built-in roles
(equivalent to the `owner` bundle). Subsequent users receive only the `viewer` role by default.
OIDC role mapping can override this.

### Lockout prevention

The system prevents removing the last remaining `access:manage` or `system.access:manage` holder.
Attempts to change roles, or to deactivate a user, in a way that would leave no user holding
either action are rejected with HTTP 409 Conflict (reason codes `lockout_access_manage` /
`lockout_system_access` -- see [Access Management API](../api/access-management.md#lockout-409-semantics)).

### How it works

How the now-deleted legacy model worked, retained for historical/migration context (see the historical-model note
above):

1. `get_user_permissions()` (`middleware/require_auth.rs`, deleted in M1.7) resolved a user's permissions: user ->
   user_roles -> role_permissions -> permissions table.
1. The resolved `Vec<Permission>` was embedded in the JWT access token (`permissions` claim, removed in M1.7) and
   returned in `UserResponse.permissions` (replaced by `actions` + `authority`, see [JWT Access Token Claims
   Contract](#jwt-access-token-claims-contract) above).
1. The `require_auth` middleware used to inject `AuthenticatedUser` with a `permissions` field decoded from the
   JWT; `AuthenticatedUser` carries no such field today.
1. Route handlers declared their permission requirement via a **typed Axum extractor** (e.g.
   `CanViewHosts(_user): CanViewHosts`), generated by a macro in the now-deleted `middleware/permission.rs`. If
   the user lacked the permission the extractor short-circuited with `403 Forbidden` before the handler body ran.
   No DB round-trip was needed.
1. Endpoints on the legacy model carried an `x-required-permission` OpenAPI extension (set in the
   `#[utoipa::path]` annotation, e.g. `extensions(("x-required-permission" = json!("view_hosts")))`).
   This made the required permission machine-readable in the generated OpenAPI spec.
1. The frontend received permissions as `string[]` (e.g. `["view_settings", "view_services"]`) and used a
   `Permission` TypeScript enum for checks; it now reads `actions: string[]` + `authority` from the `me`/login
   response and checks membership directly against the branded action strings.

### Self-authenticated endpoints

Some endpoints -- `create_api_token`, `list_api_tokens`, `revoke_api_token`, `logout`, and `me` -- are
authenticated (require a valid Bearer token) but not governed by the RBAC permission model. Any authenticated
user may call them regardless of their assigned roles. These endpoints use `Extension<AuthenticatedUser>`
directly rather than a typed permission extractor, and carry only the authenticated-only security declaration:

```rust
security(("oauth2" = []), ("developer_token" = []))
```

No permission or action scope is listed, and no `x-required-permission`/`x-action-dynamic` extension is present —
the empty scope list itself signals to automated permission-audit tooling that the endpoint requires only
**authentication** (a valid token), not any specific RBAC permission or catalog action.

### Runtime-valued permission extension (surfaces)

Shared-surface interaction routes (`crates/ui/web-api/src/routes/surfaces.rs`) are a second, distinct exception to
the typed-permission-extractor rule — separate from both the self-authenticated endpoints above and the
`// APPROVED: custom auth path` token-extraction exception used by handlers like the WebSocket upgrade route (see
[Coding Standards](../development/coding-standards.md)).

Surface descriptors and interactions carry their own `required_action: Option<String>` as **registration data**
supplied by the provider (plugin or service) — a canonical `resource:verb` catalog action string, not a value known
at route-definition time. `SurfaceProxy` parses each declared value to a catalog `Action` at registration admission
(an unparseable value rejects the whole registration); the registry stores the parsed `Action` index-aligned with
the normalized registration. No fixed `CanXxx` extractor can express "whatever action this particular
surface/interaction declares", so these handlers call `enforce_required_action()` in the handler body — running the
resolved `Action` through `AccessEngine` against the resolved descriptor/interaction — instead of a typed
extractor, and the `#[utoipa::path]` annotation declares the authenticated-only security form
(`security(("oauth2" = []), ("developer_token" = []))`) plus a boolean marker instead of a fixed scope list:

```rust
extensions(("x-action-dynamic" = json!(true)))
```

The `x-action-dynamic: true` extension tells automated tooling that this operation's OpenAPI security requirement is
intentionally authenticated-only — the real, enforced requirement is not statically expressible and lives in
registration data (the surface descriptor's or interaction's `required_action`), not in the spec.

This is checked at both the descriptor level and the interaction level, and — per [Shared Surface
Security](surfaces.md#permission-model) — happens for every method (`GET`/`POST`/`PUT`/`DELETE`) on the interaction
route family, before any `405` method-mismatch response, so a caller cannot fingerprint an interaction's registered
methods by comparing `403` against `405`.

| Exception class                 | Permission source                                     | OpenAPI marker                                 |
| ------------------------------- | ----------------------------------------------------- | ---------------------------------------------- |
| Self-authenticated endpoints    | None — any authenticated user is authorized           | none — empty-scope `security(...)` declaration |
| `// APPROVED: custom auth path` | Not RBAC — bespoke auth (token extraction, WebSocket) | handler-specific, documented inline            |
| Runtime-valued (surfaces)       | Registration data on the surface/interaction          | `x-action-dynamic: true`                       |

Automated permission-audit tooling must treat `x-action-dynamic: true` as a marker that the operation's declared
`security(...)` scopes are not the real, enforced requirement — a fixed catalog action cannot be extracted
statically from the spec for these operations — not a defect in the generated OpenAPI spec.

### Permission extractor reference

| Extractor                   | Permission checked                   |
| --------------------------- | ------------------------------------ |
| `CanViewSettings`           | `Permission::ViewSettings`           |
| `CanManageAuthSettings`     | `Permission::ManageAuthSettings`     |
| `CanManageEnrollmentTokens` | `Permission::ManageEnrollmentTokens` |
| `CanManageAgentCerts`       | `Permission::ManageAgentCerts`       |
| `CanManageGlobalSettings`   | `Permission::ManageGlobalSettings`   |
| `CanViewServices`           | `Permission::ViewServices`           |
| `CanApproveServices`        | `Permission::ApproveServices`        |
| `CanRejectServices`         | `Permission::RejectServices`         |
| `CanRemoveServices`         | `Permission::RemoveServices`         |
| `CanUpdateServices`         | `Permission::UpdateServices`         |
| `CanViewSoftware`           | `Permission::ViewSoftware`           |
| `CanCreateSoftware`         | `Permission::CreateSoftware`         |
| `CanUpdateSoftware`         | `Permission::UpdateSoftware`         |
| `CanDeleteSoftware`         | `Permission::DeleteSoftware`         |
| `CanTriggerChecks`          | `Permission::TriggerChecks`          |
| `CanTriggerUpdates`         | `Permission::TriggerUpdates`         |
| `CanManageScheduler`        | `Permission::ManageScheduler`        |
| `CanManageCommands`         | `Permission::ManageCommands`         |
| `CanTestPluginConfigs`      | `Permission::TestPluginConfigs`      |
| `CanViewHosts`              | `Permission::ViewHosts`              |
| `CanUpdateHosts`            | `Permission::UpdateHosts`            |
| `CanDeactivateHosts`        | `Permission::DeactivateHosts`        |
| `CanViewNotifications`      | `Permission::ViewNotifications`      |
| `CanManageNotifications`    | `Permission::ManageNotifications`    |
| `CanViewSystemServices`     | `Permission::ViewSystemServices`     |
| `CanApproveSystemServices`  | `Permission::ApproveSystemServices`  |
| `CanRejectSystemServices`   | `Permission::RejectSystemServices`   |
| `CanRemoveSystemServices`   | `Permission::RemoveSystemServices`   |
| `CanUpdateSystemServices`   | `Permission::UpdateSystemServices`   |
| `CanViewAuditLogs`          | `Permission::ViewAuditLogs`          |
| `CanViewSystemAuditLogs`    | `Permission::ViewSystemAuditLogs`    |
| `CanManageUsers`            | `Permission::ManageUsers`            |
| `CanManageIgnores`          | `Permission::ManageIgnores`          |

All extractors derive `Debug`, expose `pub AuthenticatedUser` as field 0 for handler use, and provide a
`::new(user)` constructor for use in unit tests that call handlers directly (bypassing the HTTP layer).

See also: [`docs/development/coding-standards.md`](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
for the permission pattern conventions.

### Adding a new permission

Legacy model only, retained for historical reference: no route family is on it any more. All
authorization work declares a catalog action and an `action_extractor!` type instead (see the next section).

1. Add a variant to the `Permission` enum in `crates/shared/types/src/permissions.rs` (with `as_str` / `from_str` /
   `description` arms).
1. Write a DB migration to insert it into the `permissions` table and assign it to the appropriate built-in
   role(s) using `grant_permission()`. Decide which role(s) should include the new permission based on the
   domain (e.g. software permissions go to `software_manager`, host permissions to `host_manager`).
1. Add a `CanXxx => Permission::Xxx` entry to the `permission_extractor!` macro call in
   `crates/ui/web-api/src/middleware/permission.rs`.
1. Use `CanXxx(_user): CanXxx` (or `CanXxx(user): CanXxx` if you need the user) in the relevant route handler(s),
   and add `extensions(("x-required-permission" = json!("xxx")))` to the corresponding `#[utoipa::path]` annotation.
1. Add the variant to the `Permission` TypeScript enum in `frontend/src/lib/types.ts`.

### Transition: action extractors (M1.4a/M1.4b)

This model was rolled out route family by route family and now backs every route family (see the completion
note at the end of this section). Converted families enforce through `action_extractor!`-generated types
(`crates/ui/web-api/src/middleware/action.rs`),
backed by the `AccessEngine` (`crates/ui/controller-core/src/access/mod.rs`) rather than the JWT's
embedded `permissions` claim — decisions reflect live DB grants on every request (immediate effect on
grant/revoke, no re-login required), and denial always returns a fixed, generic `403 Forbidden` body
(`"Insufficient permissions"`, no grant/selector detail). Their `#[utoipa::path]` annotation declares a
native OpenAPI security requirement, e.g. `security(("oauth2" = ["hosts:read"]), ("developer_token" = []))`,
instead of the `x-required-permission` extension. The `hosts` route family (`crates/ui/web-api/src/routes/hosts.rs`)
is the first converted family and serves as the reference conversion.

The M1.4b sweep (batches B1–B6) converted **all** route families except the `users.rs` / `roles.rs`
handoffs, which M1.6a/M1.6b finished. OR-of-alternatives operations (batch actions,
`list_plugin_types`, plugin-type-settings reads) declare one single-scope `oauth2` requirement per
alternative and enforce inline via `authorize_any`, with no action extractor. Dynamic surface wrappers
carry `x-action-dynamic: true` alongside the authenticated-only security form. The operator OAuth clients
API and the admin events SSE stream also enforce through action extractors (`settings.auth:manage`,
`services:read` respectively).

MCP authorization has moved onto the same `AccessEngine` in parallel with the route-family sweep: both MCP auth paths
(API token and OAuth JWT) build an `AccessContext` and gate the connection on the `mcp:use` action, and each MCP tool
declares typed catalog actions in its `ToolAuth` that a single `require_tool_auth()` helper enforces — see the
[OAuth MCP Development Guide](../development/oauth-mcp.md).

Shared surfaces enforce on the same engine: `required_action` on surface descriptors and interactions is parsed to a
catalog `Action` at registration admission and enforced by `enforce_required_action()` through `AccessEngine` before
dispatch, for both plugin- and service-backed surfaces; provider-origin (service-initiated) calls are denied for
action-gated interactions unless the interaction sets `provider_invocable`. See [Runtime-valued permission extension
(surfaces)](#runtime-valued-permission-extension-surfaces) above for the full mechanism — this paragraph only
cross-links it, the resolved-`Action` enforcement described there is the same one this transition covers.

The interactive update WebSocket (`crates/ui/web-api/src/routes/interactive_ws.rs`) gates on the `updates:trigger`
catalog action through `AccessEngine`, via `build_access_authority`. `require_auth` never runs on this route —
browser WebSockets cannot set custom headers, so the auth token arrives as a `?token=` query parameter instead — so
the check is an inline engine call rather than an extractor. A denial is a plain HTTP `403 Forbidden` returned
**before** `ws.on_upgrade()`; there is no close-frame handshake. `AccessEngine`/DB unavailability fails closed as an
HTTP `500` before upgrade, distinct from the `403` deny path.

Five further inline (non-extractor) sites gate on the engine directly. Four share an `authorize_any()` OR-gate helper
in `crates/ui/web-api/src/middleware/action.rs`, each incrementing `uptrakit_access_denies_total{reason=…}` exactly
once on an overall deny: `routes/plugin_type_settings.rs::can_view_type_settings` (`settings:read` OR
`system.settings:manage`), `routes/plugin_configs/crud.rs` (`software:read` OR `settings:read` OR
`system.settings:manage`), the `routes/system_services.rs` batch handler (per action: `system.services:approve` /
`system.services:reject` / `system.services:delete`), and the `routes/services/batch.rs` batch handler (per action:
`services:approve` / `services:reject` / `services:delete`). The fifth, `visibility.rs::is_plugin_visible_to_user`,
calls `AccessEngine::authorize()` directly for the single `system.settings:manage` action — it is a visibility
predicate used to filter instance-scoped plugins out of listings, not a request-denying gate, so it returns a `bool`
and does not increment the deny counter.

The legacy `permission_extractor!` + `x-required-permission` model described above no longer backs
any route family; the module was deleted in M1.7 (`middleware/permission.rs` no longer exists). Every
handler now imports its extractor from `crate::middleware::action::CanXxx`; the `Permission` enum itself
and its backing tables are removed in M1.8.

## System Service Credential Guard

Four capabilities grant access to sensitive infrastructure secrets:

| Capability          | Secret delivered                    |
| ------------------- | ----------------------------------- |
| `database_access`   | Database connection URL             |
| `nats_access`       | NATS server URL                     |
| `master_key_access` | Master AES-256 encryption key (hex) |
| `ca_management`     | Permission to request CA rotation   |

These credentials are delivered via `ServiceCredentials` after mTLS authentication and are never
published to NATS. Because they provide privileged access to the entire infrastructure, a service
must declare the `system_service` capability to request any of them.

The guard runs at enrollment time, before any database write, in `do_enroll()`:

```rust
if requests_system_creds && !has_system_service {
    bail!(AgentRouteError::Forbidden(
        "system credentials (database_access, nats_access, master_key_access, \
         ca_management) require the system_service capability"
    ));
}
```

A service that includes any of the four credential capabilities without `system_service` receives an
`ErrorCode::EnrollmentFailed` response with a descriptive message. The guard does not apply to the
system enrollment path (`do_enroll_system_service`), which is only reached when `system_service` is
already present.

See [System Services Architecture](https://github.com/worried-networking/uptrakit/tree/main/docs/architecture/) for the full enrollment flow
and two-tier service model.

## WebSocket Enrollment Secret Lookup

Services connect to the controller WebSocket endpoint at `/api/v1/ws/service`. Three authentication
paths exist:

| Path         | When              | How                                                                                                                               |
| ------------ | ----------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| mTLS         | Post-enrollment   | Client certificate issued after CSR approval; identity is cryptographically tied to the certificate serial number and service ID. |
| Bearer token | Enrollment window | `Authorization: Bearer <enrollment_secret>` header; secret is SHA-256 hashed and compared against the database.                   |
| Anonymous    | Pre-enrollment    | No credentials; only `enroll` messages are accepted.                                                                              |

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

See also: [Wire Protocol](https://github.com/worried-networking/uptrakit/tree/main/docs/api/) for connection sequencing.

## OAuth 2.1 for MCP

Uptrakit ships a dual-auth model for MCP access: opaque `upk_*` API tokens for non-interactive
callers (CLI, CI) and OAuth 2.1 for browser-capable MCP clients (Claude Desktop, Cursor). Auth is
prefix-dispatched at the MCP Resource Server — a `Bearer upk_` prefix routes to opaque token
validation; a `Bearer eyJ` prefix routes to JWT validation.

The cross-rejection guarantee is enforced by audience claims. Dashboard JWTs (`aud: ["uptrakit"]`,
short-lived session tokens) are rejected by the MCP Resource Server's OAuth validator. OAuth JWTs
(`aud: ["<oauth.canonical_host>/mcp"]`) are rejected by the Dashboard JWT middleware — `aud`
mismatch by design, preventing session token reuse as OAuth bearer tokens and vice versa.

See also:

- [MCP OAuth Authorization Design](../superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md)
- [ADR 0010 — MCP OAuth Authorization Server Placement](../adr/0010-mcp-oauth-authorization-server-placement.md)
- [OAuth MCP Security](oauth-mcp.md)
- [OAuth MCP Development Guide](../development/oauth-mcp.md)

## Content Security Policy

The admin UI's Content Security Policy (set in `frontend/src/app.html`) includes
`img-src 'self' https:`. This allows images from any HTTPS domain, which is required
to load OIDC provider logos configured by administrators.

**Accepted risk:** The admin who configures the OIDC provider logo URL is a trusted user.
Logo URLs are validated as HTTPS-only via `isValidLogoUrl()` in `frontend/src/lib/utils.ts`
before display. `referrerpolicy="no-referrer"` is applied to logo `<img>` elements,
preventing the Uptrakit URL from leaking to logo hosts via the `Referer` header.
