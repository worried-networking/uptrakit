# Code Review: `uptrakit-web-api-auth`

- Review date: 2026-03-17
- Scope: full 14-dimension review of all ~20 .rs files

## Summary

The authentication crate is in strong shape. Token revocation persistence, OIDC helper flows,
rate limiting, and settings-store behavior are all well-implemented and thoroughly tested. JWT
validation enforces issuer and audience claims with legacy token rejection. OIDC state stores use
atomic delete-on-consume semantics that are HA-safe. The one active finding from the prior review
(OIDC state store conflation of expired and non-existent tokens) remains valid.

## Strengths

- Auth behavior is cleanly separated from transport and query logic. The crate exposes a small,
  focused public API: `auth`, `setting_key`, `settings_store`, `error_response`.
- Rate limiting uses atomic upsert semantics with fully parameterized SQL for both PostgreSQL and
  SQLite. The sliding-window counter is correct and backend-agnostic. An injectable clock function
  enables deterministic time-based testing without `start_paused`.
- JWT validation enforces `iss="uptrakit"` and `aud=["uptrakit"]` claims via `required_spec_claims`.
  Legacy tokens without these claims are explicitly rejected. This is tested with a synthetic
  legacy token struct.
- Argon2id password verification uses constant-time comparison from the `argon2` library.
- OIDC CSRF state tokens use single-use, delete-on-consume semantics backed by the DB. The
  `take()` method uses an atomic conditional delete (`DELETE WHERE expires_at > now`) as the
  authoritative check, preventing TOCTOU races in HA deployments.
- All four OIDC state stores (`OidcFlowStore`, `AccountLinkStore`, `OidcTokenExchangeStore`,
  `OidcRegistrationStore`) follow the same proven pattern consistently.
- `OidcRegistrationStore` has a non-destructive `get()` method for validation-before-consume,
  enabling retry on validation failure without consuming the registration token.
- `TokenDenylist` supports both JTI-level and user-level revocation with monotonic cutoff
  semantics, DB persistence, cross-instance propagation, and periodic purge. Comprehensive test
  coverage includes DB-backed round-trip tests, remote revocation (memory-only), and
  `load_from_db` seeding.
- `resolve_oidc_user` rejects `email_verified != Some(true)` before any DB lookup, preventing
  account takeover via an IdP that omits or falsifies the claim.
- OIDC role sync (`sync_oidc_roles`) uses a replace-all strategy (delete then insert) which is
  correct for idempotent role mapping from IdP claims.
- Test coverage is comprehensive: password, session, token, OIDC flow, device flow, rate limiting,
  token denylist (in-memory and DB-backed), authentication settings, and role extraction are all
  covered.

## Active Findings

### [MEDIUM] OIDC state store does not distinguish expired tokens from never-existed tokens

- **Dimension**: security, user experience
- **Scope**: `crates/ui/web-api-auth/src/auth/oidc_state.rs:86-115` (`OidcFlowStore::take`)
- **Description**: `take()` returns `Ok(None)` for both an expired CSRF state token and a token
  that was never issued. The implementation first queries the row (`find_by_id`), then performs a
  conditional delete (`DELETE WHERE expires_at > now`). If the row exists but is expired, the
  delete affects 0 rows and the function returns `None` -- identical to the case where the token
  never existed.
- **Why it matters**: Callers cannot provide a specific "session expired, please log in again"
  message. For a security reviewer, the lack of a distinct expired-vs-unknown response makes it
  harder to audit the CSRF protection path.
- **Failure scenario**: A user's OIDC login times out (> 10 minutes); the callback handler returns
  a generic "invalid state" error rather than a user-friendly "session expired" message.
- **Fix**: Return a typed enum (`NotFound | Expired | Consumed(Data)`) from `take()` so callers
  can distinguish all three cases. The implementation already has all the information needed: the
  `find_by_id` result tells us whether the row exists, and the `rows_affected == 0` condition
  tells us whether it was expired.

### [LOW] `TokenDenylist` uses `tokio::sync::RwLock` instead of `parking_lot::RwLock`

- **Dimension**: coding standards
- **Scope**: `crates/ui/web-api-auth/src/auth/token_denylist.rs:5`
- **Description**: The project convention is to use `parking_lot::Mutex` (and by extension
  `parking_lot::RwLock`) in async code. The `TokenDenylist` uses `tokio::sync::RwLock`. However,
  inspection confirms that all write guards are properly dropped before `.await` points (the
  `deny_token`, `deny_user`, and `purge_expired` methods release the guard before performing
  any DB operations). `is_denied` is a pure read with no `.await` under the guard.
- **Why it matters**: While functionally correct, this deviates from the project-wide lock
  convention. `parking_lot::RwLock` would provide slightly better performance (no `.await` on
  `lock()`) and consistency with the rest of the codebase.
- **Failure scenario**: No functional failure. A future contributor might hold the `tokio` RwLock
  guard across an `.await` point without realizing the project convention forbids this pattern.
