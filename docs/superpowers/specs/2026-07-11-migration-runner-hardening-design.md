# Migration Runner Hardening — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — CRITICAL "set_foreign_keys is a silent no-op inside run_migrations'
single transaction — table-recreation migrations cascade-delete child rows" + HIGH "CombinedMigrator passes plugin
migrations through a thread-local across await points on a multi-thread runtime" (same code area, one spec).

## Problem

Two independent defects in `crates/shared/db/src/migration/`:

### 1. FK suspension never happens (CRITICAL, data loss)

`run_migrations` / `run_migrations_with_plugins` (`migration/mod.rs`) wrap all migrations in one `db.begin()`
transaction — deliberately, so every DDL statement runs on the same physical pooled connection (SQLite DDL on one
pool connection is not reliably visible to the next; see the doc comment on `run_migrations`). But SQLite documents
`PRAGMA foreign_keys` as **a no-op inside a transaction**, and sqlx enables `foreign_keys=ON` on every connection by
default. So every table-recreation migration that calls `helpers::set_foreign_keys(manager, false)` actually runs
with FK enforcement ON. Concrete consequence, verified by the audit with a live sqlite3 repro: `m20260414_000001`
drops and recreates `update_history`; `update_output_lines` has `FK → update_history ON DELETE CASCADE`; the `DROP
TABLE` performs an implicit `DELETE` that cascade-wipes all `update_output_lines` rows — silently, inside a
committed transaction — for any populated SQLite install upgrading past April 2026. Every future recreation of a
parent table carries the same hazard (RESTRICT children instead abort the migration run).

### 2. Plugin migrations ride a thread-local across awaits (HIGH, silent omission)

`run_migrations_with_plugins` stores plugin migrations in a `std::thread_local` (`PLUGIN_MIGRATIONS`), then awaits
`CombinedMigrator::up()`. sea-orm-migration awaits real DB I/O (`install()`, pending-migration queries) before
`migrations()` finally reads the thread-local. On the controller's multi-thread tokio runtime, the task can resume
on a different worker thread whose thread-local is empty — plugin migrations (`ssh_hosts`, `proxmox_*`) are then
silently omitted. Fresh installs intermittently boot without plugin tables; behavior differs between restarts.
`#[tokio::test]`'s current-thread runtime can never catch it. The `drain(..)` inside `migrations()` additionally
makes the migrator single-shot per `set()`.

## Approach

### A. Runner-owned FK lifecycle on a dedicated migration connection (SQLite file DBs)

The insight: `PRAGMA foreign_keys` works fine when it is a **connection property established outside any
transaction**. So instead of letting individual migrations toggle it mid-transaction (impossible), the runner owns
the FK lifecycle:

1. In `run_migrations` / `run_migrations_with_plugins`, when the backend is SQLite **and** the database is
   file-backed: derive a dedicated single-connection migration pool from the caller's pool —
   `db.get_sqlite_connection_pool().connect_options()` (verified available in sea-orm 2.0.0-rc.41; returns
   `Arc<SqliteConnectOptions>`, so clone via `(*pool.connect_options()).clone()`), set `.foreign_keys(false)`,
   build `SqlitePoolOptions::new().max_connections(1)`, wrap via `SqlxSqliteConnector::from_sqlx_sqlite_pool`
   (same construction as `controller-runtime::db::connect_sqlite`; WAL/busy-timeout/synchronous settings carry
   over by construction since the options are cloned). The migration connection is **born with FK OFF** — no
   pragma sequencing at all. Gate the whole branch on `db.get_database_backend() == DbBackend::Sqlite` **before**
   touching `get_sqlite_connection_pool()` — that accessor panics on non-SQLite backends, and the no-panic
   invariant requires the explicit check.
2. Run the existing single wrapping transaction on that connection (preserves both the single-physical-connection
   DDL-visibility guarantee and all-or-nothing atomicity — the two properties the current design exists for).
3. After commit, on the same connection: run `PRAGMA foreign_key_check`; any rows returned → `DbErr` naming the
   offending tables (loud failure instead of silent corruption). Then drop the migration pool; the caller's normal
   pool (FK ON) is untouched. Run the same post-migration `foreign_key_check` on the **in-memory** path too — it
   costs one statement and catches orphan-class mistakes there as well. (Caveat, stated for honesty: no
   `foreign_key_check` can detect a cascade wipe — cascades leave the DB FK-consistent. The in-memory cascade
   exposure is bounded by the invariant in step 4 and by reviewer diligence; it is not machine-enforceable.)
4. **In-memory SQLite**: keep the current path unchanged — a second pool would open a *different* empty database,
   so the FK-OFF connection technique is structurally unavailable for `:memory:`. The safety argument, stated
   precisely (not "fresh DBs are empty" — earlier migrations do seed rows: permissions, roles, scheduled_tasks):
   **no currently-recreated parent table has cascade-child rows populated by earlier migrations**, and the two
   historical migrations where that came close (`m20260306_000002`, `m20260309_000003`) already drop
   `update_output_lines` manually first. Residual divergence, documented deliberately: in-memory (test) runs keep
   FK ON during migrations — violations fail loudly at statement time, which file-backed FK-OFF runs would only
   catch at the post-commit `foreign_key_check`; conversely a future cascade-wipe would silently fire only
   in-memory, where the affected DB is a fresh test database and the file-backed production path is immune. The
   cascade-wipe regression test (Tests #1) runs on a **file** DB, pinning the production path.
   **Detection must NOT use `SqliteConnectOptions::get_filename()`**: sqlx 0.9 rewrites
   `sqlite::memory:` URLs to an internal `file:sqlx-in-memory-{n}` filename and keeps the `in_memory` flag in
   `pub(crate)` fields with no public getter — filename inspection silently misroutes every in-memory caller
   (all test call sites) onto the file-backed path. Instead, probe the caller's pool once:
   `PRAGMA database_list` — the `file` column for the `main` database is empty for in-memory/temporary databases
   and an absolute path for file-backed ones (documented SQLite behavior, independent of sqlx internals). Empty →
   current path; non-empty → dedicated-connection path.
5. **PostgreSQL**: unchanged (`begin`/`up`/`commit`; the FK machinery is SQLite-only, as today).

Operational notes on the two-pool window:

- The cloned `SqliteConnectOptions` is a plain-data `Clone` — `busy_timeout`, `journal_mode`, and `synchronous`
  are struct fields and carry over verbatim; only `.foreign_keys(false)` is overridden.
- The caller's pool and the migration pool coexist only for the duration of the migration run, which completes in
  boot Phase 3 before any request-serving or background code touches the DB (existing boot contract —
  `init_database` runs migrations before returning the connection to later phases). The caller pool's
  `min_connections(1)` idle connection holds no WAL write lock and has executed no statements yet, so there is no
  meaningful lock contention and no pre-migration statement cache to go stale. Committed DDL is visible to other
  connections via SQLite's schema-cookie reparse. Test #1 asserts post-migration queries **through the original
  caller pool** to pin this.

**Rejected alternative (single pool, borrowed connection):** issue `PRAGMA foreign_keys=OFF` on a connection
checked out of the caller's own pool, before `BEGIN`. Rejected on mechanics: SeaORM's migrator requires a
`DatabaseConnection`/transaction, and a borrowed sqlx `PoolConnection` cannot be wrapped into one via any public
API; toggling the pragma on a pooled connection and returning it to the pool would also leak FK-OFF state into
later application queries. Re-sequencing boot to create the main pool only after a migration-only pool finishes
(no overlap at all) would work but is a cross-crate boot reorder touching every caller for a window that the
notes above show is already inert.

Then **delete `helpers::set_foreign_keys` entirely** and strip its call sites in the 8 table-recreation migration
files that import it, **plus** the byte-for-byte local duplicate `async fn set_foreign_keys` defined and called
inside `m20260318_000001_host_software_item_qualifier.rs` (a 9th affected file invisible to a
`helpers::set_foreign_keys` grep — delete the local fn and its calls too). The calls are literal no-ops today
(that is the bug), so removing them changes nothing for already-applied or fresh migrations — and deleting the
helper kills the footgun class on the file-backed path: nobody can call the trap again, and no future
table-recreation migration can forget FK handling because the runner owns the lifecycle there unconditionally
(the in-memory divergence is documented in step 4 above). No external users of the helper exist (verified by workspace grep).
This supersedes the audit's suggested read-back-verification: a helper that loudly fails inside the standard
runner would just turn every recreation migration into a hard error; removing the per-migration responsibility is
the fix.

The crash-recovery helpers (`check_crash_recovery` states B/C) remain: still dead code under the single-transaction
wrapper, but harmless, and they become live again if the transaction strategy ever changes. Not this spec's problem.

### B. Thread-local → instance-based migrator (`MigratorTraitSelf`)

sea-orm-migration 2.0.0-rc.41 ships a non-static migrator trait the current code (and the audit) missed:
`MigratorTraitSelf` (`src/migrator/with_self.rs`, re-exported from the crate) with `fn migrations(&self) ->
Vec<Box<dyn MigrationTrait>>` and `async fn up(&self, db, steps)`. This removes the need for **any** cross-task
state — thread-local or global:

- `CombinedMigrator` becomes a plain struct holding a plugin-migration provider:
  `struct CombinedMigrator { plugin_provider: Box<dyn Fn() -> Vec<Box<dyn MigrationTrait>> + Send + Sync> }`,
  implementing `MigratorTraitSelf::migrations(&self)` as core migrations + `(self.plugin_provider)()` with the
  existing name-dedup. The provider (not a `Vec`) is required because `Box<dyn MigrationTrait>` is not `Clone` and
  sea-orm-migration may call `migrations()` more than once per run — each call regenerates the full list.
- `run_migrations_with_plugins` changes its parameter from `Vec<Box<dyn MigrationTrait>>` to the provider closure
  (callers already build the Vec from the static `all_descriptors()` registry — wrapping that construction in a
  closure is a one-line change per caller), constructs a local `CombinedMigrator`, and calls
  `migrator.up(&txn, None).await`.
- No thread affinity (nothing is thread-local), no single-shot `drain()`, and concurrent runs — even with
  different providers — are trivially safe because each run owns its migrator instance. The thread_local, `set()`,
  and `clear()` all disappear.

**Rejected alternatives:** (1) process-global
`parking_lot::Mutex<Option<Arc<dyn Fn() -> Vec<Box<dyn MigrationTrait>> + Send + Sync>>>` provider slot — works,
but keeps global mutable state, needs a documented no-concurrent-different-providers caveat, and sits in tension
with the snapshot's no-static-init-for-runtime-changeable-state rule; unnecessary once `MigratorTraitSelf` exists.
(2) Global `Mutex<Vec<…>>` + `mem::take` — additionally keeps the single-shot drain and races parallel
`functional-tests` runs.

Call sites to update (all four; verified complete): `controller-runtime/src/migration/mod.rs` (the boot wrapper —
the only production caller), `web-api-queries/src/queries/reset_data.rs` (`#[cfg(test)]` module),
`surface-proxy/src/proxy/tests/controller_owned/proxmox_update_protection.rs` (test), and
`functional-tests/tests/support/db.rs` (test support).

## Tests

1. **Cascade-wipe regression (the CRITICAL's repro):** on a file-backed SQLite DB (tempdir) opened through a
   normal multi-connection pool, run migrations up to just before `m20260414_000001` (`Migrator::up(db, Some(n))`
   — compute `n` from the migration name list at test time, no hardcoded index), insert an `update_history` row
   plus `update_output_lines` child rows, run the remaining migrations, then assert the child rows survive **by
   querying through the original caller pool** (also pins cross-pool schema visibility). This test fails on
   today's code and passes with the fix.
2. **`foreign_key_check` loud failure:** unit test the post-commit check helper — craft a deliberate FK violation
   on a scratch connection, assert it returns `DbErr` naming the table (not silence).
3. **Thread-affinity regression:** `#[tokio::test(flavor = "multi_thread")]` running
   `run_migrations_with_plugins` with a marker plugin migration (creates a sentinel table), assert the sentinel
   table exists afterward. Run it a few iterations in-test to make thread-hop loss probable on old code.
4. **Concurrent runs:** two tasks running `run_migrations_with_plugins` (same provider) against two separate
   in-memory DBs concurrently; both DBs end up with the sentinel plugin table.
5. Existing migration/e2e tests keep passing unchanged (in-memory callers hit the unchanged path).

Note (snapshot rule): these tests exercise SQLx/SeaORM against real DBs — no `start_paused`, no tokio time APIs.

## Released-version data-loss audit (task, not code)

Determine whether any released version shipped `m20260414_000001` under the single-transaction wrapper (check tag
history for when the `run_migrations` txn wrapper landed relative to the migration). If yes: add a release-notes /
CHANGELOG advisory for SQLite upgraders — `update_output_lines` history for pre-upgrade updates was silently
cleared and is not recoverable; no code remediation is possible. The advisory is a deliverable of the
implementation PR.

## Documentation deliverables

- `docs/development/database-migrations.md` — rewrite the table-recreation pattern section: remove
  `set_foreign_keys` from the usage pattern; document the runner-owned FK lifecycle and the `foreign_key_check`
  gate; note the in-memory vs file-backed split. **Add `PRAGMA foreign_key_check` (and the `database_list` probe)
  to the raw-SQL exception table** — both are new raw-SQL constructs with no sea_query equivalent; each call site
  needs the standard inline comment naming the limitation (AGENTS.md "No raw SQL" invariant).
- `crates/shared/db/src/migration/helpers.rs` module doc — same pattern update (the doc's usage example currently
  shows the deleted helper).
- `run_migrations` / `run_migrations_with_plugins` doc comments — describe the dedicated-connection strategy and
  the provider contract (regenerated per `migrations()` call); include `# Errors` sections per the rust-idioms
  rule for the changed public functions.
- CHANGELOG/release-notes advisory per the audit task above (if triggered).
- No new ADR: internal mechanics of the migration runner; no externally observable architecture change.

## Out of scope / deferred

- The `db-migrate` 13-missing-tables CRITICAL (separate mechanism in `migrate_core_tables.rs` — next spec).
- Making crash-recovery states B/C reachable (would require per-migration transactions; the single-transaction
  atomicity is deliberately preserved).
- `run_migrations_debug` (debug-only helper, unchanged).
- Any change to migration content or ordering.
- Postgres-side FK handling (unaffected by the bug).
