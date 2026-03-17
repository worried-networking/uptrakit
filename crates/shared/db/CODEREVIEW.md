# Code Review: `uptrakit-shared-db`

- Review date: 2026-03-17
- Scope: full 14-dimension review of `crates/shared/db/` (118 .rs files)
- Reviewer: automated code review (Claude)

## Summary

The database crate demonstrates strong schema design with proper FK constraints,
tenant isolation via the `TenantScoped` trait, and well-documented crash recovery
helpers for SQLite table recreation. Migration test coverage is thorough, including
regression tests for UUID storage repair, datetime format repair, and SQLite
connection pool behavior.

The primary remaining risks are: orphaned in-progress update rows with no automatic
cleanup, SSH private keys stored as plaintext TEXT rather than `EncryptedString`,
a thread-local leak path in `CombinedMigrator`, and a migration vec ordering that
has diverged significantly from chronological file naming.

## Strengths

- SQLite table-recreation helpers (`check_crash_recovery`, `drop_original`,
  `rename_temp`) implement a clean three-state crash recovery model with explicit
  documentation of each state transition.
- All migrations execute inside a single transaction via `run_migrations`, preventing
  partial schema visibility across SQLite connection pools. The design comment
  correctly notes that PostgreSQL treats the outer transaction as a harmless savepoint.
- The partial unique index `uix_update_history_host_active` enforces at-most-one
  active update per host at the database level, providing race safety for
  multi-controller deployments.
- `TenantDb` provides a clean abstraction for tenant-scoped queries, covering
  `find`, `find_by_id`, `update_many`, `delete_many`, and the critical
  `find_via_tenant_join` for entities without their own `tenant_id` column.
- Twenty entities implement `TenantScoped`, covering all tenant-bearing tables.
- Migration test suite (`mod tests`) includes targeted regression tests for
  UUID TEXT/BLOB repair, datetime format repair, file-based SQLite pool behavior,
  incremental migration, and permission/role seed data verification.
- `is_unique_constraint_violation` properly delegates to sqlx's typed error kind
  rather than matching error strings.
- Raw settings helpers (`raw_settings.rs`) use `ON CONFLICT` upserts correctly,
  with defensive insert-if-missing for the version counter row.

## Active Findings

### [HIGH] The schema depends on operational stale-update cleanup that does not yet exist

- **Dimension**: database, high availability
- **Scope**: `migration/m20260313_000001_per_host_update_locking.rs`, `entity/update_history.rs`
- **Description**: The partial unique index `uix_update_history_host_active` correctly
  prevents concurrent active updates per host, but no schema-level or scheduler-level
  mechanism exists to expire orphaned `InProgress` or `Pending` rows.
- **Why it matters**: If a controller crashes, a network link dies, or a host becomes
  permanently unreachable during an update, the `update_history` row remains in
  `pending` or `in_progress` status indefinitely. The partial unique index then
  blocks all future updates for that host.
- **Failure scenario**: Controller crash during update dispatch leaves a `pending` row;
  all subsequent update triggers for that host fail with `UpdateAlreadyActive` until
  manual intervention clears the row.

### [MEDIUM] SSH private keys stored as plaintext TEXT

- **Dimension**: security
- **Scope**: `migration/m20260331_000001_ssh_agent_tables.rs:41`, `SshHosts::PrivateKey`
- **Description**: The `ssh_hosts.private_key` column is defined as `TEXT NOT NULL`
  with no encryption. Other sensitive fields in the codebase (e.g.,
  `notification_channels.config`) use `EncryptedString` for at-rest encryption.
- **Why it matters**: SSH private keys are high-value credentials. A database backup,
  SQL injection elsewhere, or accidental log exposure would reveal private keys in
  cleartext.
- **Failure scenario**: An attacker with read access to the database (via backup file,
  SQL injection, or compromised admin panel) obtains all SSH private keys in
  plaintext, enabling lateral movement to every managed SSH host.

### [MEDIUM] `CombinedMigrator` thread-local not cleared on error path

- **Dimension**: correctness, resource management
- **Scope**: `migration/mod.rs:188-198` (`run_migrations_with_plugins`)
- **Description**: `CombinedMigrator::clear()` is called after `up()` succeeds, but
  if `CombinedMigrator::up(&txn, None).await?` fails (returns `Err`), the `?`
  operator returns early and `clear()` is never called. The plugin migrations
  remain in the `PLUGIN_MIGRATIONS` thread-local.
- **Why it matters**: If the same thread later calls `Migrator::migrations()` or
  another combined migration run, the stale plugin migration list could cause
  incorrect behavior. In practice this is unlikely because migration failure
  typically terminates the process, but it violates the principle of clean resource
  management.
- **Failure scenario**: Migration fails on startup, the process catches the error and
  retries on the same thread; the thread-local still holds the previous plugin list,
  causing `CombinedMigrator::migrations()` to return duplicated entries.

### [MEDIUM] Duplicate migration date/sequence pair

- **Dimension**: maintainability, correctness
- **Scope**: `migration/mod.rs:29-30`, `m20260309_000003_host_tags.rs`,
  `m20260309_000003_unified_software_tracking.rs`
- **Description**: Two migration files share the date `20260309` and sequence
  `000003`. SeaORM identifies migrations by the string returned from
  `DeriveMigrationName`, which includes the full filename. So the framework
  treats them as distinct migrations. However, the naming convention implies a
  unique `(date, sequence)` pair, and the duplication creates confusion during
  auditing.
- **Why it matters**: Reviewers and automated tools that sort or group by
  date/sequence will see ambiguous ordering. A future migration that depends on
  one of these may reference the wrong one.
- **Failure scenario**: No runtime failure, but increases the risk of human error
  during future migration authoring.

### [MEDIUM] Migration vec ordering diverges significantly from chronological file naming

- **Dimension**: maintainability
- **Scope**: `migration/mod.rs:66-121`
- **Description**: The `migrations()` vec contains 55 migrations whose order
  diverges substantially from their file date/sequence naming. For example:
  `m20260302_000002_host_packages` (date 0302) appears at position 12 after
  `m20260306_000002_update_batches` (date 0306). `m20260317_000002` appears at
  position 55 (last) after `m20260331_000001` at position 54. At least 15
  migrations are placed out of date order.
- **Why it matters**: The naming convention suggests chronological ordering, but the
  actual execution order is determined solely by vec position. This makes it
  difficult to audit whether a migration's DDL depends on tables or columns
  created by a later-dated but earlier-positioned migration.
- **Failure scenario**: A developer adds a new migration assuming chronological
  ordering and places it at the end of the vec, but it depends on a column added
  by a migration that was shuffled to a later position. The `migrations_run_on_empty_sqlite`
  test would catch this, but only after CI runs.

### [MEDIUM] `run_migrations_debug` reports wrong migration name on partial databases

- **Dimension**: correctness
- **Scope**: `migration/mod.rs:157-181`
- **Description**: The debug function calls `Migrator::up(db, Some(1))` in a loop
  indexed by `i`, then uses `migrations.get(i)` to report which migration failed.
  However, `up(db, Some(1))` applies the next *pending* migration, not migration
  number `i`. If some migrations are already applied (e.g., on an existing database
  with partial upgrades), the reported migration name will be offset from the
  actual failing migration.
- **Why it matters**: This function exists specifically for debugging migration
  failures. Reporting the wrong migration name defeats its purpose.
- **Failure scenario**: On a database where 10 of 55 migrations are already applied,
  the first `up(db, Some(1))` call applies migration 11, but `migrations.get(0)`
  reports migration 1. If migration 11 fails, the developer investigates the wrong
  file.

### [LOW] MySQL `DROP INDEX IF NOT EXISTS` uses fragile error-string matching

- **Dimension**: database, portability
- **Scope**: `migration/helpers.rs:208` (`drop_index_if_exists`)
- **Description**: The function matches MySQL error code `1091` via
  `e.to_string().contains("1091")`. This relies on the error message format
  remaining stable across MySQL/MariaDB versions and sqlx driver versions.
- **Why it matters**: If MySQL's error message format changes or the sqlx driver
  alters its `Display` implementation, the match fails and the migration errors
  instead of silently succeeding on an already-absent index.
- **Failure scenario**: A MariaDB version change reformats the error message; the
  migration fails on a rollback/reapply where the index was already dropped.

### [LOW] `proxmox_host_state` and `proxmox_pending_matches` use TEXT timestamps

- **Dimension**: consistency, coding standards
- **Scope**: `migration/m20260331_000001_ssh_agent_tables.rs:95-101`
- **Description**: The `proxmox_host_state.created_at`, `proxmox_host_state.updated_at`,
  and `proxmox_pending_matches.created_at` columns are defined as `.text().not_null()`
  rather than `.timestamp_with_time_zone().not_null()`. Every other timestamp column
  in the codebase uses `TIMESTAMPTZ` via the `helpers::timestamp()` helper.
- **Why it matters**: TEXT timestamps are not validated by the database engine and
  require manual parsing. They also do not sort correctly if the format varies
  (e.g., with/without timezone offset).
- **Failure scenario**: A code change writes a timestamp in a different format; the
  TEXT column accepts it silently, but later parsing fails or produces incorrect
  sort ordering.

### [LOW] `embedded_service_runtime_states` has no FK to services

- **Dimension**: database, data integrity
- **Scope**: `migration/m20260330_000001_embedded_service_visibility.rs:99-121`,
  `entity/embedded_service_runtime_state.rs`
- **Description**: The `embedded_service_runtime_states.service_id` column is a UUID
  primary key with no foreign key constraint to either `services` or
  `system_services`. The entity also has no `Relation` definitions.
- **Why it matters**: Without an FK constraint, orphaned runtime state rows can
  accumulate when services are deleted, and there is no referential integrity
  guarantee.
- **Failure scenario**: A service is deleted; its runtime state row persists
  indefinitely. Over time, orphaned rows accumulate and confuse HA coordination
  logic that reads this table.

### [LOW] Index migration creates index on table later dropped

- **Dimension**: maintainability
- **Scope**: `migration/m20260302_000001_add_missing_indexes.rs:43-49`
  (creates `idx_mqtt_leases_tenant_id`), `migration/m20260329_000001_drop_mqtt_and_add_service_config.rs:21-28`
  (drops `mqtt_leases` table)
- **Description**: The index migration creates `idx_mqtt_leases_tenant_id` on the
  `mqtt_leases` table. A later migration (`m20260329_000001`) drops the entire
  `mqtt_leases` table. The index migration's `down()` method attempts to drop the
  index, which would fail if the table no longer exists.
- **Why it matters**: Forward-only migrations work correctly (the index is implicitly
  dropped with the table). But a selective rollback of the index migration after
  the table-drop migration has run would fail.
- **Failure scenario**: Developer attempts to rollback the index migration for
  debugging; the `down()` fails because `mqtt_leases` no longer exists. This is a
  low-severity edge case since selective rollbacks of old migrations are rare.

## Resolved / Invalidated Findings

### [INVALIDATED] Migration ordering defect on fresh installs

- **Previously**: `m20260302_000001_add_missing_indexes` was suspected of referencing
  tables from `m20260302_000002_host_packages`.
- **Resolution**: The index migration only references `update_history`,
  `host_software_items`, `mqtt_leases`, `service_hosts`, `sessions`, and
  `host_software_item_plugins` -- all created by the initial migration
  (`m20260209_000001_initial`). None of these tables come from `host_packages`.
  The `migrations_run_on_empty_sqlite` test confirms this.

### [OUT OF SCOPE] TOCTOU race in `find_or_create_software_item`

- **Previously**: Reported as a DB schema issue.
- **Resolution**: This function lives in `crates/ui/web-api-queries/`, not in
  `crates/shared/db/`. The DB schema side is correct: `software_items` has a
  unique constraint on `(tenant_id, name)` that catches duplicate inserts. The
  application-layer TOCTOU concern belongs in the `web-api-queries` review.

### [OUT OF SCOPE] N+1 sequential plugin role queries

- **Previously**: Reported against `update_dispatch.rs`.
- **Resolution**: This code lives in `crates/ui/web-api-queries/`, not in
  `crates/shared/db/`.

### [OUT OF SCOPE] Soft-deleted plugin config filter coverage

- **Previously**: Reported against various query files.
- **Resolution**: The query files live in `crates/ui/web-api-queries/`. The DB
  schema correctly defines `deactivated_at` columns on `plugin_configs` and
  `software_items`.
