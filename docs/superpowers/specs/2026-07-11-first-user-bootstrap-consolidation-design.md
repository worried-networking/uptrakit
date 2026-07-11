# First-User Bootstrap Consolidation — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — three coupled MEDIUM findings the audit itself says to fold
together: "First-OIDC-user bootstrap deletes default roles then swallows owner-role/setup failures, committing a
user with zero roles and closed registration"; "First-user bootstrap logic exists in three divergent copies
across the auth routes"; "In-memory registration settings are published before the first-user transaction
commits". This is the deferred cluster from the session-token-integrity spec.

## Problem

The "is this the first user → assign all owner roles + close registration" bootstrap sequence has **three**
call sites across `crates/ui/web-api/src/routes/{auth.rs,oidc_auth.rs}` — and **all three swallow the failure**
(the shared helper propagates via `?`, but every caller catches and demotes to `is_first_user = false`). The
divergence and the swallow together are where the lockout/demotion bugs live:

- `register()` (password) calls the shared `handle_first_user_setup` (auth.rs:2192) but **swallows its error**
  (auth.rs:348-357): on failure `is_first_user = false`, so the first user falls through to `assign_viewer_role`
  and is committed with only the viewer role — not owner, not locked out, but silently demoted.
- `oidc_complete_registration` (oidc_auth.rs:1610-1624) reuses the shared helper but **swallows its error
  identically** (`Err(e) => { tracing::error!(…); false }`, verified during review) — a **fourth** instance of
  the same lockout bug, not a safe reference. It too falls through to `assign_viewer_role` and commits (line
  1653).
- `handle_new_user` (OIDC callback, oidc_auth.rs:1149-1204) **hand-rolls its own copy**: it deletes the default
  `user` role assigned by `resolve_oidc_user`, then calls `assign_owner_roles` and `complete_initial_setup` but
  only `tracing::error!`s their failures and returns `Ok` — so `execute_oidc_resolution` (line 922) still commits
  the transaction. If `assign_owner_roles` fails (it errors "role not found" on any of 8 hard-coded role names,
  auth.rs:2221-2230), the first user is committed with **zero roles** while `complete_initial_setup` has closed
  registration — a **permanently locked-out instance** with no way to register another account.

Independently, the shared `handle_first_user_setup` publishes the in-memory registration snapshot **inside the
caller's uncommitted transaction**: `settings.set_registration(reg)` (auth.rs:2208) calls `send_modify` on the
process-wide watch channel immediately. `handle_new_user` does the same (oidc_auth.rs:1188). If the subsequent
`txn.commit()` fails (both `register()` at auth.rs:395 and `oidc_complete_registration` at oidc_auth.rs:1653 have
explicit `registration_commit_failed` paths), the DB rolls back — no user, settings rows reverted — but the
in-memory snapshot keeps `RegistrationMode::Closed`. Every later registration is rejected until process restart
or an unrelated settings reload: **initial setup bricked exactly when the instance has zero users**.

All three are one refactor: a single bootstrap helper that (a) optionally clears pre-assigned default roles, (b)
propagates every error for atomic rollback, (c) hands the settings publish back to the caller to run only after
commit.

## Approach

### One helper, error-propagating, publish-deferred

Reshape `handle_first_user_setup` into the single implementation all three entry points call:

```rust
pub async fn handle_first_user_setup(
    txn: &impl ConnectionTrait,
    settings: &Settings,
    tenant_id: Uuid,
    user_id: Uuid,
    threshold: u64,
    clear_default_roles: ClearDefaultRoles,   // two-variant enum, not a bare bool
) -> crate::auth::Result<Option<RegistrationSettings>>
```

`ClearDefaultRoles::{Clear, Keep}` — a small two-variant enum (the coding-standards' named "boolean-like enum"
pattern, matching the codebase's ~11 `*Mode` enums) so call sites read `ClearDefaultRoles::Clear` not a bare
`true`/`false`. `Option<RegistrationSettings>` as the return is idiomatic outcome-value (precedent:
`fire_software_item_lifecycle -> Option<SoftwareItemPatch>`), and `is_first == reg.is_some()` is a genuine
derivation, not a nullable flag.

- Count users; if `> threshold`, return `Ok(None)` (not the first user — caller assigns the default/viewer
  role as today).
- On `ClearDefaultRoles::Clear`: `UserRole::delete_many().filter(tenant).filter(user).exec(txn).await?`
  **before** `assign_owner_roles` (this is the OIDC-only step, previously the swallowed `let _ = …delete…`; now
  error-propagating). The delete is idempotent and safe for the password path too, but gating it keeps the
  password path's behavior byte-identical.
- `assign_owner_roles(txn, …).await?` and `reg.complete_initial_setup(txn, tenant).await?` — both already use
  `?`; the change is that **callers stop swallowing** the propagated error.
- Return `Ok(Some(reg))` — the closed-registration snapshot to publish. **`set_registration` is removed from
  this function** entirely; publishing is the caller's post-commit responsibility. Timing verified feasible:
  `complete_initial_setup(&mut self, db, tenant)` mutates `reg` in place *during* the transaction, and
  `RegistrationSettings` is `Clone` and `Settings::registration()` returns an owned clone — so the
  already-mutated, detached `reg` survives past commit with no borrow held on the transaction.

### Call-site changes (all three commit owners publish post-commit)

- `register()` (returns `Result<_, ApiError>`): replace the swallow (auth.rs:348-357) with **`?`** — the helper
  returns `crate::auth::Result<_>` = `Report<AuthError>`, and `impl From<Report<AuthError>> for ApiError`
  (mappings.rs:740) auto-converts it to a 500; no hand-rolled `error_response`. The `?` early-return happens
  before `txn.commit()`, so the transaction RAII-rolls-back atomically (no half-provisioned first user). On
  `Ok(Some(reg))`, stash `reg` and call `settings.set_registration(reg)` **only after** `txn.commit()` succeeds
  — this mirrors the audit `hook.flush_after_commit()` already present a few lines below (auth.rs:409), the
  in-file precedent for "commit owner runs the deferred side effect post-commit". On `Ok(None)`, the existing
  `assign_viewer_role` path is unchanged. Pass `ClearDefaultRoles::Keep`.
- `handle_new_user` (OIDC): call the shared helper with `ClearDefaultRoles::Clear`, drop the hand-rolled body.
  This function returns raw `Response` (via `Redirect`), **not** `ApiError`, so it can't use `?`-conversion —
  keep the manual `on Err → return Redirect::to("/login?error=oidc_internal_error").into_response()`. (Do not
  introduce a `match e.current_context()` on `AuthError` variants here — a plain error→redirect avoids tripping
  the `check_legacy_error_matches.sh` gate that bans such matches in `routes/`.) The redirect lets
  `execute_oidc_resolution` (one call layer up, and the commit owner — verified shallow: `oidc_callback` →
  `execute_oidc_resolution` → `handle_new_user`) rolls back. Change **only the `NewUser` match arm** — the
  `LinkedUser` and `LinkViaPasswordRequired` arms carry no first-user concept and are untouched. Thread the
  `Option<RegistrationSettings>` up through `handle_new_user`'s return (`(Uuid, bool)` →
  `(Uuid, Option<RegistrationSettings>)`; first-user flag is `reg.is_some()`) so `execute_oidc_resolution`
  publishes after its commit.

  **Not touched (scoped out, contrarian-established):** `handle_new_user` calls `sync_oidc_roles` after owner
  assignment, and for a provider with role-mapping configured, `sync_oidc_roles` `delete_many()`s all roles and
  reinstalls only the mapped set (authentication.rs:320-338) — so a role-mapping provider governs the user's
  roles on **every** login (first *and* subsequent, since later logins route `LinkedUser` →
  `handle_existing_user` → `sync_oidc_roles` unconditionally). This is pre-existing, intentional
  provider-as-role-authority behavior, orthogonal to the swallow/divergence/publish findings this spec fixes. A
  first-login-only skip was considered and rejected: it only defers the downgrade to login #2, adding leaky
  complexity without closing the hazard. Correct scope: this spec does **not** change role-sync policy. The
  "first user gets 8 owner roles" guarantee therefore holds **on the bootstrap transaction and absent a
  role-mapping provider**; making owner roles sticky against provider mappings (union-not-replace, or exempting
  the bootstrap user) is a separate role-authority decision for a follow-up, not a bug fold-in here.
- `oidc_complete_registration` (returns `Result<_, ApiError>`): **same fix as `register()`, not merely a
  return-type tweak** — it currently swallows the helper error (oidc_auth.rs:1620-1623) and is a fourth bug
  instance. Replace the `Err(e) => false` arm with `?` (auto-converts to 500 via the same `ApiError` `From`
  impl) **before** its commit (oidc_auth.rs:1653), and move `set_registration` to after that commit. Pass
  `ClearDefaultRoles::Keep` — verified: it creates the user via a direct `user::ActiveModel::insert`
  (oidc_auth.rs:1554), bypassing `resolve_oidc_user`, so no default `user` role is pre-assigned.

Net: one bootstrap body, uniform error propagation (atomic rollback on any failure), and the process-wide
registration snapshot mutated only after the DB durably reflects it.

### Notes

- No new SQLite BEGIN IMMEDIATE concern introduced: the `user_count` read then role writes already run inside
  the caller's existing transaction; that transaction's mode is unchanged by this refactor (and the tx-mode
  conformance spec handles the workspace-wide begin() policy separately).
- The 8 hard-coded role names in `assign_owner_roles` are not touched — but their failure now correctly aborts
  the whole registration instead of committing a broken first user. (A follow-up could derive them from a role
  constant/enum; out of scope here — the fix is making the failure loud, not eliminating the list.)
- **Behavior-preserving otherwise:** no new audit-catalog site (the existing `user_create` stateful emit in
  each handler is unchanged — this refactor moves no state-change out of the audited transaction), the
  process-wide first-user detection stays a global `User::find().count()` (users are not tenant-scoped for this
  check), and the return type stays `crate::auth::Result<_>` (`?`-propagation, the existing error convention).
- **Four audit `details` sites consume the first-user bool** (`auth.rs:373`, `oidc_auth.rs:160`, `:933`,
  `:1677`) — each maps `reg.is_some()` back to a `bool` for its audit call. **Leave alone** the unrelated
  same-named `is_first_user` at `oidc_auth.rs:1052` (a separate `count()` inside `needs_token_for_oidc`, nothing
  to do with the bootstrap helper) — do not conflate it.

## Tests

Route tests via the `TestApp` harness (DB-backed, no `start_paused`/tokio-time — snapshot rules):

1. **Password first-user owner-role failure → atomic rollback:** drop/rename a required role row so
   `assign_owner_roles` errors; register the first user; assert 500, **no user row committed**, registration
   still Open (settings snapshot unchanged), and the process-wide `settings.registration()` still Open.
2. **Both OIDC paths same:** the OIDC-callback path (`handle_new_user`) **and** `oidc_complete_registration` —
   role failure → error/redirect, no user committed, registration Open in DB and in the snapshot. Cover both;
   `oidc_complete_registration` is the fourth-instance path the original spec draft wrongly exempted, so its
   rollback needs its own assertion, not just `register()`'s.
3. **Commit-failure snapshot integrity:** force `txn.commit()` to fail after a successful first-user setup (e.g.
   the harness's DROP-TABLE idiom on a table the commit touches, or a constraint trip); assert the in-memory
   `settings.registration()` is **not** left Closed (the publish never ran).
4. **Happy path — exact role set (no role-mapping provider):** first user (password and OIDC via a provider
   *without* role mapping) has **exactly** the 8 owner roles (no extras, no missing — locks in the
   `Keep`/`Clear` invariants), registration closes, snapshot Closed after commit. Second user gets the
   default/viewer role only.
5b. **Role-mapping provider governs roles (documents scoped-out behavior):** a first OIDC user via a provider
   *with* role mapping ends with the mapped roles, not the 8 owner roles — asserting this pins the known,
   intentional `sync_oidc_roles` behavior so a future reader sees it is deliberate, not a regression this spec
   introduced.
5. **OIDC default-role clear:** first OIDC user does not retain the `resolve_oidc_user` default `user` role
   alongside the owner roles (the `clear_default_roles` step ran and propagated cleanly).

## Documentation deliverables

- `handle_first_user_setup` doc comment: new signature, the `ClearDefaultRoles` enum, and the **contract that
  the caller must publish the returned `RegistrationSettings` only after commit** (the load-bearing invariant —
  a future caller that publishes early reintroduces the brick bug). Also document *why* deferring is safe: on
  restart, `RegistrationSettings::initialize` re-derives the mode from `User::find().count()` (zero users →
  fresh invite token + `Invite` mode; otherwise from DB), so a lost post-commit publish self-corrects and cannot
  brick a zero-user instance — a future reader must not "fix" this by persisting the in-memory snapshot naively.
- No API/wire/OpenAPI surface change (request/response shapes for register and the OIDC callback are unchanged;
  the observable delta is that a broken first-user setup now fails loudly instead of committing a demoted or
  role-less user).
- No new ADR: three conformance/consolidation fixes in one subsystem, no architectural decision. If a
  first-run/bootstrap operator doc exists under `docs/`, verify it still reads true (locate during
  implementation).

## Out of scope / deferred

- Deriving `assign_owner_roles`' role list from a constant/enum instead of 8 hard-coded strings (the fix makes
  its failure loud; eliminating the list is a separate cleanup).
- Splitting the large `auth.rs`/`oidc_auth.rs` route files (mixing handlers + audit helpers + bootstrap domain
  logic) — the audit's architecture finding notes it; this spec consolidates the bootstrap logic into one
  function but does not relocate files.
- The session/token integrity fixes (separate committed spec).
- Any change to `resolve_oidc_user`'s default-role assignment (the clear step compensates for it; changing it is
  broader).
- Making owner roles sticky against `sync_oidc_roles` for role-mapping providers (a role-authority policy
  decision — the provider is currently the source of truth for a mapping-configured user's roles on every
  login; changing that is a separate follow-up, not part of the swallow/divergence/publish fixes here).
