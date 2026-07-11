# Session & Token Integrity Fixes — Design

**Date:** 2026-07-11 **Status:** Draft **Source:** `.superpowers/audit-2026-07-11.md` — HIGH "TokenDenylist::deny_user
persists with plain INSERT so repeat revocations silently lost" + MEDIUM "oidc_exchange and oidc_link mint sessions
without re-checking user.is_active" + MEDIUM "sessions table has no production cleanup" + MEDIUM "DB failure while
loading permissions silently yields a zero-permission session".

Scope note: the first-user-bootstrap cluster (three divergent copies, error-swallow lockout, settings-before- commit)
is a separate coupled refactor and gets its own spec — deliberately excluded here.

## Problem

Four independent auth-subsystem defects, each small and mechanical:

1. **Denylist upsert lost (HIGH, security).** `TokenDenylist::deny_user` gates on a **per-instance** in-memory
   monotonic check (`if iat_cutoff > entry.iat_cutoff`) and only then persists via `model.insert(db)` into
   `revoked_token_users` (PK `user_id`). When a row already exists (second higher revocation for the same user —
   logout after password change, two logouts within the purge window), the `insert` hits a PK conflict, downgraded to
   `tracing::warn!`, leaving the DB with the OLD (lower) `iat_cutoff`. On restart / other HA instance,
   `load_from_db()` seeds the stale cutoff — access tokens issued between the two revocations are accepted again for
   their full 15-minute lifetime, the exact window the denylist exists to close. The doc comment already claims
   "upsert with monotonic iat_cutoff wins"; the DB write doesn't. **Multi-instance race (confirmed during review):**
   the in-memory gate is per-process — instance B with no cache entry for a user will pass its own gate for a _lower_
   cutoff and, with an unconditional on-conflict update, regress instance A's higher DB cutoff. The fix must guard at
   the SQL layer, not rely on the per-instance gate.
2. **OIDC mint skips `is_active` (MEDIUM, security).** `mint_oidc_auth_response` (oidc_auth.rs) loads the user by id
   and never checks `is_active`; the oidc_link password-ownership path skips it too. Every other session-minting path
   checks (login, refresh, mfa, `handle_linked_user`). A user deactivated within the 60s exchange / 10min link window
   mints a full session (+15min token). **Ordering hazard, found reading the code:** `mint_oidc_auth_response` creates
   the refresh token _before_ it loads the user — a naive is_active check placed after would leave a live refresh
   token behind.
3. **Sessions never cleaned (MEDIUM, stability).** `cleanup_expired_sessions` has only test callers; the scheduler's
   `AuthCleanupExecutor` purges every other auth table but omits `sessions`. Every login and every refresh-rotation
   inserts a row; none are deleted — unbounded growth degrades the `RefreshTokenHash` lookup on every verify/rotate
   and bloats SQLite backups.
4. **Permission-load DB error → empty session (MEDIUM, stability/security).** `login`/`register`/`refresh`/
   `mint_oidc_auth_response` treat a `get_user_permissions` DB error as `unwrap_or_default()` / `vec![]` and issue
   tokens anyway. The MFA path made the correct choice (`build_full_session` returns 500). On a transient DB error the
   user gets a valid 15-min JWT where every request 403s; worst on refresh, where the old token is already consumed.
   It also masks real DB outages as permission errors.

## Approach

Four contained fixes; each aligns an outlier with a correct sibling already in the codebase.

### 1. Real upsert for `deny_user`

Replace `model.insert(db)` with a single-statement SeaORM upsert **with a monotonic WHERE guard** — **required**, not
optional, because the per-instance gate does not stop a second instance regressing the cutoff (above):
`Entity::insert(model)` with `.on_conflict(OnConflict::column(Column::UserId)` +
`.update_columns([Column::IatCutoff, Column::PurgeAfter])` +
`.action_and_where(Expr::col(Column::IatCutoff).lt(Expr::val(iat_cutoff)))` — the update fires only when the
incoming cutoff exceeds the stored one. `OnConflict::action_and_where` is confirmed
present in the pinned sea-query (transitive via sea-orm 2.0.0-rc.41). Single statement, no transaction, no BEGIN
IMMEDIATE concern, correct under concurrent multi-instance writes. Note: the ~13 existing `on_conflict` sites use
plain `update_columns` with **no** value-guard — this is the first guarded upsert in the codebase, justified because
those sites overwrite unconditionally by design whereas this one must be monotonic across instances (the per-instance
in-memory gate cannot enforce it). `deny_user_except` **delegates to `deny_user`** (verified — it calls
`self.deny_user(...)` then mutates an in-memory allowlist only, no DB insert of its own), so it is fixed
automatically; no second site. The doc comment finally matches the code.

### 2. `is_active` check in `mint_oidc_auth_response`, before minting

Load the user and check `is_active` **before** creating the refresh token (reorder — the load currently sits after;
verified feasible: `create_refresh_token` depends only on `user_id`/`provider_id`, both function params, not on the
user load). On `!is_active`, return `error_response(StatusCode::FORBIDDEN, "User is deactivated")` — mirroring the
**refresh** path's choice (auth.rs:1942/2093), _not_ login/mfa's generic `UNAUTHORIZED "Invalid credentials"`. The
divergence is intentional and must not be "fixed" to match login: OIDC (like refresh) is post-identity-proof, so
revealing "deactivated" leaks nothing an authenticated user can't already infer. This one site covers oidc_exchange,
oidc_link, and complete-registration — the oidc_link password path funnels through `mint_oidc_auth_response`
(verified: it only sets `verified`/`denied_reason_code`, then calls `mint_oidc_auth_response` at oidc_auth.rs:1891),
so no separate fix site.

### 3. `sessions` cleanup in `AuthCleanupExecutor`

Add `Session::delete_many().filter(session::Column::ExpiresAt.lt(now)).exec(&txn)` to `AuthCleanupExecutor::execute`,
inside the existing transaction alongside the other auth tables, following their exact `.context_to()?` shape.
Retention on `revoked_at` is out of scope (the audit's "optional") — expiry-based deletion is the fix;
revoked-but-unexpired rows are still within their valid lifetime and must not be deleted. Mirror the existing executor
tests (seed an expired + a live session, run, assert the expired is gone and the live remains).

Two operational notes (contrarian): `sessions` is the only table in this executor that grows on **every** login _and_
refresh rotation, so the first cleanup after deploy may face a large backlog — a single `delete_many` holds a SQLite
write lock for its duration (one-time multi-second stall, not data loss). Do **not** pre-batch (it would make
`sessions` the lone bespoke-loop table here); instead (a) verify a `sessions.expires_at` index exists so the delete
doesn't full-scan (highest-leverage mitigation — add the index migration if absent), and (b) note a `LIMIT`-batched
variant as the escape hatch only if production shows first-run lock contention.

### 4. Permission-load error → 500

**Six sites** (grep-confirmed): `register` auth.rs:412, `login` auth.rs:662, **two** refresh-adjacent blocks
auth.rs:1947 **and** 2109, plus `mint_oidc_auth_response` oidc_auth.rs:2056. Base fix: replace the `Err → vec![]` /
`unwrap_or_default()` arm with `return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")` —
mirroring `build_full_session`'s **value**, not its shape (these handlers return bare `Response`, so
`return error_response(...)` directly, no `Err(...)` wrapper).

**Refresh-path caveat (contrarian-driven — a blind 500 here strands the user).** At the refresh sites the token
rotation has **already committed** before the permission load (verified: `rotate_refresh_token` commits, then
`is_active` at auth.rs:2093, then permissions at 2109) — so today's `vec![]` fallback still returns a _working new_
refresh token; a bare 500 aborts after rotation but before the token reaches the client, forcing a full re-login on a
transient blip. Fix consistently with the `is_active` branch immediately above (auth.rs:2093-2105), which **revokes
the just-rotated token then returns the error**: on permission error, `revoke_refresh_token(&new)` then 500 — a
consistent state (no dangling token), matching the file's own established pattern. Preferred where feasible: move the
permission load _before_ the rotation so failure aborts before the old token is consumed (user keeps the old token,
retries) — apply the reorder if the rotation doesn't depend on the permission set; otherwise revoke-then-500. This
rule applies to **any** of the six sites where a mint/rotation precedes the permission load;
`login`/`register`/`mint_oidc` (no prior irreversible commit at the load point) take the plain 500. Copy the
`is_active` branch's failure semantics exactly: its `revoke_refresh_token` is **best-effort** (`let _ = …`), so the
revoke on the permission-error branch must swallow/log a revoke failure too, not `?`-propagate it into a different
error path (a transient outage can fail the revoke as well). The `tracing::error!` stays at the four auth.rs sites;
the oidc site has **no** log today (`.unwrap_or_default()`) — add one. Implementer greps all `get_user_permissions`
fallbacks rather than trusting the enumeration.

## Tests

All in the existing auth test modules (web-api-auth unit tests + web-api route tests via the `TestApp` harness;
DB-backed, no `start_paused`, no tokio-time APIs — snapshot rules):

1. **Denylist upsert (the HIGH regression):** `deny_user(u, cutoff1)` then `deny_user(u, cutoff2>cutoff1)`; simulate
   restart by constructing a fresh `TokenDenylist` and calling `load_from_db()`; assert the seeded cutoff is
   `cutoff2`, not `cutoff1`. Also assert a lower second cutoff does not regress the DB row.
2. **OIDC is_active:** deactivate a user, drive `mint_oidc_auth_response` (via the exchange and link paths); assert
   `FORBIDDEN` and that **no** session row / refresh token persists (guards against the ordering hazard).
3. **Session cleanup:** the executor test above.
4. **Permission-load 500:** inject a permission-query failure using the codebase's established idiom —
   `db.execute_unprepared("DROP TABLE role_permission")` on the `TestApp` connection after seeding the user/ session,
   before invoking the handler (10+ existing usages of this pattern in the route tests). Assert 500 and no token
   issued. Cover the refresh path (worst consequence) at minimum; login/register/mint_oidc are the same mechanism and
   cheap to add. No manual-verification escape hatch — the pattern makes all four testable. Also add the
   multi-instance denylist regression: `deny_user(u, 100)` then a _fresh_ `TokenDenylist` (empty gate)
   `deny_user(u, 50)` → assert the DB row stays 100 (the WHERE guard, not the gate, is what holds). **This guard test
   must run against Postgres, not only SQLite** — the whole risk is `ON CONFLICT DO UPDATE … WHERE` dialect
   divergence + the `i64`→`BIGINT` bind on the strict engine; the rest of the auth tests are `sqlite::memory:`-only,
   so this one needs an explicit Postgres backend (the crate's integration-test path) or the "correct under concurrent
   multi-instance writes" claim is never exercised on the engine HA uses.
5. Refresh permission-error: assert the just-rotated token is revoked (or the load reordered before rotation) on the
   500 branch — no dangling committed-but-unreturned token.

## Documentation deliverables

- `deny_user` doc comment already claims the upsert behavior — no change needed beyond making it true; add a one-line
  note if the monotonicity relies on the in-memory gate rather than a SQL guard.
- No API/wire/OpenAPI change (behavior for legitimate flows is unchanged; deactivated-OIDC and DB-error paths change
  from wrong-success to correct-error — no schema surface change).
- No new ADR: four conformance fixes, no architectural decision.
- `docs/security/*` auth doc: if a token-revocation or session-lifecycle description exists, verify it still reads
  true after these fixes (likely already describes the intended behavior these fixes deliver).

## Out of scope / deferred

- First-user bootstrap cluster (separate spec — three-copy consolidation, error-swallow lockout,
  settings-before-commit).
- `revoked_at`-based session retention window (expiry-based cleanup is the fix).
- Proactive/global session-count limits or per-user session caps (not a reported defect).
- Denylist `cleanup_expired` sweep changes beyond the upsert (the purge path is separate and not faulted).
