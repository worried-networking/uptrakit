# Profile Management — Design

Self-service profile editing: display name, email change with dual-address verification, and
password change. Scoped to the authenticated user's own account. No admin user-management
features in this spec.

## Scope

- Edit own first name / last name (all users)
- Initiate email change with strict verification (password-auth users only)
- Cancel pending email change
- Confirm email change via token link (unauthenticated)
- Change password (password-auth users only)

Out of scope: admin user listing/management, role assignment, invite flow, 2FA, custom role
CRUD. Those are separate specs.

## Relation to UI design-language rollout

Sub-spec #3j migrates existing buttons in `/profile/+page.svelte` from Skeleton preset classes
to the `<Button>` primitive. It explicitly lists "Change password launcher on profile" as a
non-goal — this feature does not exist yet. The new profile sections added by this spec are
net-new code and must use `<Button variant="...">` from the start (no migration needed later).

**Dependency:** This spec requires sub-spec #2 (Button primitive) to be merged before the
frontend work lands. Backend work has no such dependency.

## Auth constraints

`auth_method` is read from the JWT session token — not inferred from `password_hash IS NULL`.
The JWT is authoritative. The exact claim name must be verified against the existing JWT
payload structure at implementation time. OIDC users:

- **Can** edit first name / last name
- **Cannot** change email (provider-owned)
- **Cannot** change password (no password credential)

OIDC restrictions are enforced by reading `auth_method` from the `AuthenticatedUser` extractor
**before** reading the request body. This avoids deserializing `SecretString` fields for users
who cannot use those endpoints.

No combined OIDC + password auth is planned. The two paths are mutually exclusive.

## Data model

### New table: `email_change_request`

Migration: `m20260422_000001_email_change_request`

| Column       | Type              | Notes                                              |
|--------------|-------------------|----------------------------------------------------|
| `id`         | `Uuid`            | Primary key                                        |
| `user_id`    | `Uuid`            | FK → `user.id` CASCADE DELETE; UNIQUE              |
| `new_email`  | `EncryptedString` | Encrypted at rest via `uptrakit_crypto`            |
| `token_hash` | `String`          | Output of `hash_token()` — raw token never stored  |
| `expires_at` | `OffsetDateTime`  | 24 h from creation                                 |
| `created_at` | `OffsetDateTime`  |                                                    |

`tenant_id` is intentionally omitted: the `user` entity itself has no `tenant_id` — users are
global identities, and role-to-tenant scoping lives in `user_role`. The FK through `user_id`
is sufficient for this table.

`user_id` UNIQUE enforces one pending change per user. A new request replaces the existing
row (see upsert pattern below). Expired rows are cleaned up lazily on a failed confirm attempt
or by the existing background scheduler (implementer: extend the appropriate scheduler job;
cadence: daily).

**Token generation:** Use `generate_secure_token()` from `uptrakit_web_api_auth::auth::token`.
**Token hashing:** Use `hash_token()` from the same module. Do not re-implement SHA-256
inline.

`EncryptedString` uses `uptrakit_crypto`. If the crypto key rotates, existing pending rows
become unreadable. This is acceptable: rows are short-lived (24 h max) and an unreadable row
simply forces the user to restart the email change flow. No cleanup path is required.

On email confirm, the decrypted value must be converted to `MaskedEmail` before writing to
`user.email`:

```rust
let new_email: MaskedEmail = decrypt_str(&row.new_email)?
    .parse()
    .context_to("invalid encrypted email")?;
// then: user_active_model.email = Set(new_email);
```

Note: `MaskedEmail` holds the full address internally — it only masks in `Debug`/`Display`.
It is correct as the type for both `new_email` in `InitiateEmailChangeRequest` and the
`user.email` column.

#### Upsert pattern (SeaORM)

SeaORM's `on_conflict()` builder differs between SQLite and PostgreSQL. Use explicit
find-then-replace inside a transaction:

```rust
let txn = db.begin().await?;
if let Some(existing) = EmailChangeRequest::find()
    .filter(email_change_request::Column::UserId.eq(user_id))
    .one(&txn).await? {
    existing.delete(&txn).await.context_to("delete existing")?;
}
EmailChangeRequestActiveModel {
    id: Set(Uuid::new_v4()),
    user_id: Set(user_id),
    new_email: Set(encrypt_str(&new_email.as_str())?),
    token_hash: Set(token_hash),
    expires_at: Set(OffsetDateTime::now_utc() + Duration::hours(24)),
    created_at: Set(OffsetDateTime::now_utc()),
}.insert(&txn).await.context_to("insert")?;
txn.commit().await.context_to("commit")?;
```

### Change to `/auth/me` response

```rust
pub struct MeResponse {
    // ... existing fields unchanged ...
    pub has_pending_email_change: bool,
}
```

Always present — `false` when no pending row exists. `auth_method` is not added here; the
frontend reads it from JWT claims directly.

## API endpoints

All new routes live under `/api/v1/users/{id}/` to co-locate with existing user management
routes and support future admin CRUD on the same resource.

**Self-service permission check** for profile PUT:
`authenticated_user.user_id == path_id || authenticated_user.has_permission(ManageUsers)`.
This requires an inline check in the handler — the existing `CanManageUsers` extractor blocks
any caller without `ManageUsers`, which would break self-service. Email and password endpoints
use own-only (`authenticated_user.user_id == path_id`) regardless of permissions.

All endpoints on `POST /users/{id}/email` and `PUT /users/{id}/password` must be covered by
the existing rate-limit middleware (`rate_limit.rs`). Argon2 verification on every request
makes unthrottled wrong-password loops expensive.

The confirm endpoint lives at `/api/v1/auth/email-change/confirm` rather than under
`/users/{id}/` because it is unauthenticated — no authenticated user ID is available to place
in the path. This matches the namespace of other unauthenticated auth flows (OIDC callback,
device authorize).

### `PUT /api/v1/users/{id}/profile`

**Auth:** inline own-or-`ManageUsers` check.
**Available to:** all users (password and OIDC).

```rust
pub struct UpdateProfileRequest {
    pub first_name: String,  // non-empty, max 100
    pub last_name: String,   // non-empty, max 100
}

impl Validate for UpdateProfileRequest { ... }  // standard length + non-empty checks
```

Response: updated `UserWithRolesResponse`. Writes audit log (`USER_UPDATE`, changed fields).

---

### `POST /api/v1/users/{id}/email`

**Auth:** own only.
**OIDC guard:** check `auth_method` from `AuthenticatedUser` before reading the request body
— return 403 `"Email is managed by your identity provider"` immediately.

```rust
pub struct InitiateEmailChangeRequest {
    pub current_password: SecretString,
    pub new_email: MaskedEmail,
}

impl Validate for InitiateEmailChangeRequest { ... }
```

`SecretString` cannot use derive-based length validation — no validation is needed on
`current_password` (Argon2 verify handles it). `new_email` validation is structural (provided
by `MaskedEmail::from_str` on deserialization).

Flow:

1. Verify `auth_method` — 403 if OIDC (before body read).
2. Verify `current_password` against `user.password_hash` (Argon2, constant-time). Wrong →
   401 `"Invalid credentials"`. Use constant-time comparison even when `password_hash` is
   `None` (run a dummy verify to prevent timing leaks).
3. Check `new_email` not already registered to another user → 409 on conflict.
4. Attempt both email deliveries (see Email delivery section). Collect both results before
   proceeding. If either fails → 503; do **not** save the row. The partial-delivery case
   (first email succeeds, second fails) results in a dead link (token never saved) — 503
   is returned and the user can retry cleanly.
5. Upsert `email_change_request` row (transaction pattern above).
6. Return 202 Accepted.

---

### `DELETE /api/v1/users/{id}/email`

**Auth:** own only.

Deletes `email_change_request` row for `user_id` if present. Idempotent — 204 even if no
pending change exists.

Unauthenticated cancel is intentionally not supported. An unauthenticated cancel link in the
old-address email creates a DoS vector: an attacker who initiates a change can use such a
link to cancel a legitimate user's in-progress change.

Admin cancellation of another user's pending change is **not in scope for this spec** but is
explicitly planned for a future admin user-management spec. At that point, extend this
endpoint to allow `ManageUsers` callers to cancel for any `{id}`.

---

### `GET /api/v1/auth/email-change/confirm`

**Auth:** none (token-based).
**Query param:** `?token=<raw_token>`

The raw token appears in server access logs and browser history. The frontend confirm page
must not load any third-party resources (prevents token leakage via `Referer` header). The
response must include `X-Frame-Options: DENY` and `X-Content-Type-Options: nosniff`.

Flow (all DB operations in one transaction):

```rust
let txn = db.begin().await?;

// 1. Hash and look up
let token_hash = hash_token(&raw_token);
let row = EmailChangeRequest::find()
    .filter(email_change_request::Column::TokenHash.eq(&token_hash))
    .one(&txn).await?
    .ok_or_else(|| /* 404 Not Found */)?;

// 2. Check expiry — 410 Gone (permanent failure, not transient auth error)
if row.expires_at < OffsetDateTime::now_utc() {
    row.delete(&txn).await.context_to("delete expired")?;
    txn.commit().await.context_to("commit")?;
    return Err(/* 410 */);
}

// 3. Check race: new_email taken by another user
let new_email: MaskedEmail = decrypt_str(&row.new_email)?.parse().context_to("parse email")?;
let conflict = User::find()
    .filter(user::Column::Email.eq(new_email.as_str()))
    .filter(user::Column::Id.ne(row.user_id))
    .one(&txn).await?;
if conflict.is_some() {
    row.delete(&txn).await.context_to("delete conflicted")?;
    txn.commit().await.context_to("commit")?;
    return Err(/* 409 */);
}

// 4. Update user.email
UserActiveModel {
    id: Unchanged(row.user_id),
    email: Set(new_email),
    updated_at: Set(OffsetDateTime::now_utc()),
    ..Default::default()
}.update(&txn).await.context_to("update email")?;

// 5. Delete the request row
row.delete(&txn).await.context_to("delete row")?;

txn.commit().await.context_to("commit")?;
```

After commit:

1. Invalidate **all** existing sessions. This endpoint is unauthenticated, so there is no
   "current session" to preserve. Call the full logout-all path:
   - `session_service.delete_user_sessions(user_id)` — revoke all refresh token rows
   - `token_denylist.deny_user(user_id, now, purge_after)` — block existing access tokens
     (prevents up-to-15-min window where old JWTs remain valid)
   - Publish `ControllerMessage::TokenRevoked` for cross-instance propagation. Implementer:
     verify the correct variant and payload with the controller team before shipping.
2. Return 200 `{ "message": "Email updated. Please sign in again." }`.

---

### `PUT /api/v1/users/{id}/password`

**Auth:** own only.
**OIDC guard:** check `auth_method` before body — 403
`"Password change is not available for OIDC accounts"`.

```rust
pub struct ChangePasswordRequest {
    pub current_password: SecretString,
    pub new_password: SecretString,  // min 8 chars, max 128
}

impl Validate for ChangePasswordRequest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        // SecretString has no derive support; validate via .expose_secret()
        let len = self.new_password.expose_secret().len();
        if len < 8 || len > 128 {
            // add field error
        }
        Ok(())
    }
}
```

`SecretString` is automatically zeroized when the request scope ends — no explicit drop
needed.

Flow:

1. Verify `auth_method` — 403 if OIDC (before body read).
2. Verify `current_password` (constant-time Argon2). Wrong → 401 generic.
3. Hash `new_password` (Argon2).
4. Update `user.password_hash`.
5. Invalidate all sessions **except current**:
   - Add `delete_user_sessions_except(user_id: Uuid, except_session_id: Uuid)` to
     `SessionService`. Filters `session::Column::UserId.eq(user_id)` AND
     `session::Column::Id.ne(except_session_id)`. This is a new method — implement it.
   - Call `token_denylist.deny_user_except(user_id, current_jti, now, purge_after)` to
     block old access tokens while leaving the current one valid. If this method does not
     exist, add it. Implementing a 15-minute grace window (skip the denylist and let old
     tokens expire naturally) is **not acceptable** — it leaves other active sessions able
     to call any authenticated endpoint until their token expires.
   - Publish `ControllerMessage::TokenRevoked`.
6. Write audit log (`USER_UPDATE`, `changed_fields: ["password"]`).

## Email delivery

Reuses the existing three-layer SMTP config: global defaults in `global_settings`, per-tenant
overrides in `settings`, merged via `merge_smtp_into_config(global, tenant, config)`.

**`base_url`:** Not a `SettingKey`. Use `Option<Extension<ExternalBaseUrl>>` injected by the
`resolve_proxy_headers` middleware, with fallback to `base_url_from_headers(&headers)` — same
pattern as `oidc_auth.rs` routes.

**Call site:** Reuse the lowest-level `email_plugin.deliver(config, message)` call used by the
notification dispatcher. Do not create a new email abstraction. The plugin must return a typed
error that distinguishes `SmtpNotConfigured` from `SmtpDeliveryFailed` so the handler can
return the correct 503 message. If the plugin does not yet return a typed error here, add the
variant before implementing the handler.

Both emails are dispatched **before** saving the `email_change_request` row:

1. Build both `DeliveryMessage` values.
2. Call `deliver()` for template 1. Capture result.
3. Call `deliver()` for template 2. Capture result.
4. If either result is `Err` → return 503 immediately; do not proceed to row upsert.
5. Upsert row.
6. Return 202.

### Template 1 — Confirm new email (sent to `new_email`)

- **Subject:** `Confirm your new email address — Uptrakit`
- **Body:** A request was made to change the email on account `{old_email}`. Confirm:
  `{base_url}/auth/email-change/confirm?token={raw_token}`. Expires in 24 hours. If you did
  not request this, contact your administrator.

### Template 2 — Change notification (sent to old email)

- **Subject:** `Email address change requested — Uptrakit`
- **Body:** A change was requested from `{old_email}` to `{masked_new_email}` (e.g.
  `j***@example.com`). To cancel: sign in and go to Profile → Cancel pending change.
  Masking prevents leaking the new address to anyone with access to the old inbox.

## Frontend

Profile page at `/profile` already exists (219 lines) with static account fields, API token
list, and token management modal. The new sections are additions — no existing markup is
modified by this spec.

New sections use `<Button variant="...">` primitive from the start. They are not in scope
for sub-spec #3j (which migrates only pre-existing buttons in this file). Dependency on
sub-spec #2 (Button primitive) merged before this frontend work lands.

### Additions to `/profile`

**Profile details section** (all users)

Inline editable form: First name, Last name. Save calls `PUT /users/{id}/profile`. Email
field always read-only. Password users: "Change email" button below email field. OIDC users:
"Managed by your identity provider" label instead.

**Change email section** (password users only, conditional on JWT `auth_method` claim)

Two states driven by `has_pending_email_change` from `/me`:

*No pending change:* Form with Current password and New email address fields. Submit calls
`POST /users/{id}/email`. On success: inline message "Check your inbox at {new_email} to
confirm."

*Pending change:* Banner: "Confirmation email sent. Waiting for confirmation." Cancel button
calls `DELETE /users/{id}/email`.

**Change password section** (password users only)

Form: Current password, New password, Confirm new password (client-side match validation
before submit). Submit calls `PUT /users/{id}/password`. On success: "Password changed. Other
sessions have been signed out."

### New route: `/auth/email-change/confirm`

Minimal page — no navigation chrome. Must not load any third-party resources (token in URL).
Calls `GET /auth/email-change/confirm?token=...` on mount.

- Loading state while request in flight.
- Success: "Email updated. Please sign in again." → redirect to `/login` after short delay.
- Failure (not found / expired / race): "This link has expired or has already been used."
  — link back to `/profile`.

## Error handling

| Scenario | Response |
| --- | --- |
| `new_email` already registered | 409 Conflict |
| `current_password` incorrect | 401 `"Invalid credentials"` |
| OIDC user → email endpoint | 403 (before body read) |
| OIDC user → password endpoint | 403 (before body read) |
| Confirmation token not found | 404 |
| Confirmation token expired | 410 Gone; row deleted |
| `new_email` taken on confirm (race) | 409; row deleted; user must restart |
| SMTP not configured | 503 `"Email delivery not configured"` |
| SMTP delivery failure | 503 `"Email delivery failed"` |
| Non-owner, no `ManageUsers`, profile PUT | 403 |
| `DELETE` with no pending change | 204 (idempotent) |
| User deactivated mid-flow | 401 on next auth middleware check |

## Testing

### Unit

- Argon2 wrong-password path: constant-time comparison does not exit early when
  `password_hash` is `None` (run dummy verify, assert timing is within threshold).
- Token round-trip: `generate_secure_token()` → `hash_token()` → lookup matches; different
  tokens produce different hashes.
- OIDC guard fires before body deserialization for both email and password endpoints
  (mock `AuthenticatedUser` with OIDC `auth_method`).
- `Validate` on `ChangePasswordRequest`: password < 8 chars fails; password > 128 chars
  fails; password = 8 chars passes.

### Integration (SQLite)

- `#[tokio::test(start_paused = true)]` required for any test that advances or checks
  `expires_at` (per `start_paused = true` rule: only for tests that call tokio time APIs).
- Second `POST /users/{id}/email` invalidates first token: upsert replaces row; old token
  returns 404 on confirm.
- Confirm with expired token: advance time past 24 h; confirm returns 410; row deleted.
- Confirm race: insert a `User` row with `new_email` before confirm → 409; row deleted.
- Confirm success: `user.email` updated; `delete_user_sessions` called (assert session count
  drops to 0).
- Password change: session count decreases by N−1; current session survives.
- SQLite FK: deleting a user cascades to `email_change_request` row deletion.

### EncryptedString test setup

- Call `uptrakit_crypto::enable_plaintext_mode()` in DB setup helpers for integration tests.
- Add `uptrakit-crypto = { workspace = true, features = ["testing"] }` to
  `[dev-dependencies]` of the relevant query-test crate.

### Email delivery (mock SMTP)

- Both templates sent on successful `POST /users/{id}/email`.
- Template 2 body masks `new_email` (assert `j***@example.com` format, not raw address).
- 503 `"Email delivery not configured"` when SMTP absent; row not saved.
- 503 `"Email delivery failed"` when plugin returns delivery error; row not saved.
- Partial delivery (template 1 ok, template 2 fails): 503 returned; row not saved.

### Frontend

- OIDC user: email and password sections not rendered.
- No-pending state: form visible; submit calls correct endpoint.
- Pending state: banner visible; cancel button calls DELETE.
- Confirm page: loading state on mount; success redirects to `/login`; expired/not-found
  shows error message + link to `/profile`.
