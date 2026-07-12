# Proxmox Scaling-Data Migration Postgres Portability — Design

**Date:** 2026-07-12 **Status:** Draft **Source:** `.superpowers/audit-2026-07-11.md` — HIGH "Scaling data migration
uses SQLite-only randomblob()/hex() — fails on Postgres".

## Problem

`MigrateProxmoxScalingFromProtectionTables` (migration name `m20260504_000003_migrate_scaling_from_protection_tables`,
`crates/plugins/infrastructure/proxmox/src/controller_migration.rs`) copies rows from the old protection tables into
the new scaling tables. Its two `INSERT … SELECT` statements — **C.1** (`proxmox_scaling_defaults`, ~line 1449) and
**C.2** (`proxmox_scaling_item_overrides`, ~line 1467) — synthesise each row `id` in raw SQL:

```sql
lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || … || lower(hex(randomblob(6)))
```

`randomblob()` and `hex()` are **SQLite-only** functions. The controller supports Postgres
(`crates/ui/web-api/Cargo.toml` `db-postgres` feature; `db-all` bundles it). Plugin migrations run via
`run_migrations_with_plugins()` inside a single transaction on whichever backend is active, so on **any Postgres
deployment** this migration fails when the statement is executed — **even with zero source rows**, because Postgres
rejects the unknown functions at execute time regardless of whether any row is produced. The failure rolls back the
whole migration batch and the controller **cannot complete startup migration**: the instance is down.

Two secondary defects in the same statements:

- The `randomblob` concatenation produces a **UUID-shaped but non-conformant** string (random hex in the `8-4-4-4-12`
  layout, no RFC-4122 version/variant bits). It happens to be unique, but it is not a valid versioned UUID, so any
  consumer that later parses `id` as a real UUID is on borrowed time.
- The file contains **zero** `get_database_backend()` dispatch, unlike core migrations in `crates/shared/db`, which
  branch on `manager.get_database_backend()` whenever a statement is dialect-specific (e.g.
  `m20260309_000002_simplify_autodiscovery_ignores.rs`).

C.3 / C.4 / C.5 (the `UPDATE … SET … = NULL` statements, ~line 1487+) are portable. The file's `CREATE TABLE`
statements (~line 1347, ~line 1396) are believed portable but must be confirmed (see § Audit task).

## Approach

**Replace the two `INSERT … SELECT`-with-`randomblob` statements with a read-then-insert done in Rust**, using the
`sea_query` `Query::select()` / `Query::insert()` builders. The exact in-repo template for this operation (read source
rows → generate a fresh `Uuid::now_v7()` id per row → insert) is
**`crates/shared/db/src/migration/m20260307_000001_split_version_check.rs`** (`db.query_all(&select)` →
`Uuid::try_get_by(row, "id")` → `Uuid::now_v7().into()` → `Query::insert()…values_panic([...])`);
`m20260308_000002_fix_permission_uuid_storage.rs` is a second precedent for the same select-transform-in-Rust-insert
shape. (Do **not** follow `m20260302_000003_host_packages_has_update.rs` / `m20260303_000001_global_settings.rs` here:
those use `Query::insert().select_from(select)` — a pure in-DB `INSERT … SELECT` that generates **no** ids in Rust, so
they cannot produce a per-row `now_v7()` id and would lead straight back to the dialect-dependent id-generation this
fix removes.) Concretely, for each of C.1 and C.2:

1. Build the source `SELECT` with `Query::select()` (same columns and
   `WHERE update_cores IS NOT NULL OR update_memory_mb IS NOT NULL`; C.2 keeps its
   `JOIN plugin_configs pc ON pc.id = pio.plugin_config_id` for `tenant_id`). Execute it via `txn.query_all(&select)`
   and read each column off the returned `QueryResult`. **Because C.2 is a JOIN, read columns by index
   (`try_get_by_index`, as `m20260308_000002` does) — not `try_get(prefix, name)` by name**: both joined tables expose
   `id` / `tenant_id` / `created_at`, so a name-keyed read can collide across `pio.*` and `pc.*`. Reading by ordinal
   position off the explicit select column order is unambiguous. (The two-arg `try_get(prefix, column)` form exists but
   is the wrong tool for a JOIN — index reads sidestep the aliasing entirely.) Test #1's `tenant_id` assertion is the
   guard that the right column was read. Read rows off `QueryResult` directly — **not** through entity models (see
   below).

   **Carried-over timestamps must round-trip as values, never as strings (contrarian-pass-2, portability gap one layer
   down).** C.1/C.2 copy `created_at` / `updated_at` from the protection rows **unchanged**. The protection entities
   type these as `time::OffsetDateTime`; the destination `TIMESTAMP` columns decode differently per backend (SQLite
   text affinity vs Postgres `timestamp`). Read each timestamp as **`time::OffsetDateTime`** via `try_get_by_index` and
   re-insert it via `.into()` (the `sea_query::Value` path), so the value goes `OffsetDateTime` → `Value` →
   `OffsetDateTime` and **never** through a backend-formatted `String` (reading as `String` and re-inserting the text
   can silently reformat on SQLite and be rejected by Postgres `timestamp` input — the exact dialect footgun this spec
   closes, reintroduced on the timestamp column). Note: the cited `m20260307_000001` precedent inserts a **fresh**
   `OffsetDateTime::now_utc()` and does **not** read-then-write an existing timestamp — it is the exact template for the
   `id` generation and literal columns only; the timestamp passthrough is this migration's own step, so pin the type
   explicitly here.

2. For each source row, generate the destination `id` with **`uuid::Uuid::now_v7()`** in Rust (valid RFC-4122 v7,
   monotonic-ish, replaces the invalid `randomblob` concatenation). The `id` columns are declared `TEXT` (CREATE TABLE
   ~1348, ~1397), so insert the **string** form (`Uuid::now_v7().to_string()`), matching the existing text-id storage
   — not raw UUID bytes.
3. **Loop over the returned rows and insert one at a time** (do **not** accumulate into one batched insert). Build one
   `Query::insert()` per row with all destination columns and the mapped values (literals like
   `scaling_mode = 'absolute'` and the `NULL` delta columns carry over unchanged), and execute it against the same
   transaction — exactly the per-row-in-a-loop shape of the `m20260307_000001` precedent. (`values_panic` is the
   sea_query builder used throughout `crates/shared/db/src/migration/`; it panics only on a compile-time-fixed
   column/value count mismatch, a programmer error the first test run catches — it is not a runtime no-panic-rule
   concern.)

   **Zero source rows must insert nothing — do NOT build an empty batched insert (contrarian-critical, verified against
   sea-query 1.0.1 source).** A single `Query::insert()` on which `values_panic` is never called leaves `source =
None`, and the builder emits `INSERT INTO t ("id", …)` with **no `VALUES` / `SELECT` clause** — a syntax error on
   **both** SQLite and Postgres. The zero-rows case (the exact case that fails today on Postgres, per Test #2) is the
   common one for a fresh install with empty protection tables, so an empty batched insert would trade one backend bug
   for another. The per-row loop is inherently correct here: the loop body never runs when there are no rows, so no
   insert statement is built at all. There is no `insert_many` in sea_query that guards this for you; the loop is the
   guard.

**No `unwrap`/`expect` in the migration body.** Every `try_get`/`try_get_by` read returns a `Result`; map failures to
`DbErr` (`.map_err(|e| DbErr::Custom(...))` or `DbErr::Migration`), exactly as `m20260308_000002` does — never
`.unwrap()`. Tests may `.unwrap()` (allowed in test code); the production `up()` must not.

Everything stays **inside the existing explicit transaction** (`manager.get_connection().begin()` at ~line 1446). That
transaction already exists for a documented reason (its comment: without it, C.2 failing after C.1 leaves the
migration permanently broken because a retry re-hits the `UNIQUE` constraint). The rewrite preserves that: read +
insert for both C.1 and C.2 plus the C.3–C.5 updates all commit or roll back together.

**No SeaORM entity ActiveModels in the migration.** Migration code must not depend on entity structs: entities
describe the **current** schema and drift from the historical point the migration froze at, so a later column addition
would silently change or break this migration. The project's "No raw SQL — use `sea_query` builders" rule explicitly
permits `sea_query` in migrations; that is the correct tool here. `sea_query`'s `Query::select`/`Query::insert`
reference tables and columns by name (via `Iden` / `Alias`), matching the file's existing style, with no entity
coupling.

**`BEGIN IMMEDIATE` does not apply.** The project's SQLite read-then-write rule targets the _live application path_,
where two pooled connections race between a read and a write. A migration runs on a single dedicated connection inside
its own wrapping transaction during startup, before the app serves traffic — there is no second writer to lose a
snapshot to. Standard migration transaction semantics are correct; no `begin_with_options`/`SqliteTransactionMode` is
needed. (Documented inline so the question isn't re-litigated.)

### Why not the dialect-branch alternative

The smaller alternative — keep the set-based `INSERT … SELECT` and branch on `manager.get_database_backend()`,
swapping `randomblob(...)` for `gen_random_uuid()::text` on Postgres — is **rejected** as the primary approach. It
duplicates dialect logic in-file (the thing the rest of the file's absence of `get_database_backend` shows the crate
wants to avoid), and it leaves the invalid-UUID `id`s in place on the SQLite branch. The Rust-side `now_v7()` path is
one code path for both backends, produces valid UUIDs, and matches existing migration precedent. Row counts at
migration time are tiny, so the "set-based is faster" argument for the branch does not carry weight here. The branch
would only win if these tables held millions of rows at migration time, which they do not (they are freshly-introduced
scaling tables seeded from small protection tables).

### Immutability / already-released reasoning

`m20260504_000003` may already be released. **Editing its body in place is safe:**

- SeaORM tracks applied migrations by **name** in `seaql_migrations` (no content checksum). An already-applied SQLite
  instance has the row `m20260504_000003_…` recorded and will **not** re-run the migration — the body edit is inert
  for it.
- A Postgres instance **never succeeded**: the statement fails at execute time, the batch rolls back,
  `seaql_migrations` has no row at or past this migration, and the controller wouldn't start. Fixing the body lets
  such an instance run the corrected migration from a clean state on next startup.
- No data-loss risk on either backend: today the migration either already completed (SQLite) or never ran (Postgres).

This reasoning belongs in the doc comment so a future maintainer doesn't "freeze" the migration out of a
general-but-here-inapplicable "never edit shipped migrations" reflex.

## Audit task (in-scope, part of this change)

Scan **every** `execute_unprepared` / raw statement remaining in `controller_migration.rs` for SQLite-only syntax and
record the result in the implementation:

- C.3 / C.4 / C.5 `UPDATE … SET col = NULL` — expected portable; confirm.
- `CREATE TABLE` at ~1347 and ~1396 — these use raw `execute_unprepared` hand-written SQL (`VARCHAR(16)`, `INTEGER`,
  `TIMESTAMP`, `CHECK(...)`, `TEXT`, `UNIQUE`), a pre-existing deviation from the "use sea_query `Table::create()`
  builders" rule. They are **portable** (no SQLite-only type affinities, no `AUTOINCREMENT`, no
  `strftime`/`datetime('now')` defaults), so leave them as-is — they pass the portability bar this change is about.
  Note in the PR that they remain raw-SQL-but-portable; rewriting them to builders is out of scope (YAGNI).
- Any other `randomblob`/`hex`/`strftime`/`json_*`/`printf` occurrence — grep the file and list findings.

If any additional non-portable statement is found, fold its fix into this change (same theme, same file); if none,
state "remaining statements verified portable" in the PR description. This audit is bounded to this one file — it is
not a codebase-wide migration sweep (YAGNI).

## Tests

Unit tests in the proxmox plugin's migration test module (SQLite in-memory is the crate's existing harness):

1. **Regression — data copied with valid UUIDs.** Bring the schema up to just before `m20260504_000003`, seed
   `proxmox_protection_defaults` and `proxmox_protection_item_overrides` (+ the `plugin_configs` parent rows the C.2
   JOIN and FK require), run the migration, and assert: (a) `proxmox_scaling_defaults` /
   `proxmox_scaling_item_overrides` contain the expected rows with the right column values and correct `tenant_id`;
   (b) each generated `id` parses as a valid `Uuid` (`id.parse::<Uuid>()` succeeds and version is v7) — this is the
   assertion the old code would fail; (c) the copied `created_at` / `updated_at` **equal the source timestamps**
   (explicit equality, not covered by "right column values" — timestamps are the column most likely to drift through a
   bad string round-trip, so pin them).
2. **Zero-source-rows.** Run the migration with empty protection tables; assert it **succeeds** and inserts nothing.
   This is the exact case that fails **today on Postgres** at execute time — the test pins that the corrected
   statement no longer references backend-only functions.
3. **Dialect-safety guard.** If a Postgres test harness is available in CI for this crate, run the migration against
   Postgres and assert success. If not available, assert (via a string check on the built `sea_query` SQL, or a source
   grep-guard test scoped to the migration's `up()` body — not the whole file, which legitimately contains
   `hex`/`Uuid` tokens in test helpers) that the migration emits **no `randomblob` / `hex(` tokens**, so the footgun
   cannot silently regress. Prefer the real Postgres run if the harness exists; otherwise the token guard is the
   floor.

Tests use no `tokio::time` APIs, so **no `start_paused`** (snapshot rule: only time-API tests get the paused clock).

## Documentation deliverables

- **Doc comment on `MigrateProxmoxScalingFromProtectionTables`**: state that the migration must stay dialect-portable
  (runs on SQLite and Postgres), why the ids are generated in Rust with `Uuid::now_v7()` rather than in SQL, and the
  `seaql_migrations` name-tracking immutability reasoning (safe to edit in place).
- **`docs/development/database-migrations.md`**: add a short note under the migration-authoring guidance that **plugin
  migrations must be dialect-portable** — no backend-only SQL functions (`randomblob`, `hex`, `gen_random_uuid`,
  `strftime`, …); generate values in Rust or branch on `get_database_backend()`. This is the canonical home for the
  rule; link it rather than repeating it elsewhere.
- **No API / wire / OpenAPI / CONTEXT.md change** — internal migration mechanics, no externally observable surface
  change (beyond "Postgres controllers can now start"). **No ADR** — bug fix using an existing in-codebase idiom, not
  an architectural decision.
- **No dependency change**: `uuid` is already a `[dependencies]` entry of the proxmox plugin with the `v7` feature
  enabled (`crates/plugins/infrastructure/proxmox/Cargo.toml`), so `Uuid::now_v7()` is available in production
  migration code with no manifest edit. (Verified — called out here so review doesn't re-flag it.)

## Out of scope / deferred

- Codebase-wide audit of raw SQL in **other** migration files (core `crates/shared/db` migrations already dispatch on
  backend; other plugin crates' migrations are a separate finding if one exists). This change is bounded to
  `controller_migration.rs`.
- Backfilling or repairing the invalid `randomblob`-shaped ids already written on **SQLite** instances that ran the
  old migration. There **is** a consumer that parses them: the entities `proxmox_scaling_default` /
  `proxmox_scaling_item_override` declare `pub id: Uuid`, so SeaORM (with-uuid) parses the text `id` on **every** read.
  The randomblob ids survive this today because they land in the `8-4-4-4-12` hex layout `Uuid::parse_str` accepts —
  `parse_str` validates layout only, not the RFC-4122 version/variant bits — so they read back as `Uuid` values with
  version `None`. They are therefore functional, and rewriting historical rows is not warranted now (YAGNI). The one
  real caveat: if a future change adds a **strict-v7** check (e.g. asserting `get_version() == Some(Version::SortRand)`
  or relying on v7 time-ordering), it must tolerate or backfill these legacy ids first. Note that in the migration doc
  comment. This is not the "no consumer parses them" hand-wave — the consumer exists; it just happens to be lenient.
- The transient-node-failure backup-target cache-prune HIGH in the same crate (`discovery.rs` / `policy_store.rs`) —
  different mechanism and file, its own spec.
