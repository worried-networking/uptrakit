# Code Review: uptrakit-shared-db

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
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
