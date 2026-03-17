# Code Review: `uptrakit-shared-db`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The database crate is strong on schema correctness, migration hygiene, and repair tooling. The
biggest remaining risks are: a potential fresh-install migration ordering defect that would prevent
greenfield deployments, a TOCTOU race in the discovery upsert pipeline, and the absence of a
generic stale-update recovery path.

## Strengths

- SQLite table-recreation helpers have explicit crash-recovery semantics instead of relying on
  brittle ad-hoc migration code.
- Migrations execute inside transactions, preventing partial-schema visibility across connection
  pools.
- CAS-style batch dispatch and transactional batch completion guard against multi-controller
  double-dispatch.
- The migration suite contains targeted repair tests for historically tricky storage problems.
- Newer schema work uses clearer indexes and transactional patterns than the oldest migrations.

## Active Findings

### [HIGH] Potential migration ordering defect on fresh installs

- Dimension: database, high availability
- Scope: `crates/shared/db/src/migration/mod.rs`, migration vec ordering
- Why it matters: `m20260302_000001_add_missing_indexes` appears before
  `m20260302_000002_host_packages` in the migrations vec. SeaORM executes migrations in the order
  returned by `MigratorTrait::migrations()`. If the indexes reference tables created by
  `_host_packages`, a fresh-install deployment will fail at startup with "table does not exist"
  when the schema is empty.
- Failure scenario: first-time deployment on a clean database. Existing deployments are not
  affected because the tables already exist from prior upgrade migrations.
- Action: cross-check against `migrations_run_on_empty_sqlite` test in `migration/mod.rs` to
  confirm whether the test catches this ordering on both SQLite and PostgreSQL backends. If the
  test passes, the indexes either tolerate missing tables or the ordering is correct and this
  finding is invalidated.

### [HIGH] TOCTOU race in `find_or_create_software_item`

- Dimension: database, correctness
- Scope:
  `crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs:find_or_create_software_item`
- Why it matters: the three-phase upsert checks for an existing item, then inserts if absent, then
  recovers from a UNIQUE constraint collision by falling back to a `(tenant_id, name)` lookup.
  This fallback does not verify that the colliding row belongs to the same plugin config scope; it
  can silently return a different target's item.
- Failure scenario: two concurrent autodiscovery runs for different plugin configs produce the same
  software name. One insert wins; the other's collision recovery returns the winner's item, causing
  both targets to share the wrong `software_item` row and incorrect plugin config routing.

### [HIGH] The schema depends on operational stale-update cleanup that does not yet exist

- Dimension: database, high availability
- Scope: `update_history` active-update locking plus the surrounding scheduler and query layer
- Why it matters: the partial unique locking pattern is correct, but it assumes the application
  layer will eventually clear orphaned `InProgress` rows. No scheduler executor or age-based
  cleanup currently does this.
- Failure scenario: controller crash, DB failover, host crash, or a dead network link interrupts
  an update after it is marked active. The schema then correctly prevents concurrent work, but
  nothing clears the stranded lock generically.

### [MEDIUM] N+1 sequential plugin role queries in `load_target_for_dispatch`

- Dimension: database, performance
- Scope: `crates/ui/web-api-queries/src/queries/update_dispatch.rs:load_target_for_dispatch`
- Why it matters: the function fires 8–9 sequential database queries to load one software item,
  one host, one service_host link, one service, and then 3–5 separate plugin role rows. This is on
  the critical path for every update trigger (single and batch).
- Failure scenario: under load with a large number of concurrent update triggers, each dispatch
  takes 8–9 database round-trips instead of 3–5 if role queries were batched.

### [MEDIUM] Soft-deleted plugin config filter may not be universally applied

- Dimension: database, correctness
- Scope: `crates/ui/web-api-queries/src/queries/`, various plugin-config-loading queries
- Why it matters: plugin_config rows use a `deactivated_at` soft-delete column. FK constraints
  prevent hard deletion but not soft deletion from being missed. Queries that omit the
  `deactivated_at IS NULL` filter can dispatch updates using stale, inactive configuration.
- Failure scenario: a plugin config is deactivated; a path that does not filter by
  `deactivated_at` loads it; the update runs with incorrect or revoked configuration.
- Note: `update_dispatch.rs` handles this correctly; a workspace-wide audit of all
  plugin-config-loading queries is recommended to confirm complete coverage.

### [MEDIUM] Duplicate migration date/sequence pairs in the migrations list

- Dimension: database, maintainability
- Scope: `crates/shared/db/src/migration/mod.rs`
- Why it matters: two migrations share the date `20260309` and sequence `000003`:
  `m20260309_000003_host_tags` and `m20260309_000003_unified_software_tracking`. The migration
  framework does not enforce unique names. On rollback-and-reapply scenarios, vec insertion order
  determines execution order non-deterministically.
- Action: verify the migrations guide documents that identical date/sequence pairs are safe, and
  confirm integration tests cover this scenario.

### [MEDIUM] Migration history is becoming hard to review safely

- Dimension: maintainability
- Scope: `crates/shared/db/src/migration/m20260209_000001_initial.rs` (2810 lines) and several
  later large migrations
- Why it matters: correctness is still good, but some migration files are now large enough that
  future schema changes will be difficult to audit and reason about in review.
- Failure scenario: a future cross-backend schema change touches one of the monolithic migration
  files and accidentally regresses an older repair or index recreation rule.

### [LOW] MySQL `DROP INDEX IF NOT EXISTS` uses fragile error-string matching

- Dimension: database, portability
- Scope: `crates/shared/db/src/migration/helpers.rs:drop_index_if_exists`
- Why it matters: the function matches error code `1091` via `e.to_string().contains("1091")`.
  If MySQL's error message format changes, the match fails and the migration errors instead of
  silently succeeding.
- Fix: use a more structured error code check or unconditionally log and swallow all MySQL
  DROP-INDEX errors.
