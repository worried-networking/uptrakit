# Proxmox Scaling-Data Migration Postgres Portability — Design

**Date:** 2026-07-12 (revised 2026-07-14) **Status:** Draft **Source:** `.superpowers/audit-2026-07-11.md` — HIGH "Scaling
data migration uses SQLite-only randomblob()/hex() — fails on Postgres".

> **Revision 2026-07-14.** The 2026-07-11 revision of this spec was refuted during plan review
> (`docs/superpowers/plans/2026-07-12-proxmox-migration-postgres-portability.md`, BLOCKED banner): its
> `Uuid::now_v7().to_string()`-into-TEXT approach produces rows that **fail to read back through SeaORM on both
> backends**, and its "legacy randomblob ids are functional because `Uuid::parse_str` is lenient" claim is false —
> the decode path is sqlx, not `parse_str`, and it is blob-only on SQLite. This revision reframes the root cause
> (the scaling tables' hand-written `TEXT` uuid columns, not `randomblob` alone), widens the fix to the two
> `CREATE TABLE` migrations, switches all uuid binds to `Value::Uuid`, and adds a repair migration for
> already-migrated SQLite instances. Every load-bearing claim below was re-verified against vendored dependency
> source; see § Verified evidence.

## Problem

Three defects, one root cause: the two `proxmox_scaling_*` tables and the data migration that seeds them were written
as SQLite-only hand-rolled SQL, while everything that reads and writes them at runtime goes through SeaORM entities
typed `Uuid`.

**Defect 1 — Migration C is unparseable on Postgres (the original audit HIGH).**
`MigrateProxmoxScalingFromProtectionTables` (name `m20260504_000003_migrate_scaling_from_protection_tables`,
`crates/plugins/infrastructure/proxmox/src/controller_migration.rs` ~line 1441) copies rows from the old protection
tables into the new scaling tables. Its two `INSERT … SELECT` statements — **C.1** (`proxmox_scaling_defaults`, ~line 1449)
and **C.2** (`proxmox_scaling_item_overrides`, ~line 1467) — synthesise each row `id` in raw SQL:

```sql
lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || … || lower(hex(randomblob(6)))
```

`randomblob()` and `hex()` are SQLite-only. On **any Postgres deployment** the statement fails at execute time — even
with zero source rows — the whole migration batch rolls back (see § Verified evidence, single-transaction runner), and
the controller cannot complete startup: the instance is down.

**Defect 2 — Migrations A and B declare uuid columns as raw `TEXT`, breaking all scaling writes on Postgres.**
`CreateProxmoxScalingDefaults` (`m20260504_000001`, ~line 1347) and `CreateProxmoxScalingItemOverrides`
(`m20260504_000002`, ~line 1396) are hand-written `CREATE TABLE` strings declaring `id TEXT NOT NULL PRIMARY KEY`,
`tenant_id TEXT NOT NULL`, `plugin_config_id TEXT NOT NULL` (B adds `software_item_id TEXT NOT NULL`). **Every sibling
proxmox table** (`proxmox_host_mappings`, both protection tables, `proxmox_backup_target_cache`,
`proxmox_resource_scaling_records`) declares these columns via the sea_query `.uuid()` builder; the scaling pair is the
sole deviation. The runtime store (`scaling_store.rs`) writes through entity ActiveModels — `id: Set(Uuid::now_v7())`
etc. — which bind a typed uuid parameter. Postgres rejects a `uuid`-typed bind against a `text` column, so on Postgres
**all** runtime scaling upserts are broken even before Defect 1 is reached. The original revision of this spec
mis-scoped these `CREATE TABLE`s as "portable": they parse everywhere, but they are type-incompatible with the entity
layer on Postgres. Syntax-portability was the wrong bar; bind-compatibility is the real one.

**Defect 3 — Migration C's text ids are unreadable through SeaORM on SQLite.** The entities
(`entity/proxmox_scaling_default.rs`, `entity/proxmox_scaling_item_override.rs`) declare `pub id: Uuid` (and
`tenant_id`, `plugin_config_id`, `software_item_id`). On SQLite, sqlx 0.9 decodes `Uuid` **from blob bytes only**
(`Uuid::from_slice(value.blob_borrowed())` — no text fallback), while it encodes `Uuid` as a 16-byte blob. Runtime-
written rows therefore store uuid cells as blobs and read back fine; the rows Migration C wrote store them as 36-char
text, and every entity read of such a row fails with `ParseByteLength { len: 36 }`. **Already-migrated SQLite instances
have unreadable scaling rows today** — the migrated scaling config is effectively lost to the application until
repaired (any `Entity::find()` touching a text row errors wholesale). The original spec's plan to insert
`Uuid::now_v7().to_string()` would have reproduced exactly this corruption for every future migration run.

The defects escaped because the existing Migration C tests assert via raw `SELECT COUNT(*)` — they never exercise the
entity decode path that production uses.

## Verified evidence

Re-verified 2026-07-14 against the workspace lockfile (sea-orm 2.0.0-rc.41 → sea-query 1.0.1, sqlx 0.9.0):

- **sqlx SQLite `Uuid` codec is blob-only.** `sqlx-sqlite-0.9.0/src/types/uuid.rs`: `Encode` pushes
  `SqliteArgumentValue::Blob(self.as_bytes())`; `Decode` is `Uuid::from_slice(value.blob_borrowed())`. The
  `compatible(Blob | Text)` impl only relaxes type _checking_; decode still requires 16 raw bytes, so 36-char text
  fails with a byte-length error. This is the probe result recorded in the plan's BLOCKED banner, confirmed from
  vendored source.
- **sea_query `.uuid()` renders `uuid_text` on SQLite and `uuid` on Postgres** (`sea-query-1.0.1/src/backend/{sqlite,postgres}/table.rs`).
  On SQLite the declared type is affinity-only and does not coerce sqlx's blob writes; on Postgres it is a real `uuid`
  column that accepts the entity layer's typed binds. This is what every sibling proxmox table already gets.
- **The migration runner wraps the entire batch in one transaction.** `run_migrations_with_plugins()`
  (`crates/shared/db/src/migration/mod.rs:244`) does `db.begin()` → `CombinedMigrator::up(&txn)` → `commit()`. A
  Postgres instance that ever hit Migration C therefore has **no** `seaql_migrations` rows for A, B, C, or D — the
  whole batch rolled back together.
- **Migration C's inner `begin()` nests as a `SAVEPOINT`, on the same connection — the batch-rollback claim survives
  it (contrarian pass 1).** C's `up()` calls `manager.get_connection().begin()` while the batch transaction is open.
  In sea-orm 2.0.0-rc.41 a nested `begin()` on a `DatabaseTransaction` explicitly "starts a nested transaction via
  `SAVEPOINT`" (`sea-orm-2.0.0-rc.41/src/database/transaction.rs:32`), sharing the parent's `Arc`-held connection —
  not a second `BEGIN` on a pooled connection. A failure inside C releases to the savepoint and propagates the
  `DbErr`, so the outer batch transaction (including all `seaql_migrations` bookkeeping rows) still rolls back as one
  unit on both backends. The inner `begin()` also stays meaningful standalone: the crate's migration tests invoke
  `up()` directly on a plain connection, where it is a real transaction and the documented UNIQUE-retry rationale
  applies. Keep it; the rewrite's doc comment should state the savepoint semantics so the safety argument is on
  record.
- **The runtime write path binds `Value::Uuid`.** `scaling_store.rs` upserts via ActiveModels with
  `Set(Uuid::now_v7())` / `Set(tenant_id)`; there is no `.to_string()` anywhere on that path.
- **`proxmox_resource_scaling_records` and all other sibling tables use `.uuid()` builders** — the audit for further
  TEXT-declared uuid columns in this file comes back clean; A and B are the only offenders.

## Approach

Three coordinated edits in `controller_migration.rs`, one new migration, no changes outside the proxmox plugin crate
(plus docs).

### 1. Rewrite Migrations A and B with sea_query builders (uuid columns)

Replace the two hand-written `CREATE TABLE` strings with `Table::create()` builders. Column-type idiom comes from the
same-file siblings (`CreateProxmoxProtectionPolicyTables` for `.uuid()`/`.timestamp()` column defs — note its overall
shape differs: composite PK plus FKs, which A/B do **not** copy). The surrogate-`id`-PK-plus-separate-UNIQUE-index and
`.check()` pieces have no same-file sibling; their in-repo precedents live in `crates/shared/db/src/migration/`
(`m20260316_000001_host_machine_id_partial_unique.rs` for `Index::create().unique()`, `m20260514_000001_audit_logs_v2.rs`
for `.check()`), so the implementer assembles from those rather than copy-adapting one sibling:

- `id`, `tenant_id`, `plugin_config_id` (and `software_item_id` in B): `.uuid()`, not null; `id` primary key.
- `scaling_mode`: `.string_len(16).not_null().default("none")` (preserves the shipped `VARCHAR(16) … DEFAULT 'none'`).
- The four scaling columns: `.integer().null().check(...)` preserving the shipped `CHECK (col IS NULL OR col >= 1)`
  constraints (`ColumnDef::check` with an `Expr`).
- `created_at` / `updated_at`: `.timestamp().not_null()` — the exact sibling idiom (`DiscoveredAt`/`UpdatedAt` on
  `proxmox_host_mappings`), which the entities' `OffsetDateTime` fields already work against on both backends.
- The shipped `UNIQUE (tenant_id, plugin_config_id)` / `UNIQUE (software_item_id, plugin_config_id)` constraints:
  preserve via an index/unique-key clause on the create statement.
- **Do not add foreign keys.** The shipped tables have none; adding them now would create schema drift against
  already-applied SQLite instances (which will never re-run A/B) and is scope creep. Note the omission in a comment.

Extend the existing `ProxmoxScalingDefaults` / `ProxmoxScalingItemOverrides` `DeriveIden` enums (currently
`Table`-only, ~line 1322) with the column variants the builders need. Migration **names stay unchanged**.

### 2. Rewrite Migration C as a Rust read-then-insert loop binding `Value::Uuid`

Replace the two `INSERT … SELECT` statements with a read-then-insert done in Rust using `Query::select()` /
`Query::insert()` builders. In-repo templates, each cited for what it actually demonstrates:
`crates/shared/db/src/migration/m20260307_000001_split_version_check.rs` for the operation shape (per-row read →
generate id → insert; note it reads by _name_, which C must not copy), `m20260308_000002_fix_permission_uuid_storage.rs`
for the transactional variant **and** the ordinal `try_get_by_index` reads (`String`/`Vec<u8>` there;
`m20260303_000003_audit_logs.rs:162` is the precedent for `Uuid::try_get_by_index` specifically), and `mod.rs:788`
for the `from_as`/`join_as` JOIN-builder shape only (its read is a single aggregate column, not a multi-column
ordinal read).
For each of C.1 and C.2:

1. Build the source `SELECT` with `Query::select()` (same columns and
   `WHERE update_cores IS NOT NULL OR update_memory_mb IS NOT NULL`; C.2 keeps its
   `JOIN plugin_configs pc ON pc.id = pio.plugin_config_id` for `tenant_id`). Execute via `txn.query_all(&select)`.
   **Read columns by ordinal index (`try_get_by_index`)**, never by name — both joined tables in C.2 expose
   `id`/`tenant_id`/`created_at`, and index reads sidestep the aliasing. Read uuid columns as `Uuid` (the protection
   tables are `.uuid()` columns written by the runtime, so the blob/uuid decode succeeds on both backends), the
   scaling values as `Option<i32>`, and the timestamps as `time::OffsetDateTime`. (Timestamp read-back precedent:
   `m20260318_000001_host_software_item_qualifier.rs` reads `OffsetDateTime` / `Option<OffsetDateTime>` out of rows
   and re-inserts them via `.into()` in its table-rebuild loop — the same read-then-reinsert operation, though it
   reads by _name_ where C reads by ordinal. Test #1's timestamp-equality assertion pins the round-trip regardless.)
2. Per source row, generate the destination id with `uuid::Uuid::now_v7()` and insert **all uuid values as
   `Value::Uuid` binds** — `Uuid::now_v7().into()`, `tenant_id.into()`, etc. **Never `.to_string()`** (Defect 3: text
   uuid cells are unreadable via sqlx on SQLite; conversely a text bind is rejected by the now-`uuid`-typed Postgres
   columns from step 1). Timestamps re-insert via `.into()` on the read `OffsetDateTime` — value round-trip, never a
   backend-formatted `String` (a text timestamp reformats on SQLite and is rejected by Postgres).
3. **Loop over the rows and insert one at a time** (`Query::insert()…values_panic([...])` per row, executed on the
   same `txn`). The per-row loop is the empty-batch guard: a `Query::insert()` on which `values_panic` is never called
   emits `INSERT INTO t (cols)` with no `VALUES` clause — a syntax error on **both** backends (verified against
   sea-query 1.0.1) — and zero source rows is the _common_ case (fresh install). The loop body simply never runs; no
   statement is built. Do not "optimize" into a single batched insert. This is a deliberate, documented exception to
   the "batch queries instead of per-item loops" rule: that rule targets live-path N+1 amplification, while this loop
   runs once at migration time over a bounded handful of rows and buys the empty-batch correctness above. State the
   exception in an inline comment so reviewers don't flag it as an oversight.

**No `unwrap`/`expect` in the migration body** — map every `try_get_by_index` failure to
`DbErr::Custom(format!(...))`, exactly as `m20260308_000002` does. Test bodies may unwrap.

**Everything stays inside the existing explicit transaction** (`manager.get_connection().begin()`, ~line 1446). Its
comment documents why (C.2 failing after C.1 would otherwise strand the migration on the `UNIQUE` constraint at
retry); the rewrite preserves read + insert for C.1/C.2 plus the C.3/C.4 `UPDATE … SET … = NULL` cleanups (portable —
untouched) in one commit-or-rollback unit.

**No SeaORM entity ActiveModels in the migration.** Entities describe the _current_ schema and drift from the point
the migration froze at. `sea_query` builders reference tables/columns by `Iden`/`Alias` with no entity coupling; the
project's "No raw SQL" rule explicitly blesses them in migrations.

**`BEGIN IMMEDIATE` does not apply.** The SQLite read-then-write rule targets the live application path where pooled
connections race. Migrations run on one dedicated connection during startup before traffic; there is no second writer.
Plain `begin()` is correct. (Documented inline so it isn't re-litigated.)

### 3. New repair migration for already-migrated SQLite instances

Already-applied SQLite instances will never re-run A/B/C (tracked by name), so their Migration-C-written rows keep
text uuid cells and stay unreadable. Add a **new** migration appended to `migrations()` (~line 1726) — e.g.
`m20260714_000001_proxmox_scaling_uuid_repair`.

**This exact repair class already has a working in-repo precedent:**
`crates/shared/db/src/migration/m20260308_000002_fix_permission_uuid_storage.rs` repairs TEXT-stored uuid ids on
SQLite for `permissions.id` and documents the identical `ParseByteLength { len: 36 }` failure. Copy its mechanism, do
not invent a new one:

- Branch on `get_database_backend()` and do the work **only on `DatabaseBackend::Sqlite`** (early `return Ok(())`
  otherwise, as the precedent does). This is the legitimate use of a dialect branch: the corruption itself is
  SQLite-only. On Postgres it is a structural no-op — no Postgres instance _that reached this state through
  Uptrakit's own migration runner_ can have rows needing repair (single-batch-transaction rollback, § Verified
  evidence). A manual cross-backend dump/restore is unsupported, out of scope, and would surface as a loud load-time
  decode/restore error, not silent corruption. The repair's inline comment should also restate the "startup, single
  dedicated connection, no `BEGIN IMMEDIATE`" note from Migration C so it isn't re-litigated.
- Detect text rows via **`query_all_raw` with a raw `Statement`** —
  `SELECT id, tenant_id, … FROM <table> WHERE typeof(id) = 'text'` — the approved raw-SQL exception for `typeof()`
  named in `docs/development/database-migrations.md` (SQLite-specific function, no sea_query equivalent; inline
  comment naming the limitation, exactly as the precedent does). Not `Expr::cust` inside a builder: the workspace has
  zero `Expr::cust` uses and the documented idiom for this pattern is the raw-`Statement` read.
- Read the uuid columns as `String` (`String::try_get_by_index`), parse each to `Uuid` in Rust (parse failure →
  `DbErr::Custom`, aborting the transaction). Abort — not skip — is deliberate and matches the precedent's policy:
  every text cell old Migration C could have produced is parseable (the `randomblob` concatenation is a valid
  `8-4-4-4-12` hex layout), so an unparseable cell is corruption from no known code path and must not be silently
  papered over. Because this runs on the startup path, the error message must be **actionable**: name the table, the
  offending stored value, and the remediation ("delete the row and restart"), so a crash-looping operator is not left
  with a cryptic `DbErr`.
- **Blob/text duplicate rows CAN exist — the repair must handle them (contrarian pass 1, verified against
  `scaling_store.rs`).** The naive claim "the runtime can never write a sibling of a text row" is false. The upsert
  reads with `filter(TenantId.eq(tenant_id))` — a `Value::Uuid` (blob) bind — which the text-stored row never matches
  byte-wise, so the read returns `None` without ever decoding the text row, and the upsert **inserts a fresh blob
  row**. The `UNIQUE (tenant_id, plugin_config_id)` constraint compares stored bytes, text ≠ blob, so the insert
  succeeds: one logical key, two physical rows. Blindly converting the text row would then collide with the blob
  sibling and abort the repair — a startup crash loop, worse than the corruption. Therefore, per text row: parse its
  unique-key columns, check (via a blob-bind `SELECT`) whether a blob row with the same logical unique key exists; if
  yes, **`DELETE` the text row** — the blob sibling is strictly newer (written by the runtime after the migration)
  and carries the user's later intent; if no, convert in place via one `Query::update()` setting **all** uuid-typed
  columns to `Value::Uuid` binds, keyed `WHERE id = <old text string bind>`
  (`Expr::col(id).eq(Value::String(...))`). **The duplicate probe must use exactly the table's declared `UNIQUE`
  tuple — the same one the conversion would collide on — and the two tables differ (contrarian pass 2):**
  `proxmox_scaling_defaults` is `UNIQUE (tenant_id, plugin_config_id)` (~line 1362) but
  `proxmox_scaling_item_overrides` is `UNIQUE (software_item_id, plugin_config_id)` (~line 1412) — `tenant_id` is
  **not** part of it. A uniform two-column probe would false-positive on defaults-shaped keys (deleting an
  item-override text row whose real key has no blob sibling — silent data loss) or miss the actual collision and
  crash on the constraint. Deleting the text row also discards its original `created_at` — and, on
  `item_overrides`, possibly a `tenant_id` differing from the blob sibling's (that column is not in the key) — in
  favour of the blob sibling's values. Intentional and immaterial: the `UNIQUE` constraint makes the blob row the
  sole authoritative row for that logical key, the scaling _values_ are the user's current intent, and nothing
  audits scaling-row creation time.
- Key-choice note: `m20260308_000002` keys its UPDATE on `name` because its corrupted column _was_ `permissions.id`
  and a clean second column existed; the scaling tables have no uncorrupted column, so the repair keys on the old
  text `id` directly. Safe: `id` is the primary key, the stored text is byte-exact what was selected, and SQLite
  evaluates the `WHERE` against the pre-image row before the `SET` overwrites it. Not `rowid`: no in-repo migration
  keys on `rowid`, and no need to start. Both tables inside one transaction. Bind note: the precedent binds
  `Value::Bytes` throughout, including its UPDATE; `Value::Uuid` in an UPDATE has no in-repo migration precedent but
  is mechanically equivalent on SQLite (sqlx encodes it as the same 16-byte blob) and is chosen here for consistency
  with Migration C — either bind is acceptable; do not mix them. Unlike the precedent, no FK delete-reinsert dance is
  needed: the scaling tables have no foreign keys and nothing references their ids.
- Repair-in-place is the only heal: re-running Migration C is foreclosed because its C.3/C.4 steps nulled out the
  protection-table source columns — the re-seed data no longer exists. (Noted so nobody proposes "just re-run C".)
- Fresh installs (both backends) and already-repaired databases hit zero matching rows: no-op.

`down()` for the repair migration is a no-op (the repair is not meaningfully reversible and reversing it would
re-corrupt).

### Why not the alternatives

- **Dialect-branch in Migration C** (keep `INSERT … SELECT`, swap `randomblob` for `gen_random_uuid()::text` on
  Postgres): rejected. It duplicates dialect logic, leaves invalid ids on the SQLite branch, and — decisive after this
  revision — still writes **text** uuid cells that the entity layer cannot read back on SQLite. It fixes the parse
  error while preserving the data corruption.
- **Keep the `TEXT` columns and retype the entities to `String`**: rejected. It would ripple through
  `scaling_store.rs`, `resource_scaling.rs`, and every call site; it abandons type safety on tenant/config ids; and it
  makes the scaling pair permanently deviant from every sibling table. Sibling parity (`.uuid()` everywhere) is the
  smaller and correct change.
- **Set-based `INSERT … SELECT` for speed**: row counts at migration time are tiny (freshly-introduced scaling tables
  seeded from small protection tables); the per-row loop's correctness properties (Rust-side v7 ids, typed binds,
  empty-batch guard) win.

### Immutability / already-released reasoning

`m20260504_000001/_000002/_000003` may already be released. **Editing all three bodies in place is safe:**

- SeaORM tracks applied migrations by **name** in `seaql_migrations` (no content checksum). An already-applied SQLite
  instance skips all three; the edits are inert for it. Its tables keep the `TEXT`-declared columns — harmless on
  SQLite, where the declaration is affinity-only and the runtime writes blobs regardless; the resulting declared-type
  drift between old and fresh SQLite databases is behaviorally invisible and is documented in the migration doc
  comment.
- A Postgres instance **never recorded any of them**: Migration C fails at execute time and the runner wraps the whole
  batch in one transaction, so A/B/C/D all rolled back together. The corrected bodies run fresh on next startup from a
  clean state.
- Data repair for the one real corruption case (SQLite instances that ran the old Migration C) is handled by the new
  repair migration, not by re-running edited migrations.

This reasoning belongs in the doc comment so a future maintainer doesn't "freeze" the migrations out of a
general-but-here-inapplicable "never edit shipped migrations" reflex.

## Audit task (in-scope, part of this change)

Scan every `execute_unprepared` / raw statement remaining in `controller_migration.rs` against **two** bars — syntax
portability _and_ bind/type compatibility with the entity layer (the bar Defect 2 proved necessary):

- C.3 / C.4 `UPDATE … SET col = NULL` — portable; confirm and leave.
- Any other `randomblob`/`hex(`/`strftime`/`json_*`/`printf` occurrence — grep the file and list findings.
- Any other raw `CREATE TABLE`/`ALTER` declaring a column as `TEXT` while its entity types it `Uuid` — verified clean
  as of this revision (all sibling tables use `.uuid()` builders); re-confirm at implementation time.

If an additional non-portable or bind-incompatible statement is found, fold its fix into this change (same theme, same
file); if none, state "remaining statements verified portable and bind-compatible" in the PR description. Bounded to
this one file (YAGNI).

## Tests

Unit tests in the proxmox plugin's migration test module (SQLite in-memory is the crate's existing harness). Two
harness rules motivated by how the defects escaped:

- **Seed uuid/timestamp values through `sea_query` with `Value::Uuid` / `OffsetDateTime` binds** (production-parity
  storage: runtime-written protection rows are blobs). Raw-SQL text seeds would make the migration's `Uuid` reads fail
  — and would mask exactly the class of bug this change fixes.
- **Assert through entity reads (`Entity::find()`), not raw `SELECT COUNT`.** The entity decode path is the production
  contract; the raw-COUNT assertions in the current tests are how unreadable rows shipped.

1. **Regression — data copied, readable, valid v7 ids.** Bring the schema up through A/B (builder version), seed
   `proxmox_protection_defaults` / `proxmox_protection_item_overrides` (+ `plugin_configs` parent rows for the C.2
   JOIN) via `sea_query` binds, run Migration C, then via entity reads assert: (a) expected rows with correct values
   and `tenant_id` (C.2's JOIN resolution — seed a `plugin_configs.tenant_id` distinct from every other id in the
   test); (b) each `id` is version 7 (`get_version_num() == 7`); (c) `created_at`/`updated_at` **equal** the seeded
   source timestamps (explicit equality — timestamps are the column most likely to drift through a bad round-trip).
2. **Zero-source-rows.** Empty protection tables; migration succeeds and inserts nothing. Pins the empty-batch guard
   and the fresh-install case that fails today on Postgres.
3. **Repair migration.** Seed the scaling tables directly with text-form uuid rows mimicking the old Migration C
   output (raw SQL is correct _here_ — it reproduces the corruption); run the repair; assert entity reads now succeed
   and each uuid value is **preserved** (same UUID, re-encoded — the repair must not mint new ids; assert UUID
   **equality**, never `get_version_num() == 7` — legacy `randomblob` ids carry arbitrary version bits and are not
   v7). Seed **at least two** text rows per table so a botched set-based rewrite (instead of the per-row loop) is
   caught. Second case:
   rows seeded via `Value::Uuid` binds are untouched (no-op path). Third case: mixed text and blob rows — only the
   text rows change. Fourth case — **the duplicate-pair scenario, exercised per table with that table's own `UNIQUE`
   tuple**: for `proxmox_scaling_defaults` seed a text row and a blob row sharing `(tenant_id, plugin_config_id)`;
   for `proxmox_scaling_item_overrides` seed a pair sharing `(software_item_id, plugin_config_id)` while their
   `tenant_id`s may differ (that column is not in the key — this is the arm a uniform probe would get wrong). Assert
   the repair deletes the text row, keeps the blob row's values untouched, and completes without a `UNIQUE`
   violation.
4. **Dialect-safety guard.** If a Postgres harness is available in CI for this crate, run the full migration chain
   against Postgres and assert runtime upsert + entity read-back succeed (this now also covers Defect 2). If not,
   the floor is a source-scoped token guard: the Migration C block contains no `randomblob` / `hex(` tokens, and
   Migrations A/B contain no raw `TEXT`-typed uuid column declarations. Prefer the real Postgres run.
5. **Runtime round-trip on the builder schema.** One test that upserts via `scaling_store` on a freshly-migrated
   schema and reads back through the entity — if the existing store tests already do this against the full migration
   chain, point at them instead of duplicating (verify, don't assume).

Tests use no `tokio::time` APIs, so **no `start_paused`** (snapshot rule: only time-API tests get the paused clock).

## Documentation deliverables

- **Doc comments on the three edited migrations + the repair migration**: dialect-portability rationale (ids generated
  in Rust, bound as `Value::Uuid`; uuid columns via `.uuid()` builder), the sqlx blob-only-decode fact that forced it,
  the `seaql_migrations` name-tracking immutability reasoning (safe to edit in place; Postgres never recorded the
  batch), and the declared-type drift note for pre-existing SQLite databases.
- **`docs/development/database-migrations.md`**: add a note under migration-authoring guidance: plugin migrations must
  be dialect-portable **and bind-compatible** — no backend-only SQL functions (`randomblob`, `hex`, `gen_random_uuid`,
  `strftime`, …); uuid columns are declared with the `.uuid()` builder, never raw `TEXT`; generated ids are produced
  in Rust (`Uuid::now_v7()`) and bound as `Value::Uuid`, never `.to_string()` (sqlx's SQLite uuid codec is blob-only);
  timestamps round-trip as values. Canonical home for the rule; link it elsewhere rather than repeating.
- **No API / wire / OpenAPI / CONTEXT.md change** — internal migration mechanics; the externally observable change is
  "Postgres controllers can start and scaling works there; migrated SQLite scaling config becomes readable again".
  **No ADR** — bug fix using existing in-codebase idioms.
- **No dependency change**: `uuid` (with `v7`) and `time` are already dependencies of the proxmox plugin
  (`crates/plugins/infrastructure/proxmox/Cargo.toml`). (Verified — called out so review doesn't re-flag it.)

## Out of scope / deferred

- Codebase-wide audit of raw SQL in **other** migration files (core `crates/shared/db` migrations already dispatch on
  backend; other plugin crates' migrations are a separate finding if one exists). Bounded to
  `controller_migration.rs`.
- Adding foreign keys to the scaling tables (shipped schema has none; adding them now creates drift against
  already-applied instances).
- The transient-node-failure backup-target cache-prune HIGH in the same crate (`discovery.rs` / `policy_store.rs`) —
  different mechanism and file, its own spec (`2026-07-12-proxmox-backup-target-fault-aware-prune-design.md`).

> Note superseding the 2026-07-11 revision's "legacy ids are functional" deferral: that claim rested on
> `Uuid::parse_str` leniency, but the real consumer is sqlx's blob-only SQLite decode — the legacy text rows are
> **not** readable, which is why the repair migration is in scope rather than deferred.
