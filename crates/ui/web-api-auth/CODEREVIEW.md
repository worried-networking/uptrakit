# Code Review: uptrakit-web-api-auth

- **Review date**: 2026-03-05
- **Reviewer**: AI coverage analysis (cargo-llvm-cov)
- **Branch**: docs/test-coverage

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
