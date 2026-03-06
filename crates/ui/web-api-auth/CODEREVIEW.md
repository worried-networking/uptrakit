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
