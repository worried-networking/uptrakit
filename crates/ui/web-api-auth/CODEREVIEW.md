# Code Review: uptrakit-web-api-auth

- **Review date**: 2026-03-05
- **Parallel review date**: 2026-03-06
- **Reviewer**: AI coverage analysis (cargo-llvm-cov), AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database)
- **Branch**: docs/test-coverage, docs/codereview-backend

## Test Coverage Analysis

Overall crate coverage: 2,977 / 3,575 lines (83.3%).

The crate has good overall coverage. The `oidc_state.rs` (98.7%), `rate_limit.rs`, `sessions.rs`,
and `permissions.rs` modules are well-tested. The gaps are concentrated in two files.

### Files Below 60% Coverage

| File | Coverage | Lines |
| --- | ---: | ---: |
| `auth/authentication.rs` | 51.7% | 356 |
| `settings_store.rs` | 35.3% | 399 |

### Critical Uncovered Paths

**[SECURITY] `authentication.rs` — `resolve_oidc_user` (51.7% coverage)**

The existing tests cover `AuthenticationSettings` (from_raw, default) and
`extract_mapped_roles` / `navigate_json_path` unit tests. The async DB-touching code in
`resolve_oidc_user` — the full 6-variant resolution logic — has no integration tests.

This is the most security-critical untested path in the auth subsystem. It controls:

- Email verification enforcement (rejects unverified emails)
- User-to-OIDC-link resolution (subject + provider lookup)
- Auto-creation of new users from OIDC claims
- Link-via-password detection (forces re-authentication)
- Deactivated user rejection

Recommended tests (priority order):

1. `email_verified = None` returns `EmailNotVerified` (prevents account takeover)
2. `email_verified = Some(false)` returns `EmailNotVerified`
3. Linked user with active account returns `LinkedUser`
4. Linked user with deactivated account returns `Deactivated`
5. No link, user found by email with password returns `LinkViaPasswordRequired`
6. No link, user found by email with other OIDC link returns `LinkViaOidcRequired`
7. No link, no user, auto-create enabled returns `NewUser`
8. No link, no user, auto-create disabled returns `NotAllowed`

**[SECURITY] `sync_oidc_roles` atomicity**

Must delete all existing roles and insert only the mapped ones. A partial application would
leave a user with a mix of old and new role assignments.

Recommended tests:

- Roles are fully replaced (not accumulated) on re-sync
- Empty role mapping returns without modifying `user_role` table
- Unknown claim values produce no role insertions

**[BUSINESS] `settings_store.rs` — settings persistence (35.3% coverage)**

`generate_or_load_jwt_key`, `load_settings_snapshot`, and the setting reconciliation logic
have low coverage. The `load_settings_snapshot` function reads all `SettingKey` variants from
the DB and builds a `SettingsSnapshot`.

Recommended tests:

- `load_settings_snapshot` with no settings returns all defaults
- `load_settings_snapshot` with partial settings merges correctly
- `generate_or_load_jwt_key` creates key on first call, loads on second
- `save_setting` + `load_setting` round-trip for encrypted values

## Security

### Strengths

All auth mechanisms rated as GOOD by the parallel security review (2026-03-06):

- **JWT**: Short-lived tokens (15 min), validates `exp`/`iss`/`aud`, HMAC signing via
  `jsonwebtoken`, signing key stored encrypted in DB with column-specific AAD.
- **Password hashing**: Argon2id with OWASP parameters (19 MiB, 2 iterations), random salt,
  constant-time verification, password length validation (8-1024 chars).
- **Sessions**: Refresh tokens stored as SHA-256 hashes, 7-day expiry, atomic rotation in DB
  transaction (HA-safe), revocation checks before rotation, DB CHECK constraint for OIDC
  session integrity.
- **Cookies**: HttpOnly, Secure, SameSite=Strict, path-scoped to `/api/v1/auth`.
- **Token denylist**: Per-JTI and per-user revocation, monotonic `iat_cutoff`, DB-backed with
  in-memory cache, cross-instance propagation via NATS.
- **Rate limiting**: DB-backed sliding-window counter using atomic SQL upsert, HA-safe,
  TOCTOU-resistant, fully parameterized raw SQL.

### Issues

No security issues found.

## Tests

### Issues

**[HIGH]** `settings_store.rs` -- 580 lines, 7 `OffsetDateTime::now_utc()` calls, zero
tests. Contains `generate_or_load_jwt_key`, `load_settings_snapshot`, and setting
reconciliation logic. *Found in parallel tests review (2026-03-06).*

**[MEDIUM]** DB row backdating in `auth/oidc_state.rs` (7+ instances at lines 628-639,
669-679, 702-712, 838, 916) and `auth/device_flow.rs` (1 instance at lines 236-255) violates
the documented testing philosophy. `docs/development/testing.md` explicitly states: "Do not
backdate database rows directly." These stores (`OidcFlowStore`, `AccountLinkStore`,
`OidcTokenExchangeStore`, `DeviceFlowStore`) all use `OffsetDateTime::now_utc()` in
production code without clock injection. The correct fix per the canonical pattern would be to
add `with_clock` constructors (like `RateLimitStore`) and advance the injected clock in tests.
*Found in parallel tests review (2026-03-06).*

---

## Review — 2026-03-10

### Summary

Focused review of the authentication module covering token hashing design, secret type usage,
and error-conversion redundancy. The 2026-03-06 findings around test coverage and DB backdating
remain open and are confirmed below. New findings are additive.

### Strengths

- JWT validation is rigorous: both `aud` and `iss` in `required_spec_claims`, 15-minute expiry,
  JTI + user-level denylist for immediate revocation. `test_decode_legacy_token_without_aud_rejected`
  verifies legacy tokens are rejected.
- Refresh token rotation uses an atomic DB transaction (begin → revoke old → insert new →
  commit). `test_rotate_same_token_twice_fails` validates replay detection.
- Password hashing uses Argon2id with OWASP-recommended parameters (19 MiB memory, 2
  iterations, parallelism 1), random salt, constant-time verify, 1,024-char max.

### Concerns

#### Security

| Severity | Location | Finding |
| --- | --- | --- |
| **Medium** | `auth/token.rs:17-21` | **Un-keyed SHA-256 for token hashes**: `hash_token` uses plain SHA-256 with no salt and no HMAC secret for API token and enrollment secret hashes. With 256-bit entropy the tokens resist online brute-force, but a compromised read-only DB replica allows instant verification of any token known to the attacker (deterministic un-keyed hash). Replace with `HMAC-SHA256(server_secret, token)` where the server secret is derived from or co-located with the master encryption key. |
| **Low** | `auth/refresh_cookie.rs:19,26` | **Silent empty `Set-Cookie` on `HeaderValue` parse failure**: `HeaderValue::from_str(&value).unwrap_or_else(\|_\| HeaderValue::from_static(""))` silently emits an empty header rather than surfacing a bug. The error branch is unreachable in practice (base64url tokens are always valid), but the fallback would suppress a potential header-injection signal. Replace with `expect("refresh token cookie contains only base64url characters")`. |

#### Code and Logic Consistency

| Severity | Location | Finding |
| --- | --- | --- |
| **Low** | `auth/oidc_state.rs:20-38`, `auth/error.rs` | **Redundant `#[from]` and `impl_report_conversion!`**: both attributes cover the same three conversion types. The `#[from]` derives are unused because the codebase exclusively uses `context_to()`. Remove the redundant `#[from]` attributes where `impl_report_conversion!` covers the same path. |
| **Low** | `CreatedApiToken` | **`plaintext_token: String` should be `SecretString`**: this is the one-time plaintext API token returned to the user. `SecretString` prevents accidental logging and zeroes the value on drop. |

#### Tests (confirmed from 2026-03-06)

| Severity | Finding |
| --- | --- |
| **High** | `settings_store.rs` — 580 lines, 7 `OffsetDateTime::now_utc()` calls, zero tests. `generate_or_load_jwt_key`, `load_settings_snapshot`, and setting reconciliation logic are entirely untested. *Confirmed.* |
| **Medium** | DB row backdating in `auth/oidc_state.rs` (lines 628-639, 669-679, 702-712, 838, 916) and `auth/device_flow.rs` (lines 236-255). Add `with_clock` constructors as per the `RateLimitStore` pattern and advance the injected clock in tests rather than backdating DB rows. *Confirmed.* |

---

## 2026-03-10 Comprehensive Review Update

Comprehensive 12-dimension review covering architecture, security, code quality, tests, HA,
database, coding standards, extensibility, consistency, idiomatic Rust, references and heap,
and maintainability. Only findings not already recorded above are listed.

### Dimension: Security (D2)

#### Strengths

- OIDC auto-linking is disabled by default. A user authenticated via OIDC whose email
  matches an existing local account is not silently linked; the flow requires explicit
  password re-authentication (`LinkViaPasswordRequired` resolution). This prevents account
  takeover via a compromised identity provider that emits arbitrary email claims.
- Registration token secrets are verified using Argon2id (same parameters as password
  hashing), not plain string comparison. A leaked hash does not allow offline brute-force
  within practical time bounds.
- First-user registration race condition is prevented: the handler checks user count inside
  a serialized transaction, so concurrent first-registration requests cannot both succeed.

#### Issues

**[LOW]** `auth/token_denylist.rs` -- Token denylist uses `tokio::sync::RwLock` instead of
`parking_lot::RwLock`. The lock guards are dropped before `await` points today, making this
functionally correct but inconsistent with the project-wide `parking_lot` standard. Replace
for consistency and to prevent future regressions.

**[LOW]** `auth/device_flow.rs` -- Device flow user code entropy is adequate (8 uppercase
alphanumeric characters, ~41 bits) for a short-lived user-facing code, but sits below the
commonly recommended 20-bit minimum for device codes with rate limiting. The existing
rate-limit enforcement on the poll endpoint mitigates brute-force risk.

### Dimension: Code Quality (D3)

#### Strengths

- `AuthError` uses semantic variants (`InvalidCredentials`, `TokenExpired`, `Deactivated`,
  `EmailNotVerified`, `RateLimited`, etc.) that map cleanly to HTTP status codes. No
  catch-all `Other(String)` variant exists for auth errors, forcing explicit handling of
  every failure mode.

#### Issues

**[LOW]** `auth/error.rs` -- Dual `#[from]` and `impl_report_conversion!` on 5 `AuthError`
variants (beyond the 3 already noted in the 2026-03-10 review above). The redundancy covers
`DbErr`, `CryptoError`, `JwtError`, `OidcError`, and `SessionError`. The `#[from]` derives
are unused because all call sites use `context_to()`. Remove the `#[from]` attributes to
eliminate the dead conversion paths and make the error-propagation strategy unambiguous.

### Dimension: Tests (D4)

#### Issues

**[LOW]** `settings_store.rs` -- Lacks any unit or integration tests. *Confirmed from
2026-03-06 finding; no change in status.*
