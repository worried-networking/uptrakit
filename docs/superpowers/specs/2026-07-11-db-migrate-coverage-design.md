# db-migrate Table Coverage — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — CRITICAL "db-migrate silently drops 13 tables including 2FA secrets
and OAuth tokens".

## Problem

`db-migrate` (cross-backend data migration, e.g. SQLite → Postgres) copies only the tables enumerated in
`crates/shared/db/src/migrate_core_tables.rs`. That file maintains **four parallel hand-synced lists**: the
`CORE_COPY_ORDER` const plus the `copy!`/`clean!`/`verify!` macro invocation bodies. None were updated for
migrations added since ~May 2026, so 13 live tables are silently not copied — the destination gets them empty and
`verify()` doesn't notice (it only counts listed tables):

`user_totp`, `user_recovery_codes`, `mfa_challenges`, `oauth_clients`, `oauth_consents`,
`oauth_authorization_requests`, `oauth_authorization_codes`, `oauth_refresh_tokens`, `oauth_controller_instances`,
`instance_plugin_setting`, `service_merge_redirect` (11 core), plus `proxmox_scaling_defaults` and
`proxmox_scaling_item_overrides` (2 plugin-owned).

A user who runs `db-migrate` today loses all 2FA enrollment (TOTP secrets + recovery codes → account lockout) and
every OAuth client/consent/token.

The guard test that catches exactly this — `migration_coverage_complete`
(`crates/core/controller-runtime/src/db_migrate/tables.rs`) — is `#[ignore]`d, and CI runs `--ignored` tests only
for `uptrakit-integration-tests`, so the drift has been invisible since it started.

Two failures, two fixes: the missing tables (data), and the process that let four lists drift (structure). Per the
audit and the project's stability goal, the durable fix is the structural one.

## Approach

### 1. Single source of truth for the core table list

Replace the four hand-synced lists with the pattern the codebase **already ships for the identical problem one
directory over**: `PluginTableDescriptor::for_entity::<E>(name)`
(`crates/plugins/infrastructure/core/src/descriptor.rs` — a name + boxed `copy_batch`/`clean`/`verify` closures
monomorphised per entity, collected in an FK-ordered `Vec`; used today by
`proxmox_db_migrate_tables()`).

- Define a **local mirror** `CoreTableDescriptor` in `migrate_core_tables.rs` with the same shape. Mirror the
  plugin side's *split*, not its surface: `PluginTableDescriptor` boxes name-less, `DbErr`-returning inner
  helpers (`copy_one`/`clean_one`/`verify_one`, with verify returning the raw `(src, dst)` count pair), and the
  caller attaches the table name, builds `Report<TableMigrateError>`, and does the mismatch `bail!` outside the
  boxed closures. Restructure `migrate_table`/`clean_table`/`verify_table` the same way — boxing the current
  `Report`-returning, name-taking generics directly would hit a type mismatch. `name` stays `&'static str` (the
  guard test compares into a `HashSet<String>`). It must be a local mirror, not an import: shared-db cannot
  depend on `plugin-infrastructure-core` (dependency direction + the plugin-semantic-boundary CI gate).
- One `fn core_tables() -> Vec<CoreTableDescriptor>` — a single `vec![CoreTableDescriptor::for_entity::<Tenant>
  ("tenants"), …]` in FK-safe order — becomes the sole authority. `copy()` iterates it forward, `clean()` iterates
  it with `.iter().rev()` (derived reversal, not a second list), `verify()` forward. The `CORE_COPY_ORDER` const
  is replaced by the descriptor list; the guard test switches to `core_tables().iter().map(|d| d.name)`, and the
  `#[cfg(test)] pub(crate) use … CORE_COPY_ORDER as COPY_ORDER` re-export (plus its doc comment) in
  `controller-runtime/src/db_migrate/tables.rs` — the only other consumer, verified — is updated in the same
  change.
- Net effect: adding a table is **one line**; the four consumers cannot drift from each other again. The remaining
  drift axis (migrations add a table but nobody touches this file at all) is exactly what the guard test covers.

**Rejected alternative:** a new `for_each_core_table!` macro-with-callback generating the const + three macro
bodies. Works, but introduces a novel one-off macro shape with no project precedent, while the descriptor-vec
idiom is shipping code (`PluginTableDescriptor`) solving the same problem — reuse the idiom, don't invent a
second mechanism beside it.

### 2. Add the 11 missing core tables

Append to the single list in FK-safe positions derived from each entity's `Relation` definitions (verify against
the entity files during implementation, do not trust this prose): `user_totp`, `user_recovery_codes`,
`mfa_challenges` after `users`; `oauth_clients`, then `oauth_consents` and `oauth_authorization_requests`, then
`oauth_authorization_codes` (FKs to `oauth_authorization_requests`, not just clients/users) and
`oauth_refresh_tokens` (FKs to `oauth_consents`) — the intra-OAuth ordering matters, not just clients-before-rest;
`oauth_controller_instances`; `instance_plugin_setting`; `service_merge_redirect` after `services`.
Landmine for the roundtrip test: `oauth_refresh_tokens.parent_id` is self-referential and **deliberately not an
enforced FK** (token-rotation design, documented in the entity file) — do not derive ordering or test fixtures
from it.

### 3. Register the 2 proxmox scaling tables in the plugin descriptor

`proxmox_db_migrate_tables()` (`crates/plugins/infrastructure/proxmox/src/db_migrate.rs`) gains
`PluginTableDescriptor::for_entity::<proxmox_scaling_default::Entity>("proxmox_scaling_defaults")` and
`…::<proxmox_scaling_item_override::Entity>("proxmox_scaling_item_overrides")`, positioned per the file's
documented FK-safe ordering rules (both reference `plugin_configs`/`software_items` — core tables copied earlier —
so their relative position among plugin tables is flexible; confirm against entity Relations).

### 4. Make the guard un-skippable

Remove `#[ignore]` from `migration_coverage_complete`. It needs only in-memory SQLite and runs migrations once
(sub-second) — and running full migrations on `sqlite::memory:` is already the crate's *standard non-ignored*
test setup (`reconcile.rs`, `pki.rs`, `reencrypt.rs`, `embedded/provision.rs` all do it in plain
`#[tokio::test]`s); the "integration — slower than unit tests" label on this one test contradicts the crate's own
practice and testing.md's `#[ignore]` categories (Docker/external-process tests). Un-ignored, it runs in the
standard CI coverage job (`cargo llvm-cov --workspace --all-features`, which executes tests), in the pre-push
hook's `cargo test`, and on every local run — the list cannot silently drift again on any path that runs tests
(verified: controller-runtime depends on shared-db with `db-migrate` unconditionally and has `db-sqlite` in its
defaults, so both the pre-push feature set and the CI job compile and run this test).

Guarantee scope, stated honestly: the guard proves coverage **for the feature set it compiles under**. The CI
`--all-features` run is the authoritative pass; narrower local/pre-push runs cover correspondingly less. Tables
that exist only under features outside the build, hypothetical Postgres-only tables, and the hand-maintained
`AGENT_ONLY_TABLES` exclusion list remain outside the guard — a residual manual-review surface, named here so
"un-skippable guard" is not read as "nothing can ever drift". No separate CI step, no new gate script
(YAGNI: the existing test infrastructure is the gate once the attribute is gone).

Un-ignore the sibling `migrate_sqlite_to_sqlite_roundtrip` (`db_migrate/mod.rs`) in the same commit — same file,
same stale rationale, same in-memory-SQLite shape; leaving it ignored while un-ignoring its sibling would be an
unexplained inconsistency.

**Rejected alternative:** keep `#[ignore]` and add a dedicated CI step running it with `-- --ignored`. Strictly
worse: pre-push and local runs still skip it, and it adds CI-config surface for no benefit.

## Tests

1. `migration_coverage_complete` itself, now always-on — fails the build with the exact missing/extra table names
   whenever coverage drifts (this is the audit's headline regression guard; it currently fails, and passes after
   steps 2–3).
2. Add one focused roundtrip test (none exists in `migrate_core_tables.rs` today — verified): seed one row in
   each newly-covered sensitive table (`user_totp`,
   `user_recovery_codes`, `oauth_clients` + one dependent `oauth_refresh_tokens` row to exercise FK order,
   `service_merge_redirect`), run `copy()` then `verify()` between two in-memory SQLite DBs, assert counts
   match — and for `user_totp.secret`, assert **decryptability at the destination** (read the row back through
   the entity and expose the secret), not just count parity: `verify()` is count-only by design and structurally
   cannot detect ciphertext-context problems. Proves the FK-safe positions actually insert cleanly, not just that
   the names are listed. Implementation note to verify while writing this test: `register_column_aad_mappings()`
   (`controller-runtime/src/reencrypt.rs`) does not register `user_totp`/`user_recovery_codes` columns — if the
   write path binds a column AAD the entity read path can't resolve, that is a **pre-existing** bug independent
   of db-migrate (ciphertext copies verbatim; anything decryptable at the source stays decryptable at the
   destination under the same master key). If confirmed, add the missing `ColumnAadEntry` registrations in the
   same PR (one line each); if the paths are symmetric (both empty-AAD), note that and move on. Two invariants
   this reasoning rests on — pin both during implementation: `copy()` moves ciphertext bytes verbatim (it is a
   data copy, never a re-encrypt path; the destination-decryptability assertion guards this if it ever changes),
   and the same-master-key check covers the **db-migrate invocation path** specifically, not just controller boot
   (verify; if absent, the advisory is the only guard and encrypted rows silently corrupt under a different key).
3. No `start_paused`, no tokio time APIs (DB-backed tests — snapshot rule).

## Data-loss advisory (task, not code)

Anyone who ran `db-migrate` on a release containing any of the 13 tables lost that data at the destination (the
source DB is untouched — recovery is re-running `db-migrate` from the original source with the fixed version, so
this is recoverable, unlike the cascade-wipe finding). Add a release-notes/CHANGELOG advisory: affected users
should re-run `db-migrate` from their original source database after upgrading, or restore 2FA/OAuth rows from it.
The advisory should also state the standing precondition that source and destination share the same master key
(boot-enforced already; restated because encrypted rows copied under a different key are unrecoverable).

## Documentation deliverables

- Doc comment on `core_tables()` stating it is the single authority and how to add a table (one line + FK-safe
  position + the guard test will fail until done).
- `migration_coverage_complete` doc comment updated (no longer "integration — ignored"; it is the standing gate).
- `docs/development/database-migrations.md` and `docs/end-user/db-migration.md`: both carry pre-existing drift
  this PR must fix while touching the area — they claim "34 application tables" (actual list is already 55 before
  these additions; after the fix, describe coverage without a hardcoded count — point at `core_tables()` instead,
  per the no-hardcoded-counts docs rule) and cite `crates/core/controller/src/db_migrate/*.rs` paths that moved to
  `crates/core/controller-runtime/src/db_migrate/*.rs`. Plus the CHANGELOG advisory above.
- No new ADR: no architectural change — list consolidation and a test attribute.

## Out of scope / deferred

- Auto-deriving the table list from SeaORM entity registrations or migrations (the guard test already provides
  drift detection; codegen here is machinery without a second consumer).
- Any change to db-migrate's copy mechanics, batching, or CLI surface.
- The other migration-layer audit findings (covered by
  `docs/superpowers/specs/2026-07-11-migration-runner-hardening-design.md`).
- `AGENT_ONLY_TABLES` exclusion policy (unchanged).
