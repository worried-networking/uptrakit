# 2FA for Password Authentication

**Date:** 2026-05-12
**Status:** Draft

## Overview

Adds two-factor authentication (2FA) to the password authentication path. Operators may enroll a TOTP
authenticator app (primary) or use email OTP (fallback). Eight single-use recovery codes are issued at
enrollment. Admins can enforce 2FA tenant-wide via a settings toggle.

OIDC authentication is excluded — OIDC providers handle their own MFA.

## Decisions

| #   | Decision                                                                                                                                    |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | TOTP (primary) + email OTP (fallback). No SMS.                                                                                              |
| 2   | Per-user opt-in + optional per-tenant enforcement (`SettingKey::TwoFactorRequired`, per-tenant, default `false`).                           |
| 3   | 8 single-use recovery codes, 10-char alphanumeric, Argon2id-hashed, shown once.                                                             |
| 4   | Two-phase challenge token login protocol (industry standard).                                                                               |
| 5   | Email OTP uses `NotificationOps::send_transactional_email()` (existing trait method); reuses three-layer SMTP config. 6-digit numeric code. |
| 6   | 5 failed attempts invalidates the MFA token.                                                                                                |
| 7   | Enforcement + unenrolled: restricted JWT (`setup_required: true`); middleware gates all non-enrollment routes.                              |
| 8   | No trusted devices in v1.                                                                                                                   |
| 9   | No compile-time feature flag.                                                                                                               |
| 10  | Single DB migration creates all three new tables.                                                                                           |

---

## Typed Enum: `MfaMethod`

Internal discriminator enum. NOT a wire-protocol enum (no `Other(String)` needed; deserialized from HTTP request via `FromStr`, never sent over NATS/WS).

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(strum::EnumIter))]
#[non_exhaustive]
pub enum MfaMethod {
    Totp,
    Email,
    RecoveryCode,
}

impl MfaMethod {
    pub const fn as_str(self) -> &'static str { ... }
}

impl std::fmt::Display for MfaMethod { ... }
impl std::str::FromStr for MfaMethod { ... }  // "totp" | "email" | "recovery_code"
impl serde::Serialize for MfaMethod { ... }   // serializes as &str for API responses
impl serde::Deserialize for MfaMethod { ... } // deserializes from string
```

Lives in `uptrakit-web-api-types` alongside the request/response types that use it.

---

## Database Schema

**Migration:** `m20260512_000001_2fa` — creates all three tables.

Use `.string()` (VARCHAR) for all token/hash columns to match the existing migration pattern
(see `m20260422_000001_email_change_request.rs`). Unique constraints use a separate
`create_index(...).unique()` call, not inline `UNIQUE` on the column.

### `user_totp`

One row per user. Replaced atomically on re-enrollment.

```sql
id           UUID         PRIMARY KEY
user_id      UUID         NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE
secret       TEXT         NOT NULL          -- EncryptedString (AES-GCM, master key)
is_active    BOOLEAN      NOT NULL DEFAULT FALSE
enrolled_at  TIMESTAMPTZ  NULL              -- set when enrollment confirmed
created_at   TIMESTAMPTZ  NOT NULL
```

### `user_recovery_codes`

8 rows per user. All replaced atomically on regeneration.

```sql
id         UUID         PRIMARY KEY
user_id    UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE
code_hash  TEXT         NOT NULL   -- Argon2id hash of 10-char plaintext code
created_at TIMESTAMPTZ  NOT NULL
used_at    TIMESTAMPTZ  NULL       -- NULL = unused; set on consumption
```

Index: `(user_id, used_at)` for fast lookup of unused codes.

**Performance note:** Recovery code verification requires an Argon2id compare against each unused code
(worst case 8 × ~100 ms ≈ 800 ms). This work must run inside `tokio::task::spawn_blocking` — identical
to how `verify_password` is called in the existing login path — to avoid blocking the async executor.
No fast pre-filter is needed at v1 scale.

### `mfa_challenges`

Short-lived, single-use bridge tokens between password verification and session creation.

```sql
id               UUID         PRIMARY KEY
user_id          UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE
token_hash       VARCHAR      NOT NULL   -- SHA-256 of plaintext token; separate UNIQUE index
email_code_hash  VARCHAR      NULL       -- Argon2id hash; set when email OTP sent
attempt_count    INTEGER      NOT NULL DEFAULT 0
expires_at       TIMESTAMPTZ  NOT NULL   -- created_at + 5 minutes
consumed_at      TIMESTAMPTZ  NULL       -- set on success OR attempt exhaustion
created_at       TIMESTAMPTZ  NOT NULL
```

Unique index on `token_hash`. SQLite: all read-then-write paths use `BEGIN IMMEDIATE`.

**Cleanup:** Expired and consumed challenge rows accumulate. The existing Scheduler-driven cleanup pattern
(same as session cleanup) must be applied:
`DELETE FROM mfa_challenges WHERE expires_at < now() - interval '1 day'`.
Wire into the existing periodic cleanup job in `scheduler-runtime`.

---

## API

### Modified

**`POST /api/v1/auth/login`**

Unchanged when 2FA is not enrolled (returns `200 AuthResponse`).

When TOTP is active for the user:

```text
202 MfaChallengeResponse {
  mfa_token:   String,             // plaintext opaque token, 5 min TTL
  mfa_methods: Vec<MfaMethod>,     // always [Totp]; includes Email only if SMTP configured
}
```

When enforcement is on and user is unenrolled:

```text
200 AuthResponse   // JWT contains setup_required: true claim
```

### New — unauthenticated

Both endpoints apply per-IP rate limiting via `RateLimitStore` (same key scheme as login), in addition to the per-challenge attempt limit.

**`POST /api/v1/auth/mfa/verify`**

```text
Request: MfaVerifyRequest {
  mfa_token: String,
  code:      String,
  method:    MfaMethod,
}

200 AuthResponse   // full session on success
401                // wrong code or token invalid/expired/exhausted
```

**`POST /api/v1/auth/mfa/email`**

Sends a 6-digit email OTP and stores its Argon2id hash in the challenge row. Returns `400` if the
user's email address is absent or empty (guard before calling SMTP).

```text
Request: MfaEmailRequest { mfa_token: String }

200   // email sent
400   // no email address on user record
503   // SMTP unavailable
```

### New — requires JWT (including `setup_required` JWTs)

`setup_required` enforcement uses a typed Axum extractor (analogous to existing permission extractors)
so route handlers never contain inline JWT claim checks.

**`setup_required` JWT scope:** Only `GET /2fa` (status), `POST /2fa/totp/enroll`, and
`POST /2fa/totp/confirm` are accessible. `POST /2fa/totp/disable` and
`POST /2fa/recovery-codes/regenerate` require a **full-session JWT** (no `setup_required` claim).
This prevents the enforcement bypass path where a user calls `/disable` with password re-auth from a
restricted session to remove their own TOTP and re-login without 2FA.

**`GET /api/v1/auth/me/2fa`**

```text
200 MfaStatusResponse {
  totp_enrolled:         bool,
  recovery_codes_count:  u32,          // count of unused codes; DB count cast via u32::try_from
  methods_available:     Vec<MfaMethod>,
}
```

**`POST /api/v1/auth/me/2fa/totp/enroll`**

Generates a new TOTP secret (or replaces a pending one). Does not activate until confirmed.

```text
200 TotpEnrollResponse {
  secret:   String,   // base32, for manual entry
  qr_code:  String,   // SVG string of otpauth:// QR code
  totp_uri: String,   // otpauth://totp/... URI
}
```

**`POST /api/v1/auth/me/2fa/totp/confirm`**

Verifies the submitted TOTP code against the pending secret, activates TOTP, issues 8 recovery codes.
If enforcement is on and this completes enrollment, response also includes a new full-session
`AuthResponse` (replacing the restricted `setup_required` session); the confirm handler issues a
fresh access + refresh token pair and sets the refresh token cookie.

```text
Request: TotpConfirmRequest { code: String }

200 TotpConfirmResponse {
  recovery_codes:  Vec<String>,        // 8 codes, shown once, never retrievable
  session:         Option<AuthResponse>,  // present only if setup_required was active
}
```

**`POST /api/v1/auth/me/2fa/totp/disable`**

Disables TOTP and wipes all recovery codes. Requires re-authentication. `Validate` impl must reject
`(None, None)` (both absent) and `(Some, Some)` (both provided) — exactly one must be `Some`.
Implemented via a custom `fn validate(&self) -> Result<(), ValidationErrors>`.

```text
Request: DisableTotpRequest {
  password:  Option<SecretString>,
  totp_code: Option<String>,
}

204
```

**`POST /api/v1/auth/me/2fa/recovery-codes/regenerate`**

Invalidates all existing recovery codes, issues 8 new ones. Same re-authentication requirement and mutual-exclusion validation as `DisableTotpRequest`.

```text
Request: RegenerateRecoveryCodesRequest {
  password:  Option<SecretString>,
  totp_code: Option<String>,
}

200 RegenerateRecoveryCodesResponse {
  recovery_codes: Vec<String>,   // 8 codes
}
```

### Extended — existing admin settings

**`GET /PUT /api/v1/settings/auth`** — add `two_factor_required: bool` field alongside existing `password_auth_enabled`.

---

## Auth Flow

### Login with 2FA enrolled

```text
POST /api/v1/auth/login
  → verify password (unchanged)
  → query user_totp WHERE user_id = ? AND is_active = true
  → found:
      create mfa_challenges row (hashed token, expires_at = now + 5m)
      emit AUTH_MFA_CHALLENGE_ISSUED audit event
      return 202 MfaChallengeResponse
  → not found:
      check SettingKey::TwoFactorRequired
      if required:
        issue JWT with setup_required: true
        return 200 AuthResponse (restricted)
      else:
        normal login → return 200 AuthResponse (full)
```

### MFA verification

```text
POST /api/v1/auth/mfa/verify
  → look up mfa_challenges by SHA-256(token)
  → reject if: not found | consumed_at IS NOT NULL | now >= expires_at
  → check attempt_count < 5; else poison (set consumed_at) + return 401

  method = Totp:
    verify code via totp-rs (skew=1, step=30s; effective tolerance ±30s around step boundary)

  method = Email:
    Argon2id verify code against email_code_hash
    reject if email_code_hash IS NULL (no code sent yet)

  method = RecoveryCode:
    find unused user_recovery_codes row where Argon2id verify succeeds
    set used_at = now (BEGIN IMMEDIATE txn)

  on success:
    set mfa_challenges.consumed_at = now
    create full session (SessionOps::create_refresh_token)
    issue JWT (no setup_required claim)
    emit AUTH_MFA_VERIFIED
    return 200 AuthResponse

  on failure:
    increment attempt_count
    if attempt_count >= 5:
      set consumed_at (poison)
      emit AUTH_MFA_CHALLENGE_EXHAUSTED (method)
    else:
      emit AUTH_MFA_FAILED (attempt_count, method)
    return 401
```

### `setup_required` session lifecycle

```text
1. Operator logs in, enforcement on, TOTP not enrolled
   → restricted JWT issued (setup_required: true)

2. Middleware (typed Axum extractor): if setup_required claim present,
   only /api/v1/auth/me/2fa/* routes pass; all others → 403 {
     error: "2fa_setup_required"
   }

3. Operator calls /enroll → /confirm
   On successful confirm:
     - TOTP activated
     - Server issues new full-session access + refresh token pair
     - Returns in TotpConfirmResponse.session (with Set-Cookie for refresh token)
     - Frontend replaces stored tokens

4. Old restricted session (refresh token) remains valid in DB
   but will naturally expire in 7 days; frontend discards it
   on receiving new tokens.

5. If the 15-min setup_required access JWT expires before enrollment completes,
   the Operator uses their refresh token to get a new access JWT. The refresh
   handler (routes/auth.rs) must query user_totp.is_active AND check
   SettingKey::TwoFactorRequired after token rotation, before minting the
   access JWT, to determine whether to set setup_required: true again.
   This requires one additional DB read in the refresh path.

   ⚠️ PREREQUISITE: The refresh handler modification is a hard prerequisite
   for the enforcement feature (SettingKey::TwoFactorRequired) to have any
   security value. Without it, any user with an existing session can bypass
   enforcement for up to 7 days by simply letting their access JWT expire and
   calling /auth/refresh. The enforcement toggle MUST NOT be exposed in the
   UI/API until the refresh handler check is in place and deployed.
```

---

## Code Organization

### New files

| File                                                     | Purpose                                                                                            |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `crates/ui/web-api-auth/src/auth/totp.rs`                | TOTP secret generation, verification via `totp-rs`, QR URI construction                            |
| `crates/ui/web-api-auth/src/auth/mfa_challenge.rs`       | Challenge token lifecycle; email OTP generation; recovery code generation/verification; all DB ops |
| `crates/ui/web-api/src/routes/mfa.rs`                    | `verify` and `send_email` handlers                                                                 |
| `crates/ui/web-api/src/routes/me_2fa.rs`                 | Enrollment handlers: `status`, `enroll`, `confirm`, `disable`, `regenerate_recovery_codes`         |
| `crates/shared/db/src/entity/user_totp.rs`               | SeaORM entity                                                                                      |
| `crates/shared/db/src/entity/user_recovery_code.rs`      | SeaORM entity                                                                                      |
| `crates/shared/db/src/entity/mfa_challenge.rs`           | SeaORM entity                                                                                      |
| `crates/shared/db/src/migration/m20260512_000001_2fa.rs` | Single migration: all three tables                                                                 |

### Modified files

| File                                                | Change                                                                                                                                                                                                                                                                                                                                                |
| --------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/ui/web-api-auth/src/setting_key.rs`         | Add `TwoFactorRequired` variant (`auth.two_factor_required`, per-tenant)                                                                                                                                                                                                                                                                              |
| `crates/ui/web-api-auth/src/auth/authentication.rs` | Add `two_factor_required: bool` to `AuthenticationSettings`                                                                                                                                                                                                                                                                                           |
| `crates/ui/web-api-auth/src/auth/jwt.rs`            | Add `setup_required: Option<bool>` to JWT claims struct with `#[serde(default, skip_serializing_if = "Option::is_none")]` — required so existing tokens without this field deserialize without error on deploy; update `create_access_token` signature to accept `setup_required: Option<bool>`; update all call sites in `auth.rs`, OIDC paths, etc. |
| `crates/ui/web-api-auth/src/auth/error.rs`          | Add new `AuthError` variants: `MfaChallengeNotFound`, `MfaChallengeExpired`, `MfaChallengeExhausted`, `MfaCodeInvalid`, `EmailDeliveryUnavailable`                                                                                                                                                                                                    |
| `crates/ui/web-api/src/routes/auth.rs`              | Modify `login` handler for 2FA branch; modify `refresh` handler to query `user_totp.is_active` + enforcement after token rotation before minting JWT                                                                                                                                                                                                  |
| `crates/ui/web-api/src/routes/settings_auth.rs`     | Add `two_factor_required` field to GET/PUT                                                                                                                                                                                                                                                                                                            |
| `crates/ui/web-api-types/src/auth.rs`               | New types: `MfaMethod` enum + all request/response types                                                                                                                                                                                                                                                                                              |
| `crates/shared/db/src/entity/mod.rs`                | Re-export new entities                                                                                                                                                                                                                                                                                                                                |
| `crates/shared/db/src/migration/mod.rs`             | Register new migration                                                                                                                                                                                                                                                                                                                                |
| `crates/core/scheduler-runtime/src/...`             | Add `mfa_challenges` cleanup to periodic cleanup job                                                                                                                                                                                                                                                                                                  |
| `Cargo.toml`                                        | Add `totp-rs` workspace dep (features: `gen_secret`, `otpauth`); run `cargo deny check` after — verify no duplicate transitive deps introduced                                                                                                                                                                                                        |

### New API types (`uptrakit-web-api-types`)

All implement `Validate`. All public response structs carry `#[non_exhaustive]`.

- `MfaMethod` (enum — not a response struct; no `#[non_exhaustive]` needed since it's exhaustively
  matched internally; DO add `#[non_exhaustive]` per standard)
- `MfaChallengeResponse` — `#[non_exhaustive]`
- `MfaVerifyRequest`
- `MfaEmailRequest`
- `MfaStatusResponse` — `#[non_exhaustive]`
- `TotpEnrollResponse` — `#[non_exhaustive]`
- `TotpConfirmRequest`
- `TotpConfirmResponse` — `#[non_exhaustive]`
- `DisableTotpRequest` — `Validate` impl enforces exactly-one-of constraint
- `RegenerateRecoveryCodesRequest` — same mutual-exclusion `Validate`
- `RegenerateRecoveryCodesResponse` — `#[non_exhaustive]`

---

## Audit Log Events

New `AuditActionType` constants following existing `AUTH_LOGIN` / `AUTH_LOGOUT` naming pattern:

| Constant                        | Trigger                                                           | Outcome   | Key details fields        |
| ------------------------------- | ----------------------------------------------------------------- | --------- | ------------------------- |
| `AUTH_MFA_CHALLENGE_ISSUED`     | MFA token created after password verified                         | `Success` | `user_id`                 |
| `AUTH_MFA_VERIFIED`             | Challenge completed                                               | `Success` | `method`, `user_id`       |
| `AUTH_MFA_FAILED`               | Wrong code submitted (attempts 1–4)                               | `Denied`  | `method`, `attempt_count` |
| `AUTH_MFA_CHALLENGE_EXHAUSTED`  | 5th failure; emitted INSTEAD OF `AUTH_MFA_FAILED`; token poisoned | `Denied`  | `user_id`, `method`       |
| `AUTH_MFA_ENROLLED`             | TOTP enrollment confirmed                                         | `Success` | `user_id`                 |
| `AUTH_MFA_DISABLED`             | TOTP disabled by user                                             | `Success` | `user_id`                 |
| `AUTH_MFA_RECOVERY_USED`        | Recovery code consumed                                            | `Success` | `remaining_count`         |
| `AUTH_MFA_RECOVERY_REGENERATED` | All recovery codes replaced                                       | `Success` | `user_id`                 |

All use `actor_type = User`, `actor_id = user_id`, `target = ("user", user_id)`. Emitted via `state.audit_emitter.emit_best_effort(entry)`.

---

## Email OTP Delivery

Use `NotificationOps::send_transactional_email()` (on `state.plugin.plugin_ops`). Do NOT construct `DeliveryMessage` manually or call `transport()` directly.

**Actual signature** (confirmed in `crates/plugins/infrastructure/core/src/plugin_ops.rs`):

```rust
async fn send_transactional_email(
    &self,
    tenant_db: &uptrakit_tenant_db::TenantDb,
    to: &str,
    subject: &str,
    text_body: &str,
    html_body: &str,
) -> Result<(), TransactionalEmailError>
```

`TransactionalEmailError` is `#[non_exhaustive]`; match with wildcard arm.
Import `TransactionalEmailError` from `uptrakit_plugin_infrastructure_registry` (re-exported there;
see `routes/users.rs:1233` for existing usage pattern).

```text
1. Look up challenge by token_hash → get user_id
2. Load user from DB → get email address; return 400 if absent/empty
3. Construct TenantDb for default_tenant_id (same pattern as other handlers)
4. Generate 6-digit code: OsRng, zero-padded decimal (000000–999999)
5. Hash with Argon2id inside spawn_blocking (same params as password hashing)
6. Store hash in mfa_challenges.email_code_hash (BEGIN IMMEDIATE txn)
7. Call state.plugin.plugin_ops.send_transactional_email(
       &tenant_db,
       &user.email,
       "Your uptrakit login code",
       &format!("Your code is: {code}\n\nValid for 5 minutes. Do not share."),
       &format!("<p>Your code is: <strong>{code}</strong></p><p>Valid for 5 minutes.</p>"),
   ).await
8. TransactionalEmailError::NotConfigured → 503
   TransactionalEmailError::DeliveryFailed(_) → 503
   _ => 503 (wildcard required; enum is #[non_exhaustive])
```

The code expires when the MFA challenge expires (5 minutes). Sending a new email (re-calling this
endpoint) overwrites `email_code_hash` — previous code is invalidated.

---

## Security Properties

- MFA token: 32-byte random value, SHA-256 hashed for storage (same pattern as refresh tokens)
- Recovery codes: 10-char from charset `ABCDEFGHJKLMNPQRSTUVWXYZ23456789` (32 chars; excludes
  visually ambiguous 0/O/1/I), Argon2id-hashed. Entropy: 32^10 ≈ 2^50.
- TOTP secret: stored via `EncryptedString` (AES-GCM, master key); never returned after enrollment
- Constant-time comparison for all code verification (`subtle::ConstantTimeEq`); MFA token lookup is DB-index equality, same pattern as refresh tokens
- TOTP: skew=1, step=30s — effective tolerance is ±30s around each step boundary
- 5-attempt limit per challenge: challenge poisoned on exhaustion (not just rate-limited)
- Per-IP rate limit on `/auth/mfa/verify` and `/auth/mfa/email` via `RateLimitStore` (prevents
  challenge spray across multiple concurrent challenges from one IP)
- Disabling TOTP or regenerating recovery codes requires password or current TOTP code re-auth
- `setup_required` JWT blocked from all routes except `/api/v1/auth/me/2fa/*` by typed extractor

---

## Frontend Changes

### Login page (`frontend/src/routes/(public)/login/+page.svelte`)

- Detect `202` response from `POST /api/v1/auth/login`
- Transition to MFA step: TOTP code input (6 digits, auto-submit on fill)
- "Use email instead" button → `POST /api/v1/auth/mfa/email` → show email code input
- Submit calls `POST /api/v1/auth/mfa/verify`

### Profile page — new Security section

- Show 2FA enrollment status
- "Set up authenticator app" flow: display QR + secret → confirm code → show recovery codes
- "Disable 2FA" button with re-auth modal
- Recovery codes section: count of unused codes, "Regenerate codes" button with re-auth modal

### Auth settings page

- Add `two_factor_required` toggle (ManageSettings permission required)

### Global middleware / layout

- Handle `403 { error: "2fa_setup_required" }` globally
- Redirect to enrollment flow when detected

---

## Testing

### Unit tests

- `totp.rs`: secret generation, code verification, skew=1 acceptance, out-of-window rejection
- `mfa_challenge.rs`: token generation, expiry check, attempt exhaustion, email code hash replacement, recovery code single-use semantics

### Integration tests (in-memory SQLite, same pattern as `auth.rs`)

- `routes/mfa.rs`: successful verify (Totp, Email, RecoveryCode); wrong code increments attempt_count;
  5th failure poisons; expired token rejected; consumed token rejected; email endpoint returns 400 for
  missing user email; email endpoint returns 503 on SMTP failure
- `routes/me_2fa.rs`: enroll → confirm → status shows enrolled; disable requires re-auth; regenerate
  invalidates old codes; confirm under enforcement returns new full-session tokens

### Enforcement path

- Unenrolled user + enforcement on → login returns `setup_required` JWT
- `setup_required` JWT → blocked on non-enrollment routes (403)
- `setup_required` JWT → enrollment succeeds → full-session JWT returned
- Full-session JWT → all routes accessible
- Refresh of `setup_required` session before enrollment → new `setup_required` JWT
- Refresh of `setup_required` session after enrollment → full JWT (no `setup_required`)

### Rate limiting

- Exceed per-IP limit on `/auth/mfa/verify` → 429
- Exceed per-IP limit on `/auth/mfa/email` → 429

### E2E

- Extend `frontend/tests/e2e/public-entry.spec.ts` for new login flow

---

## Documentation Deliverables

- `docs/development/coding-standards.md` — no change needed (no new patterns introduced)
- `docs/end-user/` — new page: "Setting up two-factor authentication" (user-facing enrollment guide)
- `docs/admin/` — new section: "Enforcing 2FA for all operators"
- `CONTEXT.md` — no new domain terms required
- No ADR required (no surprising architectural trade-off)
