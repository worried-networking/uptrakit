# Code Review: uptrakit-shared-db

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-shared-db` is the central SeaORM entity library shared across the controller, web-api,
and related crates. The foundation is solid: UUID v7 primary keys, a compile-time `TenantScoped`
trait, typed enums for state columns, and well-declared referential integrity. These are meaningful
architectural wins that prevent entire classes of runtime errors.

Several Medium-severity schema decisions (dual output storage, nullable version, entity test
coverage gaps) require remediation. The `RoleMapping` tests in `oidc_provider.rs` are exemplary
but all other entities have zero inline test coverage.

## Architecture

### Strengths

- `src/lib.rs` -- Clean re-export surface. The `crypto` submodule is a thin re-export shim
  (`pub use uptrakit_crypto::*`) preserving backward compatibility without leaking implementation
  detail.
- `src/entity/tenant_scoped.rs` -- `TenantScoped` trait drives architectural safety. Provides a
  single `tenant_id_column()` associated function. `TenantDb` in `web-api` uses this to make it
  structurally impossible to fetch rows from a tenant-scoped table without applying the tenant
  filter. Ten entities implement the trait.
- `Cargo.toml` -- Dependency graph is intentional. Every dependency is workspace-pinned. No
  transitive version surprises.
- All `Relation` enum variants have matching `impl Related<...>` blocks. No dangling relation
  stubs.

- `src/entity/update_batch.rs` -- Properly models tenant-scoped batch entity with SeaORM
  relations. The batch record ties together tenant ownership, batch metadata, and child
  update history records via declared `Relation` and `Related` implementations.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `src/entity/oidc_provider.rs:80` -- `EncryptedString` used for `client_secret`. AES-256-GCM
  encryption is applied transparently via the `ValueType`/`TryGetable` impls.
- `src/entity/oidc_provider.rs:41-52` -- Sentinel value on serialization error for `RoleMapping`.
  Rather than silently writing `{}`, the `From<RoleMapping> for sea_orm::Value` impl writes
  `{"__serialization_error": true}` and logs at `error` level. Makes data corruption detectable.
- Zero `unsafe` in the crate.
- `Zeroizing` used on sensitive in-memory values via `uptrakit-crypto` dependency.

### Issues

**[LOW]** `src/entity/api_token.rs` -- No `expires_at` column. API tokens are valid indefinitely
until explicitly revoked. There is no mechanism for time-bounded tokens (e.g., "expire this
token after 90 days"). A forgotten or leaked token remains valid until manually revoked. Adding
`expires_at: Option<OffsetDateTime>` and an index on `(user_id, expires_at)` would enable both
user-defined expiration and automated cleanup.

## Code Quality

### Strengths

- Consistent `time::OffsetDateTime` for all timestamp columns. No `chrono` usage.
- `src/entity/update_history.rs:4-15` -- Typed enums for state columns: `UpdateStatus`,
  `OutputStreamType`, `MqttClientConnectionStatus`, `SoftwareDiscoveryState`, and
  `SessionTokenType` are all `DeriveActiveEnum` types. Impossible state values rejected at the
  ORM boundary.
- `src/entity/oidc_provider.rs:10-53` -- `RoleMapping` newtype with complete ORM integration.
  Six tests cover: normal round-trip, empty map, `serde_json` round-trip, special characters,
  default-is-empty, non-JSON rejection, and wrong JSON shape rejection.
- `src/entity/plugin_config.rs:14` -- `plugin_config.config` stored as `serde_json::Value`. Using
  `Json` column type is the correct approach for a polymorphic plugin configuration blob.
- All entities implement `ActiveModelBehavior` (default baseline, correct for entities without
  before-save hooks).

### Issues

**[MEDIUM]** `src/entity/update_history.rs:28` -- Dual output storage: `output` column (inline
Text) and the `update_output_lines` child table. The consuming query in
`web-api/src/queries/update_history.rs` checks `if record.output.is_empty()` and conditionally
loads child rows. No DB constraint ensures only one path is populated. The canonical storage path
should be documented as a code comment on the `output` field.

**[MEDIUM]** `src/entity/available_version.rs:10` -- `version` column is `Option<String>` with no
semantic default. A row with `release_date` but no `version` is ambiguous: the pipeline cannot
answer "is version Y available?" for a NULL-version row. Either make `version` NOT NULL or
document which code paths produce and consume NULL-version rows.

**[MEDIUM]** No unit tests for any entity other than `oidc_provider.rs`. The `RoleMapping` tests
are exemplary, but entities with custom ORM type impls are untested: `EncryptedString`
round-trip through the SeaORM mock driver, `UpdateStatus` enum serialization, and
`OutputStreamType` round-trip.

**[LOW]** `src/entity/update_output_line.rs` and `src/entity/update_history.rs` -- No tests
verifying the dual-storage fallback logic. The conditional `output` vs `update_output_lines`
branching is error-prone enough to warrant a dedicated integration test.

## High Availability

### Strengths

- FK `ON DELETE` actions declared for all relations. Cascades and restrictions explicitly
  declared, preventing orphaned rows.
- Append-only tables for audit data: `update_history` and `controller_events` have no
  `deactivated_at` or soft-delete column.
- UUID v7 primary keys throughout. Time-ordered, eliminating B-tree hot-spot contention on
  insert-heavy tables.
- Partial (filtered) unique indexes for soft-delete:
  `uq_plugin_configs_active_name WHERE deactivated_at IS NULL` and
  `uq_software_items_active_name WHERE deactivated_at IS NULL`.
- `CHECK` constraint on `sessions`: `auth_method != 'oidc' OR oidc_provider_id IS NOT NULL`.
- All ephemeral tables have `expires_at` indexes for efficient TTL cleanup.
- Migration `down()` drops all 28 tables in correct reverse FK dependency order.
- Unique index `uq_host_software_items_active` prevents duplicate assignments.

### Issues

**[LOW]** Migration -- Missing index on `update_history(host_id, software_item_id, status)` for
pending-update lookups. Frequent check requires index intersections without a composite index.

**[LOW]** Migration -- No partial unique index on `oidc_providers` for active names. Unlike
`plugin_configs` and `software_items`, OIDC provider slugs cannot be reused after
soft-deletion.

## Coding Standards

### Strengths

- `Cargo.toml` -- `edition = "2024"` and workspace-pinned dependencies.
- Consistent `time::OffsetDateTime` across all entities. No `chrono` usage.
- `publish = false` correctly set.
- Typed enum serialization follows the `as_str()` / `DeriveActiveEnum` / `FromStr` pattern.
- Zero `#[allow(clippy::...)]` or `#[allow(dead_code)]` annotations.

### Issues

**[LOW]** `src/entity/controller_event.rs:8` -- `id` is `i64` (sequential auto-increment), not
UUID. This is the only entity diverging from UUID v7 PKs and there is no comment explaining the
rationale. A code comment stating "Sequential i64 is intentional: controller_events is an
append-only event log; sequential IDs preserve insertion order for the event poller cursor"
would prevent future reviewers from treating this as a mistake.

## Extensibility

### Strengths

- All entities are `Clone + Debug`, safe for `Arc<>`, `tracing`, and async boundaries.
- `entity/prelude.rs` re-exports all `Entity` and `Model` types from a single path.
- `TenantScoped` is open for extension. New tenant-scoped entities just need an additional
  `impl TenantScoped` block.

### Issues

**[MEDIUM]** `src/entity/plugin_config.rs:11` -- `plugin_type` stored as unvalidated `String`
rather than the `PluginType` enum from `uptrakit-shared-types`. The `PluginType` enum has
`FromStr`, `Display`, and `as_str()` implementations. Raw `String` storage allows invalid
plugin type strings and forces every query helper to call `PluginType::from_str()` individually.
Using `DeriveActiveEnum` on `PluginType` (or a newtype wrapper) would push validation to the
ORM boundary.

**[LOW]** `src/entity/plugin_config.rs:14` -- `plugin_type` as `String` provides forward
compatibility at storage level (new plugin types work without migration), but typos in manual
edits produce runtime errors rather than constraint violations.

## Tests

### Strengths

- `src/entity/oidc_provider.rs:130-220` -- Seven tests for `RoleMapping`: normal round-trip,
  empty map, `serde_json` round-trip, special characters in role names, default-is-empty,
  non-JSON rejection, and wrong JSON shape rejection. These are exemplary unit tests at the
  ORM type boundary.
- `src/migration/mod.rs:43` -- Migration smoke test runs all 28 table creations and
  rollback against an in-memory SQLite instance, verifying the migration `up()` and `down()`
  sequences are syntactically valid and execute without error.
- `src/migration/mod.rs:92-98` -- Migration smoke test verifies `update_batches` table and
  `batch_id` column exist after running all migrations.

### Issues

**[MEDIUM]** No unit tests for any entity other than `oidc_provider.rs`. Entities with
custom ORM type implementations are untested in isolation: `EncryptedString` round-trip
through the SeaORM mock driver is not exercised (even though `uptrakit-crypto` tests it at
the primitive level); `UpdateStatus`, `OutputStreamType`, `SoftwareDiscoveryState`, and
`SessionTokenType` `DeriveActiveEnum` serialisations have no per-entity tests confirming
the DB column values match their Rust strings.

**[LOW]** `src/entity/update_output_line.rs` and `src/entity/update_history.rs` -- No tests
for the dual-storage fallback logic. The conditional `output` vs `update_output_lines`
branching path mentioned in the Code Quality section is too complex for implicit coverage
from the migration test alone.

## Database

### Strengths

- `src/entity/tenant_scoped.rs` -- The `TenantScoped` trait is the single most important DB safety
  feature in the codebase. It makes cross-tenant data leakage structurally impossible at the ORM
  level for all ten tenant-scoped entity types. Any new entity that carries a `tenant_id` merely
  needs one `impl TenantScoped` block to gain the full protection.
- `src/entity/revoked_token_jti.rs` and `src/entity/revoked_token_user.rs` -- These two tables are
  an elegant persistence layer for an in-memory security structure. The `expires_at` / `purge_after`
  columns carry exactly enough metadata for the `auth_cleanup` scheduler task to reclaim rows
  precisely when the underlying tokens would have expired naturally, preventing unbounded table growth.
- UUID v7 primary keys across all 30+ entities. Time-ordered UUIDs eliminate B-tree index hot-spot
  contention on high-insert tables like `update_history`, `notification_log`, and `sessions`, which
  would otherwise experience significant page splits with random UUIDs or sequential integers.
- `src/entity/session.rs` -- The compound `(user_id, expires_at)` index added in
  `m20260302_000001_add_missing_indexes.rs` is a proper covering index for the frequent
  "list active sessions for user" query pattern, avoiding table reads after the index scan.
- `src/entity/crl_cache.rs` -- Intentional absence of a foreign key on `ca_fingerprint` is
  correctly documented: it allows the CRL cache row to survive a CA record deletion without
  cascading. The trade-off is documented in both the entity doc comment and the migration.
- `src/entity/host_software_item.rs:15-16` -- The `JsonBinary` column type on
  `latest_release_metadata` is correctly chosen over plain `Json`: binary JSON avoids
  re-parsing on every read in SQLite, which stores the serialized form directly.
- `src/migration/m20260302_000001_add_missing_indexes.rs` -- Dedicated follow-up migration
  adding six missing indexes is cleaner than retrofitting the initial migration. The composite
  `(plugin_config_id, package_identifier)` index on `host_software_item_plugins` directly
  supports the autodiscovery ignore-rule lookup pattern used in `queries/autodiscovery.rs`.
- `src/entity/settings_version.rs` -- Three-column versioning table (`version`,
  `global_version`, `revocation_version`) provides a cheap cross-instance change-detection
  probe without scanning the full `settings` table. Polling a single integer row is
  O(1) regardless of how many settings exist.

### Issues

**[HIGH]** `src/entity/update_history.rs:32` -- `update_history.output` is declared as a
`not_null()` TEXT column with no default in the migration DDL, yet the application always
inserts it as an empty string (`output: Set(String::new())`) and then reads it conditionally
(`if record.output.is_empty()`). An empty string and NULL are semantically different in
SQL but are being used interchangeably here. More critically, the `output_bytes` column is a
second source of truth about the content: a record can have `output = ""`, `output_bytes = 0`,
and non-empty rows in `update_output_lines` simultaneously. No DB CHECK constraint prevents
`output` being non-empty while `update_output_lines` rows also exist for the same record,
leaving both paths populated. This is a data integrity gap -- there is no enforcement at the
database level that the two storage paths are mutually exclusive.

**[MEDIUM]** `src/entity/notification_log.rs:11` -- `notification_log.status` is `String` with a
DB default of `"pending"` but no CHECK constraint restricting values to `{pending, delivered,
failed}`. An application bug or direct DB write can insert an arbitrary string that
`NotificationDeliveryStatus::parse()` in `queries/notifications.rs:439-448` cannot handle,
silently defaulting to `Pending` with a `tracing::warn!`. The `notification_channel.channel_type`
and `notification_rule.event_type` columns have the same vulnerability: unconstrained TEXT
with parse-time fallback. Adding CHECK constraints (or migrating to `DeriveActiveEnum` typed
columns where the DB backend supports it) would catch invalid values at write time.

**[MEDIUM]** `src/entity/enrollment_token.rs:15` -- `allowed_capabilities` is stored as a
JSON-serialized string (`Option<String>` with JSON content) in a plain TEXT column rather than
using the `Json` column type. This means the DB has no knowledge that the column contains
JSON. The `model_to_response` deserializer in `queries/enrollment_tokens.rs:26-29` uses
`.and_then(|s| serde_json::from_str(s).ok())`, silently treating a parse failure as an absent
value. If the column contains malformed JSON (e.g., from a direct DB edit), the capability
restriction is silently dropped instead of causing an error. Storing as `Json` column type
would enable DB-level structural validation.

**[LOW]** `src/entity/revoked_token_jti.rs:20` and `src/entity/revoked_token_user.rs:15-22`
-- `expires_at`, `iat_cutoff`, and `purge_after` are stored as `i64` Unix timestamps (seconds
since epoch) rather than `OffsetDateTime`. This is inconsistent with every other timestamp
column in the codebase, which uses `OffsetDateTime` (stored as `TIMESTAMP WITH TIME ZONE`).
The `i64` choice means the DB cannot apply timestamp-aware comparisons or timezone formatting,
and a direct SQL query (`SELECT * FROM revoked_token_jtis WHERE expires_at < ?`) requires
knowing the epoch convention. A note explaining the rationale (e.g., "matches the JWT `exp`
claim which is always a Unix timestamp integer") would prevent future confusion.

**[LOW]** `src/entity/notification_log.rs` -- `notification_log` has no `updated_at` column.
The `delivered_at` column partially fills this role, but the `action_taken` column (populated
when a user acts on a notification email link) can change after delivery without any audit
trail of when the action was recorded. An `updated_at` column would provide a complete
write history.

**[LOW]** `src/entity/update_batch.rs:11-12` -- `batch_type`, `actor_type`, and `actor_id`
are all `String` columns with no CHECK constraint or enum backing. Any string value can be
inserted, and invalid values are only caught at application-level parsing. Unlike
`UpdateStatus` which uses `DeriveActiveEnum`, these columns have no DB-level validation.

**[LOW]** `src/entity/mqtt_client.rs` -- The `mqtt_clients` table has no unique constraint on
`(tenant_id, client_id)`. MQTT `client_id` values must be unique within an MQTT broker
namespace. If two `mqtt_client` rows for the same tenant share a `client_id`, the second
connection attempt will evict the first from the broker, causing a disconnect loop. A unique
constraint on `(tenant_id, client_id)` would prevent this misconfiguration at the DB layer.

## Database -- Additional Findings (2026-03-06)

### Migration Quality

**[INFO]** `src/migration/mod.rs` -- Migration ordering in the `migrations()` vector does not
match chronological file naming. For example, `m20260302_000002_host_packages` (file date 0302)
appears after `m20260303_000001_global_settings` (file date 0303). This was intentional
(migrations reordered to resolve FK dependencies) but a comment at the top of the `migrations()`
function explaining this would prevent future contributors from "fixing" the order.
*(2026-03-06 parallel review -- maintainability, database)*

**[INFO]** No automated check that migration files are registered in `Migrator::migrations()`.
A future contributor could add a migration file but forget to register it. A compile-time or
test-time assertion that all `m20*` files in the directory appear in the vector would be a
useful guardrail.
*(2026-03-06 parallel review -- maintainability)*

**[LOW]** `src/migration/m20260312_000001_system_enrollment_tokens.rs:81` -- Raw SQL
`DELETE FROM global_settings WHERE key = 'system_services.enrollment_token'` could be expressed
with `Query::delete()` from sea_query. Per the migration standards, plain DML should use the
builder API when possible.
*(2026-03-06 parallel review -- database)*

### Tenant Isolation

**[INFO]** Two entities with `tenant_id` are missing `TenantScoped` impl:
`autodiscovery_ignore` (`src/entity/autodiscovery_ignore.rs:9`) and `mqtt_lease`
(`src/entity/mqtt_lease.rs:9`). For `mqtt_lease`, direct `Entity::find()` calls are used
extensively in `mqtt_lease_coordinator.rs` (19 occurrences). While some may be system-level
operations, this bypasses the compile-time tenant isolation guardrail. At minimum,
`autodiscovery_ignore` should implement `TenantScoped` since it is directly involved in
per-tenant discovery operations.
*(2026-03-06 parallel review -- architecture, database)*

**[INFO]** `TenantDb::db()` is public. While documented as intentional (needed for raw
`DatabaseConnection` operations), `tenant_db.db()` returns an unscoped connection. Any code
path using `Entity::find().all(tenant_db.db())` silently bypasses tenant isolation. Consider
renaming to `db_unscoped()` or adding a lint comment pattern to flag intentional usage.
*(2026-03-06 parallel review -- architecture)*

### Positive Findings (2026-03-06)

- `src/migration/m20260302_000003_host_packages_has_update.rs` -- Exemplary migration: handles
  all three partial-run states for crash recovery, uses the approved SQLite table-recreation
  pattern, includes proper down migration, and adds covering indexes.
  *(2026-03-06 parallel review -- database)*
- 29 migrations with both `up()` and `down()`. Irreversible migrations (UUID repair, datetime
  repair) have explicit no-op `down()` methods with comments.
  *(2026-03-06 parallel review -- maintainability)*

---

## Review — 2026-03-10

### Summary

This review adds findings from a migration-quality, schema-integrity, and tenant-isolation pass
on 2026-03-10. Several issues are new; existing open issues from prior rounds are confirmed
where still unresolved.

### Database

**[MEDIUM]** `src/migration/mod.rs:51-91` — Migration execution order in the `migrations()`
vector does not match the alphabetical date-based file naming for several entries. `sea-orm-migration`
uses the `name()` string as the identifier, so the vector position controls actual execution
order. A developer adding a migration at the wrong vector position could silently break the
schema. Recommendation: add a prominent comment at the top of the `migrations()` function
stating that the vector is the authoritative execution order and must not be reordered; consider
a CI test that panics if any migration with an earlier date appears after one with a later date.
*Prior finding (2026-03-06, `[INFO]`) confirmed and promoted to `[MEDIUM]` based on risk.*

**[MEDIUM]** `src/migration/m20260209_000001_initial.rs` — `update_output_lines.update_history_id`
has no index in the initial migration. Queries filtering on this column (used in
`web-api/src/queries/update_history.rs:61,171` to load output lines for a history record) perform
full table scans as the table grows. Recommendation: add a migration creating
`idx_update_output_lines_update_history_id`.

**[MEDIUM]** `src/migration/m20260318_000002_cron_to_interval.rs:265-270` — Raw SQL UPDATE uses
`format!` string interpolation for a `'{task_type}'` literal value. No user input is involved
here, but establishing the pattern of interpolating string values into raw SQL fragments is
dangerous precedent that future contributors may imitate with user-controlled values.
Recommendation: replace with a parameterized `sea_query` statement.

**[LOW]** `src/migration/m20260309_000003_host_tags.rs` — `down()` relies on implicit cascade to
drop the `uix_host_tags_tenant_name` partial index when the table is dropped. The `up()` method
explicitly creates the index with a named identifier. An asymmetric `down()` is fragile if the
DB engine's cascade behavior differs between SQLite and PostgreSQL. Recommendation: add an
explicit `DROP INDEX IF EXISTS uix_host_tags_tenant_name` before dropping the table.

### Strengths (2026-03-10)

- All migrations implement `down()`. Large structural migrations use the rename-temp-table pattern
  with crash-recovery detection. `run_migrations` wraps all migrations in a single transaction.
  Confirmed correct.
- Partial unique index `uix_update_history_host_active` provides database-level enforcement of
  "at most one active update per host". Confirmed correct.
- Three-format ciphertext design (`v1`/`v2`/`v3`) with backward compatibility in
  `uptrakit-crypto` is correctly handled by the migration path. Confirmed correct.
