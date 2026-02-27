# CODEREVIEW — uptrakit-shared-db

> Reviewed: 2026-02-23
> Reviewer: Senior Rust Engineer (Phase 2 automated review)
> Crate: `uptrakit-shared-db` (`crates/shared/db/`)
> Scope: Entity models, schema design, soft-delete conventions, type safety, test coverage,
> dependency coupling, and how this crate's entities are consumed by `web-api` query helpers.

---

## Summary

`uptrakit-shared-db` is the central SeaORM entity library shared across the controller,
web-api, and related crates. The foundation is solid: UUID v7 primary keys, a compile-time
`TenantScoped` trait, typed enums for state columns, and well-declared referential integrity.
These are meaningful architectural wins that prevent entire classes of runtime errors.

Several Medium-severity schema decisions (soft-delete naming inconsistency, dual output storage, nullable version, missing indexes, duplicate indexes) require remediation before the system scales. One architectural concern — `uptrakit-crypto` unconditionally depending on `sea-orm` — is inherited through this crate's dependency on `uptrakit-crypto`.

---

## Architecture

### Strengths

- **Clean re-export surface.** `lib.rs` re-exports only the types that downstream crates
  genuinely need from `uptrakit-shared-types`. The `crypto` submodule is a thin re-export
  shim (`pub use uptrakit_crypto::*`) preserving backward compatibility without leaking
  implementation detail.

- **Dependency graph is intentional.** The crate depends on `sea-orm`, `uptrakit-crypto`,
  `uptrakit-shared-types` (with `features = ["sea-orm"]`), `time`, `serde`, and `zeroize`.
  Every dependency is workspace-pinned. No transitive version surprises.

- **`TenantScoped` trait drives architectural safety.** Defined in
  `src/entity/tenant_scoped.rs`, the trait provides a single `tenant_id_column()` associated
  function. `TenantDb` in `web-api` can implement a generic `find_by_id_scoped` helper that
  makes it structurally impossible to fetch rows from a tenant-scoped table without applying
  the tenant filter, as long as the path uses the typed helper. Ten entities implement the
  trait (hosts, services, oidc_providers, scheduled_tasks, software_items, plugin_configs,
  mqtt_clients, settings, settings_versions, user_roles).

- **Relation declarations are complete.** Every `Relation` enum variant has a matching
  `impl Related<...>` block. No dangling relation stubs.

### Issues

---

## Security & Safety

### Strengths

- **`EncryptedString` used for all secrets stored in the DB.** `oidc_providers.client_secret`
  (`entity/oidc_provider.rs:80`) and SSH private keys in the agent-ssh local DB use
  `crate::crypto::EncryptedString` / `uptrakit_crypto::EncryptedString`. AES-256-GCM
  encryption is applied transparently via the `ValueType`/`TryGetable` impls.

- **Zero `unsafe` in the crate.** Confirmed across all entity files and the `crypto.rs`
  re-export module.

- **`Zeroizing` used on sensitive in-memory values.** The dependency on `zeroize` in
  `Cargo.toml` is purposeful: `uptrakit-crypto` uses `Zeroizing<>` for the master key and
  decrypted key material; this crate inherits that guarantee.

- **Sentinel value on serialization error for `RoleMapping`.** Rather than silently writing
  `{}`, the `From<RoleMapping> for sea_orm::Value` impl writes
  `{"__serialization_error": true}` and logs at `error` level
  (`entity/oidc_provider.rs:41-52`). This makes data corruption detectable in logs and
  distinguishable from a legitimate empty mapping.

### Issues

No open issues in this section.

---

## Code Quality

### Strengths

- **Consistent `time::OffsetDateTime` for all timestamp columns.** Every entity in this crate
  uses `OffsetDateTime` rather than raw integers or `chrono`. This makes timestamp arithmetic
  correct, timezone-aware, and uniform across the entire schema — a contrast to the
  agent-ssh local DB (see Database section).

- **Typed enums for state columns.** `UpdateStatus` (`entity/update_history.rs:4-15`),
  `OutputStreamType` (re-exported from `uptrakit-shared-types`), `MqttClientConnectionStatus`,
  `SoftwareDiscoveryState`, and `SessionTokenType` are all modelled as `DeriveActiveEnum`
  types. Impossible state values are rejected at the ORM boundary.

- **`RoleMapping` newtype with complete ORM integration.** The `ValueType`, `TryGetable`,
  and `From<RoleMapping>` implementations in `entity/oidc_provider.rs:10-53` are thorough.
  The implementation correctly handles the `sea_orm::Value::Json` variant, propagates
  deserialization errors through `TryGetError::DbErr`, and includes a documented rationale
  for the serialization sentinel.

- **`plugin_config.config` stored as `serde_json::Value`.** Using `Json` column type with
  `serde_json::Value` is the correct approach for a polymorphic plugin configuration blob
  where the schema depends on `plugin_type`. Avoids premature normalization.

- **`ActiveModelBehavior` implemented on every entity.** All entities implement the default
  `ActiveModelBehavior`, which is the correct baseline for entities that do not need
  before-save hooks.

### Issues

#### 2026-02-24 Review

#### Issues

**[SEVERITY: Low]** `crates/ui/web-api/src/queries/plugin_configs.rs:155-158` — Second instance of string-based unique violation detection duplicated from autodiscovery.rs

Two independent copies of the same fragile detection logic. Should be consolidated into `uptrakit-shared-db` using backend-specific error codes.

**[SEVERITY: Medium]** `crates/shared/db/src/entity/update_history.rs:28` — Dual output
storage: `output` column (inline Text) and the `update_output_lines` child table.

The `update_history` entity has an `output: String` column (column_type `Text`) at line 27-28
and a sibling `update_output_lines` entity (`entity/update_output_line.rs`) with an FK back
to `update_history`. The consuming query in `web-api/src/queries/update_history.rs` checks
`if record.output.is_empty()` and conditionally loads child rows, meaning output can exist
in either place. There is no DB constraint or migration guard ensuring only one path is
populated. A partially-completed migration (new records use `update_output_lines`, old records
use `output`) leaves both paths active indefinitely. The canonical storage path should be
documented as a code comment on the `output` field, and a migration should either backfill
one path or add a CHECK constraint preventing both from being non-empty simultaneously.

**[SEVERITY: Medium]** `crates/shared/db/src/entity/available_version.rs:10` — `version`
column is `Option<String>` with no semantic default.

```
// entity/available_version.rs:10
pub version: Option<String>,
```

The schema has a CHECK constraint in the migration that allows a row with `release_date` but
no `version`. This means the update pipeline must handle a record representing "we know
a release happened on date X but do not know what version it is." This is semantically
ambiguous: the pipeline cannot answer "is version Y available?" for a NULL-version row.
The `version` field should either be `NOT NULL` (most release tracking scenarios know the
version), or the nullable behaviour must be explicitly documented with a code comment
explaining which code paths produce and consume NULL-version rows.

---

## Tests

### Strengths

- **`RoleMapping` has thorough round-trip tests.** Six tests in `entity/oidc_provider.rs:127-215`
  cover: normal round-trip, empty map, `serde_json` round-trip, special characters (spaces,
  Unicode, backslash), default-is-empty, non-JSON value rejection, and wrong JSON shape
  rejection. This is the most comprehensively tested custom ORM type in the codebase.

- **In-memory SQLite available via `sea-orm` mock feature.** `Cargo.toml:25` gates the mock
  feature to `dev-dependencies`, which is the correct pattern. Query integration tests in
  `web-api` use in-memory SQLite and exercise the entity models with full ORM semantics.

### Issues

**[SEVERITY: Medium]** No unit tests for any entity other than `oidc_provider.rs`.

The `RoleMapping` tests in `oidc_provider.rs` are exemplary, but all other entities have zero
inline `#[cfg(test)]` coverage. Entities with custom ORM type impls are the highest priority:
`EncryptedString` round-trip tests belong here (confirming encrypt-then-retrieve produces the
original plaintext through the SeaORM mock driver), `UpdateStatus` enum serialization, and
`OutputStreamType` round-trip. Entities that carry business invariants (e.g., that `version`
being `None` in `available_version` is handled consistently) would also benefit from
test-level documentation.

**[SEVERITY: Low]** `update_output_line.rs` and `update_history.rs` have no tests verifying
the dual-storage fallback logic.

The conditional `output` vs `update_output_lines` branching in `queries/update_history.rs`
is the kind of logic error-prone enough to warrant a dedicated integration test that seeds
both kinds of records and asserts the correct output is returned in each case.

---

## High Availability

### Strengths

- **FK `ON DELETE` actions declared for all relations.** Cascades and restrictions are
  explicitly declared, preventing orphaned rows at the DB level without application-layer
  cleanup.

- **Append-only tables for audit data.** `update_history` and `controller_events` have no
  `deactivated_at` or soft-delete column, correctly modelling them as immutable audit logs.

---

## Database

### Strengths

- **UUID v7 primary keys throughout.** All entities in this crate use
  `#[sea_orm(primary_key, auto_increment = false)]` with `Uuid` type. UUID v7 is
  time-ordered, which eliminates B-tree hot-spot contention on insert-heavy tables and makes
  the PK usable as a coarse-grained cursor for range queries. Observed consistently across
  `hosts`, `services`, `software_items`, `plugin_configs`, `update_history`, `oidc_provider`,
  `mqtt_lease`, `api_token`, `available_version`, and all other entities.

- **Partial (filtered) unique indexes for soft-delete.** The migration creates
  `uq_plugin_configs_active_name WHERE deactivated_at IS NULL` and
  `uq_software_items_active_name WHERE deactivated_at IS NULL`. This correctly enforces name
  uniqueness only among active records without blocking reuse of names after deactivation,
  and without requiring application-layer uniqueness checks.

- **Referential integrity with explicit ON DELETE actions.** Every FK in the migration
  carries an explicit `ON DELETE` action (CASCADE or RESTRICT/NO ACTION as appropriate).
  No "accidental" implicit NO ACTION defaults.

- **CHECK constraint on `sessions`.** `auth_method != 'oidc' OR oidc_provider_id IS NOT NULL`
  is enforced at the DB level. The ORM model cannot accidentally create an OIDC session
  without a plugin ID.

- **All ephemeral tables have `expires_at` indexes.** Pending OIDC flows, device flows,
  pending account links, and pending token exchanges all have indexes on `expires_at`,
  enabling efficient TTL cleanup.

#### 2026-02-24 Review

#### Strengths

- **Migration `down()` drops all 28 tables in correct reverse FK dependency order.** `m20260209_000001_initial.rs:1999-2050`.
- **Unique index `uq_host_software_items_active` prevents duplicate assignments.** `m20260209_000001_initial.rs:1093-1101`.

#### Issues

**[SEVERITY: Medium]** `m20260209_000001_initial.rs:1093-1101` — Missing index on `host_software_items(plugin_config_id, package_identifier)` for autodiscovery lookup

Phase 2 of `process_one_discovery` queries by this combination. Only `plugin_config_id` is indexed alone.

**[SEVERITY: Medium]** `m20260209_000001_initial.rs:862-893` — Missing index on `service_hosts(host_id)` for host-to-agent lookups

Primary key is `(service_id, host_id)`. Queries filtering by `host_id` require a full scan.

**[SEVERITY: Low]** `m20260209_000001_initial.rs:1399-1439` — Missing index on `update_history(host_id, software_item_id, status)` for pending-update lookups

Frequent check requires index intersections without a composite index.

**[SEVERITY: Low]** `m20260209_000001_initial.rs:309` — No partial unique index on `oidc_providers` for active names

Unlike `plugin_configs` and `software_items`, OIDC provider slugs cannot be reused after soft-deletion.

### Issues

**[SEVERITY: Medium]** `crates/core/agent-ssh/src/db/entity/ssh_host.rs:71-72` — Integer
epoch timestamps instead of typed TIMESTAMP columns in agent-ssh local DB.

```
// crates/core/agent-ssh/src/db/entity/ssh_host.rs:71-72
pub created_at: i64,
pub updated_at: i64,
```

The `ssh_hosts` table in the agent-ssh local SQLite DB uses `INTEGER` columns for timestamps.
Every other entity in the codebase uses `OffsetDateTime` (mapped to `TIMESTAMPTZ` in
PostgreSQL or `TEXT` in SQLite via SeaORM). This breaks tooling assumptions: log formatters,
DB inspection tools, and any future migration helper that iterates entity columns will silently
skip these timestamps or display raw Unix epoch integers. The custom `now_unix_timestamp()`
helper in `host_ops.rs:197-202` also uses `SystemTime` rather than `time::OffsetDateTime::now_utc()`,
creating a different clock source from the rest of the codebase and silently returning `0`
on error. These columns should be migrated to `OffsetDateTime`.

**[SEVERITY: Low]** `crates/shared/db/src/entity/api_token.rs` — No `expires_at` column.

```
// entity/api_token.rs — current fields
pub id: Uuid,
pub user_id: Uuid,
pub name: String,
pub token_hash: String,    // unique
pub created_at: OffsetDateTime,
pub last_used_at: Option<OffsetDateTime>,
pub revoked_at: Option<OffsetDateTime>,
```

API tokens are currently valid indefinitely until explicitly revoked. There is no mechanism
for time-bounded tokens (e.g., "expire this token after 90 days"). For infrastructure access
tokens this is a meaningful gap: a forgotten or leaked token remains valid until it is
discovered and manually revoked. Adding `expires_at: Option<OffsetDateTime>` and an index on
`(user_id, expires_at)` would enable both user-defined expiration and automated cleanup of
long-stale tokens.

**[SEVERITY: Low]** `crates/shared/db/src/entity/controller_event.rs:8` — `id` is `i64`
(sequential auto-increment), not UUID. This is intentional for event-ordering semantics but
is undocumented.

```
// entity/controller_event.rs:7-8
#[sea_orm(primary_key)]
pub id: i64,
```

The `auto_increment = false` override is absent, meaning SeaORM uses the default auto-increment
behaviour. An `i64` sequential PK is the correct choice for an event log where total ordering
must be preserved and range scans (`WHERE id > last_seen`) are the primary access pattern.
However, this is the only entity in the crate that diverges from UUID v7 PKs and there is no
comment explaining the rationale. A code comment stating "Sequential i64 is intentional:
controller_events is an append-only event log; sequential IDs preserve insertion order for
the event poller cursor" would prevent future reviewers from treating this as a mistake.

### Index Audit

**Duplicate indexes — Low severity waste:**
- `tenants.slug`: has both `string_uniq()` (which creates a unique index) AND an explicit
  `Index::create()` in the migration. Two indexes on the same column double write overhead
  for every INSERT/UPDATE.
- `users.email`: same pattern as `tenants.slug`.

**Missing indexes — Medium severity:**

- `update_history.created_at`: The list query in `queries/update_history.rs` uses
  `ORDER BY created_at DESC` with pagination. Without an index on `created_at`, every
  pagination request is a full table scan + sort. As update_history grows this becomes the
  dominant query cost.

- `host_software_items.software_item_id`: The composite PK starts with `host_id`, so
  `software_item_id` is not the leading column. Queries that look up "which hosts have
  this software item" (used in `load_item_hosts`) must do a full scan of the junction table.
  A standalone index on `software_item_id` fixes this.

- `mqtt_leases.tenant_id`: Lease reconciliation queries filter by `tenant_id`. Without an
  index, the reconciliation loop scans all leases across all tenants.

- `sessions(user_id, expires_at)`: "Find active sessions for user" is a common auth query.
  Without a composite index, it scans all sessions for the user and then filters by
  `expires_at`, or scans all non-expired sessions and then filters by `user_id`.

---

## Coding Standards

### Strengths

- **`edition = "2024"` and workspace-pinned dependencies.** `Cargo.toml` is fully
  workspace-aligned with no inline version overrides.

- **Consistent `time::OffsetDateTime` across all entities.** No `chrono` usage; the crate
  consistently uses `time = { workspace = true }`.

- **`publish = false`.** The crate correctly marks itself as an internal library not intended
  for crates.io.

- **Typed enum serialization follows the `as_str()` / `DeriveActiveEnum` / `FromStr` pattern**
  established in `uptrakit-shared-types`. `UpdateStatus` in `update_history.rs` is a clean
  example.

- **No `#[allow(clippy::...)]` or `#[allow(dead_code)]` annotations anywhere in the crate.**
  Consistent with the workspace-wide standard.

### Issues

**[SEVERITY: Low]** `crates/ui/web-api/src/queries/autodiscovery.rs:673-676` —
`is_unique_violation()` uses string matching on error messages.

```rust
fn is_unique_violation(e: &sea_orm::DbErr) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("unique") || msg.contains("duplicate")
}
```

This helper is in `web-api`, not in this crate, but it is called when inserting rows into
tables defined by this crate (`autodiscovery_ignore`). The implementation matches against
lowercased error message strings. This is backend-specific and brittle: SQLite uses "UNIQUE
constraint failed", PostgreSQL uses "duplicate key value violates unique constraint",
MySQL uses "Duplicate entry". If the DB backend changes or the error message format changes
in a SeaORM release, the detection silently fails (returns `false` when it should return
`true`), causing idempotent insert operations to surface as errors. The correct approach is
to match on `sea_orm::DbErr::RecordNotInserted` or inspect the underlying `sqlx::Error`
for the backend-specific error code (PostgreSQL error code `23505`, SQLite extended code
`SQLITE_CONSTRAINT_UNIQUE`). This helper should be moved into `uptrakit-shared-db` as
a utility function so it is co-located with the entity definitions.

---

## Extensibility

### Strengths

- **All entities are `Clone + Debug`.** Every entity model derives `Clone` and `Debug`,
  making them safe to use in `Arc<>`, log with `tracing`, and pass across async boundaries.

- **`prelude.rs` re-exports all entities.** `entity/prelude.rs` re-exports all `Entity` and
  `Model` types. Downstream crates can import everything from a single path.

- **`TenantScoped` is open for extension.** The trait is defined in this crate and
  implemented for entities here; any new tenant-scoped entity just requires an additional
  `impl TenantScoped for NewEntity::Entity` block.

#### 2026-02-24 Review

#### Issues

**[SEVERITY: Low]** `crates/shared/db/src/entity/plugin_config.rs:14` — `plugin_config.plugin_type` stored as `String` in DB

Provides forward compatibility at storage level but typos in manual edits produce runtime errors rather than constraint violations.

### Issues

**[SEVERITY: Medium]** `crates/shared/db/src/entity/plugin_config.rs:11` — `plugin_type`
stored as unvalidated `String` rather than the `PluginType` enum from `uptrakit-shared-types`.

```
// entity/plugin_config.rs:11
pub plugin_type: String,
```

The `PluginType` enum in `uptrakit-shared-types` is the canonical typed representation of
plugin types, with `FromStr`, `Display`, and `as_str()` implementations. The entity uses
raw `String`, delegating the string-to-enum conversion to every consumer. This allows
invalid plugin type strings to be stored in the DB (e.g., from a future migration gap or
direct DB edit) and forces every query helper to call `PluginType::from_str()` individually,
with inconsistent error handling. Using a `DeriveActiveEnum` on `PluginType` (or at minimum
a newtype wrapper that validates on deserialize) would push the validation to the ORM boundary.
Note that `uptrakit-shared-types` already gates its SeaORM derives behind a `sea-orm` feature
flag, so adding `DeriveActiveEnum` there is architecturally correct.

---

*End of review — `crates/shared/db/`*
