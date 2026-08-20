# Clippy-Enforced Raw-SQL Ban — Design

Date: 2026-08-17. Status: draft (pending `/review-spec`).

## Problem

AGENTS.md rule "No raw SQL" is prose-enforced only. The `begin_immediate()` invariant got clippy
`disallowed-methods` backing; raw SQL did not. Consequence (mistake ledger,
`mistake-invariant-exception-false-rationale`, 2026-08-17): an implementer took a raw-SQL exception with a
false rationale and no inline justification comment, and nothing mechanical caught it. This spec adds
compiler enforcement plus a documented escape-hatch policy so every exception is visible, justified, and
auditable. Scope honesty: the gate mechanically closes the *undeclared-exception* mode; the
*false-rationale* mode itself is not machine-checkable and remains the reviewer's verification duty
(§ Design, § 2) — the gate's contribution there is forcing every rationale to exist, be colocated, and
name a taxonomy category the reviewer can test.

## Goal

1. Any new raw-SQL entry point fails `cargo clippy` unless annotated with
   `#[expect(clippy::disallowed_methods, reason = "…")]` (or `disallowed_macros`) whose reason names a
   concrete limitation from the approved taxonomy.
2. Existing sites are either rewritten to builders or annotated per that taxonomy.
3. The "when is an exception legitimate" policy is written down in canonical docs.

## Verified current state (sea-orm 2.0.1, sea-query 1.0.1 — from Cargo.lock, checked against crate source)

Raw SQL reachable from this workspace's connection types enters SeaORM through three construction
sources plus one direct-string method:

- `sea_orm::Statement::from_string`, `sea_orm::Statement::from_sql_and_values` — the only `Statement`
  constructor *methods*. `Statement`'s fields are all `pub` (statement.rs:14-23), so a struct-literal
  construction is possible and lint-invisible (`disallowed-methods` cannot match a literal) — a
  documented gap alongside the sqlx/driver-inherent ones below; nobody writes it accidentally, and the
  gate targets accidental raw SQL, not evasion.
- `sea_orm::raw_sql!` — proc macro producing a `Statement`.
- `sea_orm::ConnectionTrait::execute_unprepared` — takes `&str` directly, bypassing `Statement`.

Every `Statement` consumer (`execute_raw`, `query_one_raw`, `query_all_raw`, `stream_raw`,
`Select::from_raw_sql`, `FromQueryResult::find_by_statement`, `TryGetableMany::find_by_statement`,
`SelectorRaw::from_statement`) takes a `Statement` by value, so banning the sources chokes all of them —
the consumers are deliberately NOT listed (each extra path is maintenance surface with no added coverage).

sea-query additionally offers raw-fragment hatches inside builders: `Expr::cust`, `Expr::cust_with_values`,
`Expr::cust_with_expr`, `Expr::cust_with_exprs`, `Expr::custom_keyword`, `Func::cust`.

Bypass surface outside sea-orm: `sqlx` is a direct workspace dependency, but repo-wide usage is
pool/connect-option types only — zero direct `sqlx::query*`/`sqlx::raw_sql`/`Executor` query-API calls.
Deliberately unbanned (same zero-usage policy as the `Expr::cust` tail in § 1); revisit if one appears.
Likewise sea-orm's driver connection types (`sqlx_sqlite`/`sqlx_postgres`/`sqlx_mysql`/`rusqlite`
modules) expose inherent `execute_unprepared` methods that do not resolve through `ConnectionTrait`;
the workspace never holds driver types directly, so these are a documented zero-usage gap, not ban
entries.

Call-site census (2026-08-17; counts are a snapshot — the implementation plan re-greps at dispatch time and
the grep output, not this table, is the authoritative edit set):

| Bucket | Sites |
| --- | --- |
| Prod (non-test, non-migration) | 13 |
| Migration bodies | 84 |
| Migration files' `#[cfg(test)]` halves | 54 |
| Test code (non-migration files) | 49 |
| Total | 200 |

API mix: `execute_unprepared` 158, `query_one_raw` 22, `query_all_raw` 16, `Statement::from_string` 34
(all nested inside the raw query calls), `Statement::from_sql_and_values` 2, `execute_raw` 2. Zero usage
repo-wide of `raw_sql!`, the `Expr::cust` family, `Func::cust`, `from_raw_sql`, `find_by_statement`,
`from_statement`, `stream_raw`.

Relevant existing enforcement: workspace lints set `clippy::all = deny`, `allow_attributes = deny`,
`allow_attributes_without_reason = deny`, `unfulfilled_lint_expectations = deny` — so annotations MUST be
`#[expect(..., reason = "…")]`, and an `#[expect]` that stops firing fails the build (self-cleaning).
`clippy.toml` already carries the `begin_immediate()` `disallowed-methods` block; this spec extends the
same mechanism. No new CI wiring is needed: every existing clippy gate (pre-push, CI `-D warnings` runs)
evaluates `clippy.toml`.

## Design

Mechanism choice: the repo's other enforcement idiom — a frozen, shrink-only `ci/` allowlist (several
precedents, e.g. `verify_no_raw_body_extractors`'s FROZEN allowlist) — was considered and rejected:
category-1 raw SQL legitimately recurs (every future SQLite table-recreation migration), so a
shrink-only frozen list mismodels the domain and would route each legitimate new case through a
gate-script amendment. `#[expect]` is colocated, IDE-visible, and self-cleaning
(`unfulfilled_lint_expectations = deny` deletes dead exceptions) — none of which an allowlist file
provides. Limit stated plainly: this gate makes every exception **declared, visible, and
self-cleaning**; it cannot validate that a `reason` string is true — that remains the reviewer's check
(verify the stated rationale independently, per § 2).

### 1. clippy.toml ban list

Append to the existing `disallowed-methods` list (reasons follow the existing entries' style: name the
replacement, then the hazard; final wording at implementation time):

- `sea_orm::Statement::from_string` — raw SQL banned; use sea_query builders
  (docs/development/database-migrations.md § No raw SQL).
- `sea_orm::Statement::from_sql_and_values` — same.
- `sea_orm::ConnectionTrait::execute_unprepared` — same; also note `DatabaseConnection::ping()` for
  connectivity probes.
- `sea_query::Expr::cust` — raw SQL fragment inside a builder; use typed expressions.
- `sea_query::Expr::cust_with_values` — same.

Add a new `disallowed-macros` section:

- `sea_orm::raw_sql` — raw SQL banned; use sea_query builders.

Deliberately unbanned: `Expr::cust_with_expr`, `Expr::cust_with_exprs`, `Expr::custom_keyword`,
`Func::cust` (zero usage, obscure; add later if one appears in review) and all `Statement` consumers
(unreachable once sources are banned).

### 2. Escape hatch

Form: `#[expect(clippy::disallowed_methods, reason = "<taxonomy category>: <concrete limitation>")]`
(`clippy::disallowed_macros` for `raw_sql!`). The reason must name the real limitation — never a claim
about a dependency's API that has not been verified (ledger: `mistake-invariant-exception-false-rationale`;
the reviewer's check is: verify the stated rationale independently, not just the conclusion).

Granularity:

- Default: statement-level (on the `let`/expression statement containing the call).
- Migration `up()`/`down()` fns where every raw-SQL call shares a single rationale (e.g. the SQLite
  table-recreation pattern): one fn-level `#[expect]` with one reason — **only if the fn contains no
  transaction opener**. `disallowed_methods` is a single lint carrying the `begin*` bans too, so a
  fn-level `#[expect]` in a fn that also calls `begin_immediate` would silence a later stray `.begin()`
  (real shape: `m20260309_…_fix_permission_created_at_format.rs` and `m20260512_…_drop_file_keys.rs`
  both open transactions inside `up()`). Such fns stay statement-level per call, as do
  mixed-rationale fns. Fn-level is safe here only because migration files are append-only — future
  work lands in new files, so the attribute cannot silently cover later additions; do not cite this
  allowance as precedent outside migrations.
- Never file-level (`#![expect]`) — it silences the gate for future edits to the whole file, and copied
  file-level suppressions are a known failure shape (ledger: `mistake-wrong-pattern-exemplar`).
- Expression-position calls (closure body, match arm, argument position) have no stable attribute
  attachment point: bind the call to a `let` and annotate that statement.

### 3. Exception taxonomy (the "when to use it" policy)

> **Correction (2026-08-20, from implementation).** Three claims below were falsified while
> implementing this spec, each checked against the pinned sea-query 1.0.2 source and a live SQLite:
> the category-1 examples (read `PRAGMA` shapes, window functions, functional indexes, `typeof()`,
> `ALTER COLUMN … TYPE … USING`) are all builder-expressible and do **not** qualify; the category-2
> `ping()` guidance is inverted (`DatabaseConnection::ping()` executes no SQL on SQLite, so health
> checks use a builder `SELECT 1` instead); and the `pragma_table_info` probe in § 5 is expressible
> via the read-only `pragma_*` table-valued functions, so it is frozen under category 4, not
> inexpressible under category 1. The shipped taxonomy —
> [coding-standards.md § Raw-SQL ban](../../development/coding-standards.md) — is canonical; the text
> below is preserved as the design-time record. Do not cite it for what sea_query can express.

A site may carry the `#[expect]` only when it falls into one of:

1. **Builder limitation** — SQL genuinely inexpressible in sea_query, wherever it occurs (not
   migration-only): SQLite `ALTER TABLE`/`PRAGMA` shapes (incl. `PRAGMA foreign_keys` toggles in test
   setup), `CREATE DATABASE` (no sea_query builder), window functions, functional indexes, `typeof()`.
   The reviewer test is falsifiable: "is this genuinely inexpressible?" Canonical detail:
   docs/development/database-migrations.md § No raw SQL.
2. **Connectivity probe** — only where `DatabaseConnection::ping()` is genuinely unavailable (e.g. probing
   over a generic `ConnectionTrait`); plain health checks must use `ping()` instead.
3. **Test-only schema sabotage** — corrupting schema/data to exercise DB-failure paths, only where the
   sabotage is genuinely inexpressible via builders. Plain `DROP TABLE` does not qualify — it goes
   through the builder-based form in § 4 with no annotation.
4. **Frozen merged migration** — raw SQL inside an `up()`/`down()` body of a migration merged to
   `main`, regardless of builder-expressibility. Merged migrations are treated as applied (a
   per-deployment "has it run" fact is not repo-observable); rewriting an `up()` risks
   live-vs-fresh-install divergence, and `down()` bodies are frozen with them on blast-radius grounds
   (down-paths are largely untested; a rewrite adds risk with no gain). Unmerged migrations on a
   branch may still be freely revised. This is deliberately a separate category from 1 so that
   builder-expressible frozen sites (e.g. a plain `DROP TABLE IF EXISTS` in an old `down()`) never
   carry a false "inexpressible" claim, and a grep for this category's reason prefix bounds the
   frozen set. Migration files' `#[cfg(test)]` halves are NOT migration bodies — they never ran on any
   deployment and are ordinary test code, rewrite-by-default.

A "crate-boundary unnameable entity" category was considered and dropped: `sea_query::Alias` table refs
need no entity, so builder rewrites discharge every candidate site by construction (the one real
candidate, `runtime_support.rs`, is rewritten in § 5). Re-add via § Deferred only if a genuinely
builder-inexpressible cross-crate case appears.

Anything else: rewrite with builders. `disallowed-methods`/`disallowed-macros` have NO test exemption
(unlike `allow-unwrap-in-tests`; ledger `mistake-clippy-test-exemption-scope`), so test code follows the
same taxonomy.

### 4. Shared test helper

`DROP TABLE` is expressible without raw SQL: `sea_query::TableDropStatement` implements
`StatementBuilder` (verified in sea-orm 2.0.1's `build_schema_stmt!`), so
`db.execute(Table::drop().table(Alias::new(name)).to_owned())` needs no `#[expect]` at all. Add
`drop_table(db, table_name)` to `crates/ui/web-api/src/test_harness/` wrapping that builder call; all
web-api route/integration tests that currently inline `execute_unprepared("DROP TABLE …")` switch to it.
`DROP TABLE` test sites exist only in web-api (verified by grep), so no shared cross-crate
test-support dependency is needed; other crates' test raw-SQL sites are different statement shapes,
handled per § 5.

### 5. Remediation of existing sites

Rewrites (raw SQL removed, no `#[expect]` needed):

- `crates/ui/web-api/src/routes/health.rs` (`SELECT 1`) → `DatabaseConnection::ping()` (verified present
  in sea-orm 2.0.1).
- `crates/core/controller-runtime/src/reload/db_pool.rs` (`SELECT 1`) → `ping()`.
- `crates/core/agent-ssh-runtime/src/runtime_support.rs` (`DELETE FROM proxmox_host_state`) →
  sea_query `DeleteStatement` with `Alias::new("proxmox_host_state")` table ref, executed via the
  non-raw `execute` (takes a `StatementBuilder`). No entity required, so the crate boundary is not an
  obstacle for this expression-level fix; the deeper ownership cleanup is the deferred bead.
- `crates/shared/db-tx/src/lib.rs` `busy_snapshot_tests` — the module builds its `CREATE TABLE`/
  `INSERT`/`SELECT` statements with sea_query, then string-renders them into `execute_unprepared`;
  rewrite to pass the builders directly to `ConnectionTrait::execute`/`query_one`
  (`TableCreateStatement`/`InsertStatement`/`SelectStatement` all implement `StatementBuilder`). The
  module's header comment claiming the entity-less crate "has no plumbing to send a built query through
  any other way" is false (verified against sea-orm 2.0.1) and is removed in the same change — no
  taxonomy category covers builder-expressible test DML. This is the exact
  `mistake-invariant-exception-false-rationale` shape, sitting in the same file as the canary module
  (`busy_snapshot_tests` is a sibling of `mod tests`, which hosts the § 6 canaries); a naive
  grep-and-annotate pass would wrongly `#[expect]` it.

`proxmox/src/agent/migration.rs` carries four raw-SQL sites (two `INSERT OR IGNORE … SELECT`
statements, a `sqlite_master` table-existence probe, a `pragma_table_info('ssh_hosts')` column probe).
All four are **annotated, not rewritten**, under the applied-migration freeze (category per site: the
all four are category 4 — see the § 3 correction: the `pragma_table_info` probe was assumed
inexpressible at design time and is not): these migration bodies
have already run on the live deployment, so a rewrite changes what fresh installs execute relative to
what the live DB already absorbed (e.g. `ON CONFLICT DO NOTHING` is narrower than SQLite's
`INSERT OR IGNORE` — it does not suppress NOT NULL/CHECK violations), and the divergence is
undetectable by tests, which only ever run the new form. The freeze rule is taxonomy category 4: raw
SQL in any migration body merged to `main` is annotated, never rewritten; rewrites are restricted to
everything else — including migration files' `#[cfg(test)]` halves, which are ordinary test code
(§ 3). Consequence, stated plainly: the ~84 merged migration-body sites become annotated-and-frozen
rather than actively gated; the gate's live rewrite-or-justify pressure applies to the other ~116
census sites and to all future code. For new migrations the gate forces a declared category-1 claim
before merge, whose truth (genuine inexpressibility) remains the reviewer's check; after merge the
freeze forbids rewriting it, but the category-1 label stays (still true). Category 4 therefore only
ever labels this remediation pass's legacy set — builder-expressible sites the freeze exempts from
rewriting.

Everything else follows the § 3 default: builder-expressible sites are **rewritten** (e.g. the
`functional-tests/tests/proxmox_update_lifecycle.rs` `SELECT … LIMIT 1` table probes are plain
`Query::select()` over `Alias` refs — same shape as `runtime_support.rs`); only genuinely
inexpressible sites are annotated per taxonomy. Annotated sites that already carry justification
comments keep their prose and gain the `#[expect]` with the reason distilled from it; sites without
comments get both. The census in § Verified current state locates them; the plan re-greps for the
authoritative list.

### 6. Gate verification protocol

Before the change lands, each ban class must be observed to fire **in the real workspace** (full
`[workspace.lints]` + the repo's `clippy.toml` — never a scratch crate; ledger
`mistake-unfaithful-scratch-probe`), via a temporary deliberate violation compiled with
`cargo clippy -p <crate> --all-targets`:

1. Inherent method: `Statement::from_string`.
2. Trait method: `execute_unprepared` called on a `DatabaseConnection`. Near-certain to pass — the
   existing db-tx canary proves trait-path `disallowed-methods` entries match calls on concrete
   receivers (`db.begin()` on `&DatabaseConnection` fulfills the `TransactionTrait::begin` canary).
   Fallback if it somehow doesn't: ban the concrete impl paths instead
   (`DatabaseConnection`/`DatabaseTransaction` et al. `::execute_unprepared`).
3. Re-exported sea-query path: `Expr::cust` (def-path resolution through the re-export is the known flaky
   spot).
4. Proc macro: `raw_sql!` via `disallowed-macros`.

Fallback if the `raw_sql` macro path cannot be matched (try `sea_orm::raw_sql` first, then
`sea_orm_macros::raw_sql`): ban the raw `Statement` consumers instead
(`ConnectionTrait::{execute_raw, query_one_raw, query_all_raw}`, `StreamTrait::stream_raw`,
`Select::from_raw_sql`, `FromQueryResult::find_by_statement`, `TryGetableMany::find_by_statement`,
`SelectorRaw::from_statement`) — that closes the funnel from the consumption side at the cost of ~40
additional annotation sites (the existing `query_one_raw`/`query_all_raw`/`execute_raw` calls). Record
whichever branch was taken in the coding-standards section. The same fallback logic applies if the
`Expr::cust` paths cannot be matched: drop those two entries and note the gap (they guard zero current
usage; the whole-statement funnel remains closed).

Verification violations are removed before commit. Permanent canaries ARE added, extending the existing
`crates/shared/db-tx` canary precedent (its `#[cfg(test)]` module already carries one
`#[expect(clippy::disallowed_methods, reason = "canary: …")]` call per banned `begin*` path): one canary
call per new ban entry, colocated in that db-tx canary module (`sea_orm::sea_query` is unconditionally
re-exported, and db-tx inherits the workspace sea-orm `macros` feature — verified in root `Cargo.toml`
and db-tx's manifest — so `raw_sql!` is reachable there too). Rationale: if an upgrade renames or relocates a
banned path, the unresolvable `clippy.toml` entry degrades to a config warning that `-D warnings` does
not deny; the canary's `#[expect]` then goes unfulfilled and `unfulfilled_lint_expectations = deny`
fails the build. Real annotated sites provide this de facto for heavily-used entries, but the
zero-usage entries (`Expr::cust`, `Expr::cust_with_values`, `raw_sql!`) carry no live `#[expect]` at
all without a canary — their bans would die silently on upgrade, recreating the exact "nothing
mechanical caught it" problem this spec exists to fix.

## Documentation deliverables

- `docs/development/coding-standards.md` — new subsection under Lint Suppressions: the raw-SQL ban list,
  the `#[expect]` form, granularity rules, and the four-category taxonomy (canonical home of the
  taxonomy, including the merged-migration freeze). Cross-link to database-migrations.md for
  category-1 detail.
- `docs/development/database-migrations.md` § "No raw SQL — use sea_query builders for DML" — note that
  the ban is now clippy-enforced, link to the coding-standards subsection, keep owning the
  migration-qualification detail.
- `AGENTS.md` MUST-FOLLOW rule "No raw SQL" — one added sentence: clippy-enforced via
  `disallowed-methods`/`disallowed-macros`; escape hatch documented in coding-standards.md.
- No ADR: the invariant already exists; this is enforcement tooling, matching the un-ADR'd
  `begin_immediate()` ban precedent. No new gate command, so quality-gates.md is untouched.
- `.superpowers/standards-snapshot.md` regenerates automatically on next staleness check (clippy.toml and
  the touched docs are watched sources) — not a hand-edited deliverable.

## Testing

- Gate verification protocol (§ 6) is the test of the gate itself — one deliberate violation per ban
  class, observed to fire, then removed (ledger `mistake-undercovered-gate-modes`: every mode of the gate
  exercised, including the macro mode and the re-export mode).
- `drop_table` helper: exercised transitively by every migrated test; it needs no dedicated test (a
  helper wrapping one builder statement — testing it would test sea-orm, against the "do not test
  upstream crate behavior" rule).
- Canary module (§ 6): checked at build time — `#[expect]` fulfillment under
  `cargo clippy --all-targets` IS the check (db-tx precedent); no runtime assertions.
- Rewritten sites (§ 5): covered by existing tests (health endpoint tests, reload tests, agent-ssh
  factory-reset flow tests); the plan verifies each rewrite's covering test exists and still passes, and
  adds one where coverage is missing (success + failure paths per AGENTS.md).
- Full gates after remediation: `cargo clippy --all-targets --all-features` and the db-sqlite variant
  must pass with zero new warnings — proving every one of the ~200 sites is either rewritten or carries a
  fulfilled `#[expect]`.

## Dependencies

No open spec/plan epics overlap (checked 2026-08-17: no open epics touch clippy.toml, the migration tree,
or the raw-SQL sites; the 2026-05-01 clippy-hardening spec is closed and this extends its documented
`#[expect]` pattern). Cross-cycle: none wired.

## Deferred / out of scope

- `uptrakit-5desg` — decouple agent-ssh-runtime from the proxmox plugin's `proxmox_host_state` table via a
  plugin-owned reset seam (filed during grilling; soft-related to this spec's epic). This spec only
  converts the site from raw SQL to an `Alias`-based builder; the ownership boundary stays.
- Banning `Expr::cust_with_expr`/`cust_with_exprs`/`custom_keyword`/`Func::cust` — zero usage; revisit if
  one appears.
- Any relaxation for future legitimate raw-SQL categories — extend the taxonomy in coding-standards.md
  when a real case appears, not speculatively. Includes re-adding a crate-boundary-unnameable-entity
  category, dropped here for zero population (§ 3).

## Success criteria

1. `clippy.toml` bans the five methods + one macro (or the documented fallback set), each ban class
   was observed to fire in-workspace during implementation, and a permanent canary per ban entry lands
   in the db-tx canary module (§ 6).
2. `cargo clippy --all-targets --all-features` (and db-sqlite variant) pass on the remediated tree.
3. The § 5 rewrites (four named sites plus every builder-expressible site found by the re-grep) land
   builder-based (no `#[expect]`); merged migration bodies are annotated, never rewritten (freeze
   rule); every remaining raw-SQL site carries a taxonomy-conformant `#[expect(..., reason)]`.
4. Coding-standards, database-migrations, and AGENTS.md updates land as specified; markdownlint passes.
5. A future raw-SQL addition through any banned path without `#[expect]` fails pre-push and CI clippy
   gates with no new wiring (documented gaps — `Statement` struct literal, sqlx query API,
   driver-inherent methods — excluded; all are zero-usage and not accidental shapes).
