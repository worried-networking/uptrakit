# Plugin Boundary Hardening — Completion

**Date:** 2026-05-04
**Status:** Spec
**Predecessor:** [`2026-05-03-plugin-boundary-hardening.md`](./2026-05-03-plugin-boundary-hardening.md)

## Background

The predecessor spec landed Waves 1–7 successfully but a post-merge audit found
two unfinished items and one missing CI enforcement:

1. **Wave 4 incomplete.** Four of the five Proxmox entities moved to
   `crates/plugins/infrastructure/proxmox/src/entity/`
   (`proxmox_protection_audit`, `proxmox_protection_default`,
   `proxmox_protection_item_override`, `proxmox_backup_target_cache`). The fifth
   entity, `proxmox_host_mapping`, is still hosted by `shared-db`:
   - `crates/shared/db/src/entity/proxmox_host_mapping.rs`
   - `pub mod proxmox_host_mapping;` in `crates/shared/db/src/entity/mod.rs:62`
   - Re-export in `crates/shared/db/src/entity/prelude.rs:56-58`
   - `impl TenantScoped` block in `crates/shared/db/src/entity/tenant_scoped.rs:137-141`

   The Proxmox plugin still imports its own entity from `shared-db` at:
   `surfaces.rs:643,924,978,1970-1971`, `reset.rs:23`, `discovery.rs:260`,
   `matching.rs:15`, `protection_store.rs:22`. This violates Wave 4's acceptance
   rule: "All internal `use uptrakit_shared_db::entity::proxmox_*` imports in
   the proxmox plugin change to `use crate::entity::proxmox_*`."

2. **Plugin knowledge in non-plugin crate.**
   `crates/core/controller-runtime/src/db_migrate/tables.rs` hardcodes
   `ProxmoxHostMapping` and `"proxmox_host_mappings"` in four places — the
   `COPY_ORDER` const (line 67) plus `copy!` (137), `clean!` (154), and
   `verify!` (274) macro invocations. This is the data-migration machinery
   used by `uptrakit-controller db-migrate` to migrate rows between SQLite and
   Postgres deployments. Plugin-specific table knowledge in this non-plugin
   crate is the same boundary violation that Wave 6 fixed for `reset_data.rs`,
   but `tables.rs` was missed.

3. **Audit enforcement gap.** The predecessor's Wave 6 audit grep used a
   `| grep "use "` filter, which silently skipped the macro-style references
   (`copy!(ProxmoxHostMapping, ...)`) in `tables.rs`. The Python checker
   (`ci/check_plugin_semantic_boundary.py`) also has no rule that detects
   plugin-owned entities re-exported from `shared-db`'s `entity/prelude.rs` or
   declared in `entity/mod.rs`. `RULE_CONCRETE_PLUGIN_IMPORT` only triggers on
   imports whose path begins with a plugin crate name (`uptrakit_plugin_*`),
   so a leak that lives inside `shared-db` itself is invisible to it.

A discovery during the design phase: `proxmox_protection_audit.mapping_id`
has a `SetNull` foreign key to `proxmox_host_mappings.id`. This is the only
inter-plugin-table FK in the codebase. The order of entries inside a plugin's
`db_migrate_tables` Vec therefore matters — `host_mapping` must precede
`protection_audit`.

## Goals

1. Zero `proxmox_*` entity references in `crates/shared/db/`.
2. Zero plugin-named identifiers in
   `crates/core/controller-runtime/src/db_migrate/tables.rs`.
3. Plugins register their own tables for the `db-migrate` SQLite ↔ Postgres
   data migration via a new `PluginDescriptor::db_migrate_tables` field that
   mirrors the `reset_tenant_data` pattern.
4. `controller-runtime/db_migrate/tables.rs` shrinks to a thin orchestrator
   ("core first, plugins last") with all per-table logic hosted by
   `shared-db`.
5. CI catches future regressions:
   - Plugin-owned entity files reintroduced into `shared-db/entity/`.
   - Plugin-owned modules re-exported via `shared-db/entity/prelude.rs` or
     declared in `entity/mod.rs`.
   - Schema drift — a live DB table missing from the migration coverage.
6. Single Python checker; the legacy shell checker is deleted.

## Non-Goals

- Adding `impl TenantScoped` for any plugin entity other than
  `proxmox_host_mapping`. The other four Proxmox entities currently do not
  implement `TenantScoped`; adding them is a separate concern.
- Schema or FK changes to any existing table.
- Modifying agent-side Proxmox tables (`proxmox_host_state`,
  `proxmox_pending_matches`) in `crates/core/agent-ssh/` or
  `crates/shared/db/src/migration/m20260331_000001_ssh_agent_tables.rs`.
  These belong to the SSH agent and are unrelated to the controller-side
  Proxmox plugin.
- Audit-table deletion changes — `proxmox_protection_audit` continues to be
  permanent record.

## Wave structure

Four waves, ordered **A → B → D → C**:

| Wave | Purpose                                                                                                                 | Atomic? |
| ---- | ----------------------------------------------------------------------------------------------------------------------- | ------- |
| A    | Add type-system primitives in `plugin-infrastructure-core` (additive only)                                              | yes     |
| B    | Move `proxmox_host_mapping` entity + flip `db-migrate` dispatch through registry                                        | yes     |
| D    | Add `RULE_PLUGIN_ENTITY_IN_SHARED_DB`, replace `COPY_ORDER` length assertion with structural test, delete shell checker | yes     |
| C    | Move core table list into `shared-db::migrate_core_tables`; reduce `tables.rs` to a thin orchestrator                   | yes     |

Order rationale:

- **A first.** Purely additive: new types and fields with no plugin populating
  them. CI green throughout.
- **B before D.** The new audit rules introduced in D would fail against the
  current state (the leaks in `shared-db` and `tables.rs`); they can only land
  after B has removed those leaks. B is also the wave where the entity move
  and the dispatch flip must happen together — once `prelude.rs` no longer
  re-exports `ProxmoxHostMapping`, every macro reference in `tables.rs` stops
  compiling, so they must be removed in the same change.
- **D before C.** D adds the structural completeness test that catches schema
  drift; landing it before C's restructure means the test guards the
  restructure itself.
- **C last.** C is an independent restructure (move core list into
  `shared-db`); its only practical effect is to shrink `tables.rs`. Lands
  last so it does not churn during the higher-priority work.

---

## Wave A — Type-system primitives

**Goal:** Add `PluginTableDescriptor`, `DbMigrateTablesFn`,
`db_migrate_tables` descriptor field, the shared `TableMigrateError`
enum, and registry helpers (`copy_plugin_tables` / `clean_plugin_tables` /
`verify_plugin_tables`). No plugin populates the new field yet. Purely
additive.

**Why typed descriptors instead of a `sqlite_master`-driven schema-less
copy?** A schema-less approach (read every table via `sqlite_master`,
`INSERT … SELECT *`) is simpler but only works for SQLite ↔ SQLite. The
real use case is SQLite ↔ Postgres: metadata catalogs differ
(`sqlite_master` vs `information_schema.tables`), `PRAGMA foreign_keys`
has no Postgres analogue, and column-type coercions (booleans, JSON,
timestamps with offsets) need SeaORM's typed model layer to round-trip
correctly. The typed-entity descriptor approach reuses the same
`IntoActiveModel` path that already works in `migrate_table<E>` today
and keeps cross-backend semantics correct without per-table special-casing
in the orchestrator.

**Why does `TableMigrateError` live in `shared-db` even though Wave A
adds no `shared-db` helpers using it yet?** Cycle direction:
`plugin-infrastructure-core` already takes `uptrakit-shared-db` as an
optional dep. Reversing the edge for shared types would create a cycle.
`shared-db` is the only crate downstream-of-nothing-relevant that all
migration consumers (core helpers in Wave C, registry helpers from Wave
A, orchestrator in Waves B + C) can import without rearranging the
dependency graph. The enum has only one external consumer between Waves
A and C (the registry); Wave C adds the second (core helpers). The
intermediate state is brief and the location is justified by graph
shape, not by usage count at any single moment.

### `shared-db` — new `db-migrate` feature + shared error type

Add a `db-migrate` feature flag in `crates/shared/db/Cargo.toml`:

```toml
[features]
db-migrate = []
```

Wave A introduces only the new error enum under this flag; Wave C adds the
core helpers later. Hosting the enum in `shared-db` (rather than
`plugin-infrastructure-core`) avoids a dependency cycle: `shared-db` is a
parallel/upstream crate to `plugin-infrastructure-core`
(`plugin-infrastructure-core` already takes `uptrakit-shared-db` as an
optional dep), so reversing the direction is impossible. `shared-db` is
also the natural home for migration-related types.

New file `crates/shared/db/src/migrate_core_tables.rs`:

```rust
//! Shared types and (in Wave C) per-table operations for the `db-migrate`
//! subcommand.

#![cfg(feature = "db-migrate")]

/// Errors produced by per-table copy / clean / verify operations.
///
/// Surfaces the table name in both variants so the orchestrator in
/// `controller-runtime/db_migrate/tables.rs` can convert into the
/// existing `DbMigrateError::TableOp` and `DbMigrateError::Mismatch`
/// variants via a single `.context_to()?` boundary, without losing
/// context.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum TableMigrateError {
    /// A SeaORM driver error occurred for `table`.
    #[error("table `{table}` operation failed: {err}")]
    Db {
        table: &'static str,
        #[source]
        err: sea_orm::DbErr,
    },
    /// `verify` found different row counts for `table`.
    #[error("row count mismatch for table `{table}`: source={src}, target={dst}")]
    Mismatch {
        table: &'static str,
        src: u64,
        dst: u64,
    },
}

/// Module-local `Result` alias following the project's `Report<E>`
/// convention (see `docs/development/error-handling.md`).
pub type Result<T> = std::result::Result<T, rootcause::Report<TableMigrateError>>;
```

Wave C extends this module with `copy`, `clean`, `verify` functions plus
the `CORE_COPY_ORDER` const. Wave A only adds the enum.

`plugin-infrastructure-core/Cargo.toml` already declares
`uptrakit-shared-db = { workspace = true, optional = true }`. Pull it
in under the `migrations` feature (matching how `sea-orm` is gated):

```toml
[features]
migrations = ["sea-orm", "uptrakit-shared-db/db-migrate", "uptrakit-tenant-db"]
```

(Replace any existing `migrations` feature definition with the union.)

### `plugin-infrastructure-core/src/descriptor.rs`

Add under `#[cfg(feature = "migrations")]`, alongside the existing
`ResetTenantDataFn` definitions:

````rust
/// Per-table copy/clean/verify operations for the `db-migrate` subcommand.
///
/// Constructed via [`PluginTableDescriptor::for_entity`].
///
/// Type erasure: the closures monomorphise the generic
/// `copy_one` / `clean_one` / `verify_one` helpers (in
/// `plugin-infrastructure-core::db_migrate`) per entity `E`. Each
/// `for_entity::<E>(...)` call produces a descriptor with the same shape
/// regardless of `E`.
#[cfg(feature = "migrations")]
pub struct PluginTableDescriptor {
    /// Table name as it appears in the database (matches
    /// `#[sea_orm(table_name = "...")]` on the entity).
    pub name: &'static str,

    /// Bulk-copy rows from `src` to `dst` for this table. Returns row count.
    pub copy_batch: for<'a> fn(
        src: &'a sea_orm::DatabaseConnection,
        dst: &'a sea_orm::DatabaseConnection,
        batch_size: u64,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<u64, sea_orm::DbErr>> + Send + 'a>,
    >,

    /// Delete every row in this table on `dst`.
    pub clean: for<'a> fn(
        dst: &'a sea_orm::DatabaseConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), sea_orm::DbErr>> + Send + 'a>,
    >,

    /// Count rows on both `src` and `dst`. Returns `(src_count, dst_count)`.
    /// The caller (registry helper) compares the pair and constructs a
    /// structured `TableMigrateError::Mismatch` with the table name on
    /// disagreement — the descriptor itself does not carry the table name
    /// into the closure body.
    pub verify: for<'a> fn(
        src: &'a sea_orm::DatabaseConnection,
        dst: &'a sea_orm::DatabaseConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(u64, u64), sea_orm::DbErr>> + Send + 'a>,
    >,
}

#[cfg(feature = "migrations")]
impl PluginTableDescriptor {
    /// Build a descriptor for a SeaORM entity.
    ///
    /// Bounds match the existing `migrate_table<E>` helper in
    /// `controller-runtime/db_migrate/tables.rs`.
    pub fn for_entity<E>(name: &'static str) -> Self
    where
        E: sea_orm::EntityTrait + 'static,
        E::Model: sea_orm::IntoActiveModel<E::ActiveModel> + Send + Sync + 'static,
        E::ActiveModel: sea_orm::ActiveModelTrait<Entity = E>
            + sea_orm::ActiveModelBehavior
            + Send
            + 'static,
    {
        Self {
            name,
            copy_batch: |src, dst, batch| {
                Box::pin(crate::db_migrate::copy_one::<E>(src, dst, batch))
            },
            clean: |dst| Box::pin(crate::db_migrate::clean_one::<E>(dst)),
            verify: |src, dst| Box::pin(crate::db_migrate::verify_one::<E>(src, dst)),
        }
    }
}

/// Function returning a plugin's tables in FK-safe order.
///
/// Order rule: parent tables (referenced by FKs) must come before child
/// tables. The registry iterates **forward** for copy and verify, and
/// **reverse** for clean. Getting the order wrong is silent: copy will
/// fail with a FK violation, but clean might succeed by accident depending
/// on FK action.
///
/// Example — Proxmox plugin, where `proxmox_protection_audit.mapping_id`
/// references `proxmox_host_mappings.id`:
///
/// ```ignore
/// fn proxmox_db_migrate_tables() -> Vec<PluginTableDescriptor> {
///     vec![
///         PluginTableDescriptor::for_entity::<proxmox_host_mapping::Entity>(
///             "proxmox_host_mappings",
///         ),
///         PluginTableDescriptor::for_entity::<proxmox_protection_default::Entity>(
///             "proxmox_protection_defaults",
///         ),
///         // ... other independents ...
///         PluginTableDescriptor::for_entity::<proxmox_protection_audit::Entity>(
///             "proxmox_protection_audit",
///         ),
///     ]
/// }
/// ```
#[cfg(feature = "migrations")]
pub type DbMigrateTablesFn = fn() -> Vec<PluginTableDescriptor>;

/// Placeholder used when `migrations` feature is not active.
#[cfg(not(feature = "migrations"))]
pub type DbMigrateTablesFn = fn();
````

Add the field to `PluginDescriptor`, alongside `reset_tenant_data`:

```rust
pub struct PluginDescriptor {
    // ── existing fields unchanged ──

    /// Plugin-owned tables registered for the `db-migrate` subcommand.
    /// `None` for plugins with no own tables.
    ///
    /// Real type only meaningful under `migrations` feature; outside it,
    /// `DbMigrateTablesFn` is a `fn()` placeholder.
    pub db_migrate_tables: Option<DbMigrateTablesFn>,
}
```

The `Option<DbMigrateTablesFn>` field is unconditionally present (no `#[cfg]`
on the field). Same dual-alias pattern that already supports
`reset_tenant_data` and `migrations`.

### `plugin-infrastructure-core/src/db_migrate.rs` (new module)

New file. Hosts the generic helpers under `#[cfg(feature = "migrations")]`.
Bodies copied verbatim from
`crates/core/controller-runtime/src/db_migrate/tables.rs::migrate_table` /
`clean_table` / `verify_table` (lines 287–397), with two adjustments:

- Error type changed from `Result<u64, Report<DbMigrateError>>` to
  `Result<u64, sea_orm::DbErr>` (and equivalent for `clean_one`).
- `eprintln!` progress messages preserved.

```rust
//! Generic per-table operations for the `db-migrate` subcommand.
//!
//! Plugins do not call these directly — they construct descriptors via
//! [`PluginTableDescriptor::for_entity`], which captures `E` and produces
//! type-erased fn pointers wrapping these helpers.

#![cfg(feature = "migrations")]

use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, DatabaseConnection, DbErr, EntityTrait,
    IntoActiveModel, PaginatorTrait,
};

pub(crate) async fn copy_one<E>(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64, DbErr>
where
    E: EntityTrait + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Send + Sync + 'static,
    E::ActiveModel: ActiveModelTrait<Entity = E> + ActiveModelBehavior + Send + 'static,
{
    // Paginated batch copy — body adapted from the existing
    // `migrate_table<E>` helper in
    // `controller-runtime/db_migrate/tables.rs` (lines 287–349). Error
    // type is `DbErr` directly; the registry helper attaches the table
    // name via `TableMigrateError::Db { table, err }`.
    // Caller (registry helper / shared-db core helper) wraps this call
    // with `eprintln!("{name}: copying...")` / `"{name}: {copied} rows"`
    // before/after — `copy_one` has no `name` in scope and intentionally
    // does not emit progress lines itself.
    let mut copied = 0u64;
    let mut offset = 0u64;
    loop {
        let batch = E::find()
            .offset(offset)
            .limit(batch_size)
            .all(src)
            .await?;
        if batch.is_empty() {
            break;
        }
        let n = batch.len() as u64;
        let active: Vec<_> = batch.into_iter().map(IntoActiveModel::into_active_model).collect();
        E::insert_many(active).exec(dst).await?;
        copied += n;
        offset += n;
    }
    Ok(copied)
}

pub(crate) async fn clean_one<E: EntityTrait>(
    dst: &DatabaseConnection,
) -> Result<(), DbErr> {
    E::delete_many().exec(dst).await.map(|_| ())
}

pub(crate) async fn verify_one<E: EntityTrait>(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
) -> Result<(u64, u64), DbErr> {
    let src_count = E::find().count(src).await?;
    let dst_count = E::find().count(dst).await?;
    Ok((src_count, dst_count))
}
```

`db_migrate` module is registered in `lib.rs` under `#[cfg(feature = "migrations")]`
and is `pub(crate)` — only `PluginTableDescriptor::for_entity` references it.

### `plugin-infrastructure-core/src/macros.rs`

Add `db_migrate_tables` to `declare_plugin!` as an optional parameter,
following the pattern already used for `migrations` and `reset_tenant_data`
(lines 60–61, 268–269 of `macros.rs`):

```rust
// In the macro arm that accepts the optional parameter list:
$(, db_migrate_tables: $db_migrate_fn:expr )?

// In the struct literal emitted by the macro:
db_migrate_tables: $crate::__option_expr!( $( $db_migrate_fn )? ),
```

The field must always appear in the literal (Rust does not allow `#[cfg]`
attributes on struct-literal fields). `__option_expr!` collapses the optional
match to `Some(expr)` when present and `None` when absent.

### `plugin-infrastructure-registry/src/lib.rs`

Add three async helpers under `#[cfg(feature = "migrations")]`, following
the existing `reset_plugin_tenant_data` pattern (lines 97–117). All three
return `Result<_, Report<TableMigrateError>>` (the project's standard
`Report<E>` shape; see `docs/development/error-handling.md`). The
orchestrator converts to `DbMigrateError` via `.context_to()?` enabled
by `impl_report_conversion!` (defined in Wave B):

```rust
use rootcause::prelude::*;
use uptrakit_shared_db::migrate_core_tables::TableMigrateError;

/// Copy every plugin's tables from `src` to `dst`. Returns total rows.
///
/// Iterates plugin descriptors in registration order. Within each plugin,
/// iterates tables in the order returned by `db_migrate_tables` (FK-safe:
/// parent tables first).
///
/// # Ordering note
///
/// Currently only Proxmox registers `db_migrate_tables`. If multiple
/// plugins register tables with FKs across plugin boundaries, registration
/// order would matter. We do not have such cross-plugin FKs today; if a
/// future plugin introduces one, add a `migration_order` hint to
/// `PluginDescriptor` (mirroring the same TODO already documented for
/// `reset_plugin_tenant_data`).
#[cfg(feature = "migrations")]
pub async fn copy_plugin_tables(
    src: &sea_orm::DatabaseConnection,
    dst: &sea_orm::DatabaseConnection,
    batch_size: u64,
) -> Result<u64, Report<TableMigrateError>> {
    let mut total = 0u64;
    for descriptor in all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            for table in tables_fn() {
                let copied = (table.copy_batch)(src, dst, batch_size)
                    .await
                    .map_err(|err| report!(TableMigrateError::Db { table: table.name, err }))?;
                eprintln!("  {}: {copied} rows", table.name);
                total += copied;
            }
        }
    }
    Ok(total)
}

#[cfg(feature = "migrations")]
pub async fn clean_plugin_tables(
    dst: &sea_orm::DatabaseConnection,
) -> Result<(), Report<TableMigrateError>> {
    for descriptor in all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            // Reverse for FK-safe deletion (children before parents).
            for table in tables_fn().into_iter().rev() {
                (table.clean)(dst)
                    .await
                    .map_err(|err| report!(TableMigrateError::Db { table: table.name, err }))?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "migrations")]
pub async fn verify_plugin_tables(
    src: &sea_orm::DatabaseConnection,
    dst: &sea_orm::DatabaseConnection,
) -> Result<u64, Report<TableMigrateError>> {
    let mut total = 0u64;
    for descriptor in all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            for table in tables_fn() {
                let (src_count, dst_count) = (table.verify)(src, dst)
                    .await
                    .map_err(|err| report!(TableMigrateError::Db { table: table.name, err }))?;
                if src_count != dst_count {
                    bail!(TableMigrateError::Mismatch {
                        table: table.name,
                        src: src_count,
                        dst: dst_count,
                    });
                }
                total += src_count;
            }
        }
    }
    Ok(total)
}
```

The inner generic helpers (`copy_one`, `clean_one`, `verify_one` in
`plugin-infrastructure-core::db_migrate`) keep their bare
`Result<_, sea_orm::DbErr>` shape — they have no table name in scope, so
they cannot construct `TableMigrateError`. The registry helpers attach
table-name context at the boundary via `report!()`, which is the
canonical project pattern (see `docs/development/error-handling.md`
Pattern 4).

`uptrakit-plugin-infrastructure-registry/Cargo.toml` must declare a direct
dependency on `uptrakit-shared-db` to use the `TableMigrateError` name in
`use` paths and signatures (Rust requires a direct edge — transitive deps
are not enough for name resolution). Add under `[dependencies]`, gated
behind the registry's `migrations` feature so it stays out of agent-side
builds:

```toml
[dependencies]
# ... existing entries ...
uptrakit-shared-db = { workspace = true, optional = true }

[features]
migrations = [
    # ... existing activations ...
    "dep:uptrakit-shared-db",
    "uptrakit-shared-db/db-migrate",
]
```

If `uptrakit-shared-db` is already declared optional, only extend the
`migrations` feature list. Verify the existing `Cargo.toml` shape during
implementation.

### Acceptance

- `cargo check --all-features` clean.
- `cargo check --no-default-features --features db-sqlite` clean.
- `cargo clippy --all-targets --all-features` clean.
- `cargo test --all-features` passes (no plugin uses the new field; behaviour
  is unchanged).
- `cargo doc --no-deps` smoke-check: doc comments on `DbMigrateTablesFn`,
  `PluginTableDescriptor`, and the three registry helpers render without
  warnings.

---

## Wave B — Move `proxmox_host_mapping` + flip dispatch

**Goal:** Move the entity to its owning plugin and replace `tables.rs`
plugin-named macro invocations with registry-helper calls. Atomic — both
changes ship in one commit so CI is never red.

### Move the entity

Create `crates/plugins/infrastructure/proxmox/src/entity/proxmox_host_mapping.rs`
with:

- The full body of the current
  `crates/shared/db/src/entity/proxmox_host_mapping.rs` (entity definition
  with `#[sea_orm(table_name = "proxmox_host_mappings")]`).
- The `impl TenantScoped` block, inlined at file end. Note the trait method
  name: it is `tenant_id_column()` returning `Self::Column` (matches the
  signature in `crates/shared/db/src/entity/tenant_scoped.rs`):

```rust
impl uptrakit_tenant_db::TenantScoped for Entity {
    fn tenant_id_column() -> Self::Column {
        Column::TenantId
    }
}
```

Add `pub mod proxmox_host_mapping;` to
`crates/plugins/infrastructure/proxmox/src/entity/mod.rs`.

The Proxmox plugin already depends on `uptrakit-tenant-db` (added in Wave 1
of the predecessor). No `Cargo.toml` change.

### Delete from `shared-db`

- `crates/shared/db/src/entity/proxmox_host_mapping.rs` — delete file.
- `crates/shared/db/src/entity/mod.rs:62` — delete `pub mod proxmox_host_mapping;`.
- `crates/shared/db/src/entity/prelude.rs:56-58` — delete the three-line
  `pub use super::proxmox_host_mapping::{ Entity as ProxmoxHostMapping, Model as ProxmoxHostMappingModel };`
  block.
- `crates/shared/db/src/entity/tenant_scoped.rs:6` — remove
  `proxmox_host_mapping` from the import list.
- `crates/shared/db/src/entity/tenant_scoped.rs:137-141` — delete the
  `impl TenantScoped for proxmox_host_mapping::Entity` block.

### Update plugin-internal imports

Replace `use uptrakit_shared_db::entity::proxmox_host_mapping` with
`use crate::entity::proxmox_host_mapping` at the following sites:

| File                                                            | Line(s)                                                                                      |
| --------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `crates/plugins/infrastructure/proxmox/src/surfaces.rs`         | 643 (with `host`, `plugin_config`), 924, 978, 1970–1971 (`mock_proxmox_host_mapping` helper) |
| `crates/plugins/infrastructure/proxmox/src/reset.rs`            | 23                                                                                           |
| `crates/plugins/infrastructure/proxmox/src/discovery.rs`        | 260                                                                                          |
| `crates/plugins/infrastructure/proxmox/src/matching.rs`         | 15 (with `host`)                                                                             |
| `crates/plugins/infrastructure/proxmox/src/protection_store.rs` | 22                                                                                           |

`mock_proxmox_host_mapping` at `surfaces.rs:1970-1971` has return type
`uptrakit_shared_db::entity::proxmox_host_mapping::Model` and constructs
`proxmox_host_mapping::Model { ... }` in its body. After the move, both
the return type **and** the construction expression must use
`crate::entity::proxmox_host_mapping::Model`. Search for any other
`proxmox_host_mapping::Model` references in the plugin and update all
hits, not only the use-statement on line 1971.

`protection_store.rs:22` currently reads:

```rust
use uptrakit_shared_db::entity::{plugin_config, prelude::*, proxmox_host_mapping};
```

The `prelude::*` glob currently brings `ProxmoxHostMapping` into scope and
the file uses it at line 159. After Wave B, `prelude::*` no longer exports
`ProxmoxHostMapping`. Two options:

- (preferred) Stop relying on the alias: replace `ProxmoxHostMapping::find()`
  with `crate::entity::proxmox_host_mapping::Entity::find()` (or alias locally
  within the file). Keeps the `prelude::*` glob valid (still used for other
  entities).
- (fallback) Remove the alias use entirely and add an explicit
  `use crate::entity::proxmox_host_mapping::Entity as ProxmoxHostMapping;`.

Pick whichever reads cleaner during implementation. Both compile.

### Register Proxmox `db_migrate_tables`

New file `crates/plugins/infrastructure/proxmox/src/db_migrate.rs`:

```rust
//! Plugin-owned tables registered for the `db-migrate` subcommand.

#[cfg(feature = "migrations")]
pub(crate) fn proxmox_db_migrate_tables(
) -> Vec<uptrakit_plugin_infrastructure_core::PluginTableDescriptor> {
    use uptrakit_plugin_infrastructure_core::PluginTableDescriptor;

    use crate::entity::{
        proxmox_backup_target_cache, proxmox_host_mapping,
        proxmox_protection_audit, proxmox_protection_default,
        proxmox_protection_item_override,
    };

    // FK-safe order: parents before children.
    //
    // - `proxmox_protection_audit.mapping_id` → `proxmox_host_mappings.id`
    //   (SetNull). audit must come AFTER host_mapping in copy order;
    //   reverse iteration handles clean correctly.
    // - All other proxmox tables FK only into core tables, so their
    //   relative position within this list does not matter for FK
    //   safety. Order chosen for stability: independents follow
    //   host_mapping, audit (the dependent) goes last.
    vec![
        PluginTableDescriptor::for_entity::<proxmox_host_mapping::Entity>(
            "proxmox_host_mappings",
        ),
        PluginTableDescriptor::for_entity::<proxmox_protection_default::Entity>(
            "proxmox_protection_defaults",
        ),
        PluginTableDescriptor::for_entity::<proxmox_protection_item_override::Entity>(
            "proxmox_protection_item_overrides",
        ),
        PluginTableDescriptor::for_entity::<proxmox_backup_target_cache::Entity>(
            "proxmox_backup_target_cache",
        ),
        PluginTableDescriptor::for_entity::<proxmox_protection_audit::Entity>(
            "proxmox_protection_audit",
        ),
    ]
}

#[cfg(not(feature = "migrations"))]
pub(crate) fn proxmox_db_migrate_tables() {}
```

Each table-name string MUST match the `#[sea_orm(table_name = "...")]` on
the corresponding entity. Verified table names (from grep of `entity/*.rs`):

- `proxmox_host_mappings`
- `proxmox_protection_defaults`
- `proxmox_protection_item_overrides`
- `proxmox_backup_target_cache`
- `proxmox_protection_audit`

Update the `declare_plugin!` invocation for Proxmox: append
`db_migrate_tables: db_migrate::proxmox_db_migrate_tables` (the optional
parameter takes care of `Some`/`None` wrapping). Add
`pub(crate) mod db_migrate;` to the plugin's `lib.rs`.

### Strip `tables.rs` of plugin knowledge

Edit `crates/core/controller-runtime/src/db_migrate/tables.rs`:

- Delete `"proxmox_host_mappings",` from `COPY_ORDER` (line 67).
- Delete `copy!(ProxmoxHostMapping, "proxmox_host_mappings");` (line 137).
- Delete `clean!(ProxmoxHostMapping, "proxmox_host_mappings");` (line 154).
- Delete `verify!(ProxmoxHostMapping, "proxmox_host_mappings");` (line 274).

Update `crates/core/controller-runtime/src/db_migrate/error.rs` — add
`#[non_exhaustive]` to `DbMigrateError` (project convention for extensible
public-shaped enums; see `docs/development/coding-standards.md`). No new
variants are added: the existing `TableOp { table, db_err }` and
`Mismatch { table, src, dst }` variants cover both core and plugin paths
because the registry helpers return `Report<TableMigrateError>` which
carries the table name, and the cross-boundary `ReportConversion` impl
maps each variant directly to the matching `DbMigrateError` variant.

```rust
#[non_exhaustive]
#[derive(Debug, Error)]
pub(crate) enum DbMigrateError {
    // ... existing variants unchanged ...
}
```

Add the `ReportConversion` impl alongside `DbMigrateError` so the
orchestrator can use `.context_to()?` directly (see
`docs/development/error-handling.md` Pattern 2 — `impl_report_conversion!`
with custom mapping closure):

```rust
use uptrakit_shared_db::migrate_core_tables::TableMigrateError;
use uptrakit_shared_macros::impl_report_conversion;

impl_report_conversion!(TableMigrateError => DbMigrateError, |e| match e {
    TableMigrateError::Db { table, err } => {
        DbMigrateError::TableOp { table, db_err: err }
    }
    TableMigrateError::Mismatch { table, src, dst } => {
        DbMigrateError::Mismatch { table, src, dst }
    }
});
```

Insert calls to the new registry helpers in `tables.rs` using
`.context_to()?` (no manual `map_err` mapping function needed):

- `copy_all`: at the very end (after the last `copy!(...)` macro),
  before `Ok(total)`:

  ```rust
  total += uptrakit_plugin_infrastructure_registry::copy_plugin_tables(src, dst, batch_size)
      .await
      .context_to()?;
  ```

- `clean_all`: at the very start (before the first `clean!(...)`):

  ```rust
  uptrakit_plugin_infrastructure_registry::clean_plugin_tables(dst)
      .await
      .context_to()?;
  ```

  (Plugin tables come first in clean because they are leaves of the FK
  graph relative to core; core tables follow. This is the inverse of the
  copy order.)

- `verify_all`: at the very end:

  ```rust
  total += uptrakit_plugin_infrastructure_registry::verify_plugin_tables(src, dst)
      .await
      .context_to()?;
  ```

The test-only `COPY_ORDER` length assertion (`assert_eq!(COPY_ORDER.len(), 49,
"COPY_ORDER must list all 49 app tables")` at line ~406) becomes wrong
because `COPY_ORDER` shrinks by one to 48. Fix in Wave D, not here — Wave B
just decrements the literal to 48 to keep the test green if it runs:

```rust
assert_eq!(
    COPY_ORDER.len(),
    48,
    "COPY_ORDER must list all 48 core tables; plugin tables are registered via PluginDescriptor"
);
```

(Wave D replaces the entire test with a structural check.)

### Acceptance

- `crates/shared/db/src/entity/proxmox_host_mapping.rs` does not exist.
- `pub mod proxmox_host_mapping` is removed from `entity/mod.rs`.
- `ProxmoxHostMapping` re-export is removed from `entity/prelude.rs`.
- `impl TenantScoped` for `proxmox_host_mapping::Entity` is removed from
  `tenant_scoped.rs`; the equivalent impl exists in the plugin's entity file.
- `grep -rn "uptrakit_shared_db::entity::proxmox_host_mapping" crates/plugins/infrastructure/proxmox/`
  returns zero hits.
- `grep -nE "ProxmoxHostMapping|proxmox_host_mappings" crates/core/controller-runtime/src/db_migrate/tables.rs`
  returns zero hits.
- `cargo check --all-features` clean.
- `cargo check --no-default-features --features db-sqlite` clean.
- `cargo clippy --all-targets --all-features` clean.
- `cargo test --all-features` passes.
- `cargo test -p uptrakit-controller db_migrate -- --ignored` passes
  (the `migrate_sqlite_to_sqlite_roundtrip` integration test verifies that
  every table — including the now-plugin-owned `proxmox_host_mappings` — is
  copied and verified).

---

## Wave D — Audit hardening + script consolidation

**Goal:** Detect future regressions of the leaks Wave B fixed; replace the
brittle `COPY_ORDER` length assertion with a schema-driven structural check;
delete the legacy shell checker.

### New Python rule `RULE_PLUGIN_ENTITY_IN_SHARED_DB`

In `ci/check_plugin_semantic_boundary.py`:

```python
RULE_PLUGIN_ENTITY_IN_SHARED_DB = "plugin-entity-in-shared-db"
```

Add to `KNOWN_RULE_IDS` and to `RULE_MATCH_KINDS`:

```python
RULE_PLUGIN_ENTITY_IN_SHARED_DB: {"file_path", "module_token"},
```

Plugin-owned entity stems are derived **dynamically** at checker startup
by scanning `crates/plugins/**/entity/*.rs` files. The set of "stems
owned by plugins" is the union of those filenames (without the `.rs`
suffix), excluding `mod.rs`, `prelude.rs`, and `tenant_scoped.rs`.
Hardcoding a fixed prefix list (`proxmox_`, `docker_`, …) would silently
miss future plugin families; dynamic discovery keeps the checker in lock-step
with the actual plugin tree.

```python
def _plugin_owned_entity_stems(repo_root: Path) -> frozenset[str]:
    """Return entity-file stems owned by plugins.

    Scans every `crates/plugins/**/src/entity/*.rs` file (non-recursive
    inside the entity directory) and returns the set of stems. These
    stems must not appear under `crates/shared/db/src/entity/`.
    """
    plugins_root = repo_root / "crates" / "plugins"
    skip = {"mod.rs", "prelude.rs", "tenant_scoped.rs"}
    stems: set[str] = set()
    for entity_dir in plugins_root.glob("*/*/src/entity"):
        for path in entity_dir.glob("*.rs"):
            if path.name in skip:
                continue
            stems.add(path.stem)
    return frozenset(stems)
```

Two checks:

**Check (a) — filename collision.** For each `.rs` file directly under
`crates/shared/db/src/entity/` (non-recursive; `mod.rs`, `prelude.rs`, and
`tenant_scoped.rs` are not entities), fail with `match_kind="file_path"` if
the filename stem appears in `_plugin_owned_entity_stems(...)`.

**Check (b) — `mod.rs` / `prelude.rs` token scan.** In
`crates/shared/db/src/entity/mod.rs` and
`crates/shared/db/src/entity/prelude.rs`, fail with
`match_kind="module_token"` if a line declares or re-exports a module
whose name appears in `_plugin_owned_entity_stems(...)`:

- `pub mod <stem>;`
- `pub use super::<stem>::...;`

Both checks emit `Finding(rule_id=RULE_PLUGIN_ENTITY_IN_SHARED_DB, ...)`.

This design self-updates: a new plugin entity automatically blocks any
attempt to host the same entity in `shared-db`, with no checker change
required.

**Naming-convention implication.** Because the rule fires on stem
collisions, plugin entity files must use plugin-prefixed names
(`proxmox_*`, `docker_*`, …) to avoid clashing with core entities like
`host.rs`, `service.rs`, `tag.rs`. This is already the established
convention; the rule turns it into a hard requirement. If a plugin
author legitimately needs to introduce a new entity whose stem matches a
core entity, the resolution is to rename the plugin entity (with a
prefix), not to weaken the rule. Document this expectation in
`docs/development/plugin-guidelines.md` alongside the existing plugin
naming guidance.

Per-rule allowlist support is reused from the existing checker
infrastructure. No allowlist entries are added by this spec.

### Replace `COPY_ORDER` length assertion with structural test

The current test at `crates/core/controller-runtime/src/db_migrate/tables.rs:406`
asserts a hardcoded length (49 today, 48 after Wave B). It does not catch
real schema drift — only "someone changed the count." Replace with a
schema-driven check.

After Wave C, `COPY_ORDER` lives at
`uptrakit_shared_db::migrate_core_tables::CORE_COPY_ORDER`. Before Wave C,
keep the old name. Implementation plan handles the rename.

```rust
#[tokio::test]
#[ignore = "integration — runs schema migrations on in-memory SQLite"]
async fn migration_coverage_complete() {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DbBackend, Statement};
    use std::collections::HashSet;

    let opt = ConnectOptions::new("sqlite::memory:");
    let db = Database::connect(opt).await.expect("source db");
    crate::migration::run_migrations(&db)
        .await
        .expect("source migrations");

    // Every live application table.
    let live: HashSet<String> = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master \
             WHERE type='table' \
               AND name NOT LIKE 'sqlite_%' \
               AND name != 'seaql_migrations'"
                .to_owned(),
        ))
        .await
        .expect("query live tables")
        .into_iter()
        .map(|row| row.try_get::<String>("", "name").expect("name"))
        .collect();

    // Tables covered by the migration code.
    let mut covered: HashSet<String> = super::tables::COPY_ORDER
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    for descriptor in uptrakit_plugin_infrastructure_registry::all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            for td in tables_fn() {
                covered.insert(td.name.to_owned());
            }
        }
    }

    let missing: Vec<_> = live.difference(&covered).cloned().collect();
    let extra: Vec<_> = covered.difference(&live).cloned().collect();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "schema drift between migrations and db-migrate coverage:\n  \
         missing from migration: {missing:?}\n  \
         extra in lists: {extra:?}"
    );
}
```

The test catches:

- A new entity migration without a corresponding entry in
  `COPY_ORDER` / `CORE_COPY_ORDER` or a plugin descriptor.
- A stale entry pointing to a dropped table.

`#[ignore]` matches the existing `migrate_sqlite_to_sqlite_roundtrip` test
because both run full schema migrations (slow). Same `--ignored` invocation
runs both.

The old `assert_eq!(COPY_ORDER.len(), 48, ...)` test (whichever ID it has
after Wave B) is deleted.

### Delete the shell checker

Verify each rule in `ci/check_plugin_semantic_boundary.sh` has a Python
equivalent. Mapping (verified against current source):

| Shell rule                                                  | Python rule                                                                               |
| ----------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `dashboard-icons bespoke surface`                           | `RULE_LEGACY_DASHBOARD_BESPOKE_SURFACE` (verify pattern)                                  |
| `PluginTypeId semantic helper callsites/uses`               | `RULE_FORBIDDEN_PLUGIN_HELPER`                                                            |
| `PluginTypeId semantic helper definitions`                  | `RULE_FORBIDDEN_PLUGIN_HELPER`                                                            |
| `identity-specific helpers`                                 | `RULE_FORBIDDEN_PLUGIN_HELPER`                                                            |
| `plugin_ids token references in non-plugin production code` | `RULE_PLUGIN_IDS_REFERENCE` (predecessor Wave 7 added inline qualified-path scan — Fix A) |

For each row, the implementation plan adds a checker test fixture that
(a) reproduces the input that triggered the shell rule, and (b) verifies
the Python rule fires. If any shell pattern lacks Python coverage, port it
before deleting the shell file. (Current expectation: all five have
equivalents.)

**Caveat on `RULE_LEGACY_DASHBOARD_BESPOKE_SURFACE`.** The Python checker
intentionally excludes this rule ID from `KNOWN_RULE_IDS` (see comment at
the constant definition: "Intentionally excluded from KNOWN_RULE_IDS so
allowlists remain spec-canonical only"). Findings emitted with this
`rule_id` still surface as violations, but the rule cannot be referenced
by allowlists. The implementation plan must verify that the rule's
detection logic is equivalent to the shell rule and that the existing
Python rule still fires on the same inputs that triggered the shell
rule — without adding the rule ID to `KNOWN_RULE_IDS` (that would
contradict the existing comment). If a discrepancy is found, fix the
Python rule's pattern to match shell behaviour rather than touching
`KNOWN_RULE_IDS`.

Then:

- Delete `ci/check_plugin_semantic_boundary.sh`.
- Verify `.github/workflows/ci.yml` does not reference the shell script
  (the predecessor's Wave 7 proposed adding it; this consolidation skips
  that step — only the Python checker runs in CI per `ci.yml:57`).
- Update any docs that reference the shell script (search:
  `docs/development/plugin-guidelines.md`,
  `docs/development/coding-standards.md`, etc.) to reference only the
  Python checker.

### Acceptance

- `python3 ci/check_plugin_semantic_boundary.py` exits 0 against the
  post-Wave-B codebase.
- Fixture: copying any existing plugin entity file
  (e.g. `crates/plugins/infrastructure/proxmox/src/entity/proxmox_host_mapping.rs`)
  back into `crates/shared/db/src/entity/` causes the checker to exit
  non-zero with `RULE_PLUGIN_ENTITY_IN_SHARED_DB` (`match_kind="file_path"`).
- Fixture: adding `pub use super::proxmox_host_mapping::Foo;` to
  `crates/shared/db/src/entity/prelude.rs` causes the checker to exit
  non-zero with `RULE_PLUGIN_ENTITY_IN_SHARED_DB` (`match_kind="module_token"`)
  — the stem `proxmox_host_mapping` is dynamically discovered as
  plugin-owned because it lives at `crates/plugins/infrastructure/proxmox/src/entity/proxmox_host_mapping.rs`.
- `migration_coverage_complete` passes against the post-Wave-B codebase.
- Artificially deleting one entry from a plugin's `db_migrate_tables`
  causes `migration_coverage_complete` to fail with a helpful diff
  showing the missing table name.
- `ci/check_plugin_semantic_boundary.sh` does not exist.
- `.github/workflows/ci.yml` does not reference the shell script.
- `cargo test --all-features` passes.

---

## Wave C — Move core table list to `shared-db`

**Goal:** Move per-table operations and the `COPY_ORDER` list from
`controller-runtime/db_migrate/tables.rs` to a new
`shared-db::migrate_core_tables` module, leaving `tables.rs` as a thin
"core then plugins" orchestrator.

### `shared-db` feature `db-migrate` (added in Wave A; helpers added here)

The `db-migrate` feature flag was already declared in
`crates/shared/db/Cargo.toml` during Wave A (alongside the
`TableMigrateError` enum). Wave C extends `migrate_core_tables` with the
actual `copy` / `clean` / `verify` functions plus `CORE_COPY_ORDER`. No
new feature flag, no new dependencies — `sea-orm` is already a workspace
dep.

### New module `shared-db::migrate_core_tables`

Create `crates/shared/db/src/migrate_core_tables.rs`. Under
`#[cfg(feature = "db-migrate")]`, host:

- `pub async fn copy(src, dst, batch_size) -> Result<u64, Report<TableMigrateError>>`
- `pub async fn clean(dst) -> Result<(), Report<TableMigrateError>>`
- `pub async fn verify(src, dst) -> Result<u64, Report<TableMigrateError>>`
- `pub const CORE_COPY_ORDER: &[&str] = &[ … ];` — the 48 core tables
  (today's `COPY_ORDER` minus `proxmox_host_mappings`).

Each helper attaches the table name via `report!(TableMigrateError::Db { table, err })`
when wrapping a `sea_orm::DbErr` from the per-table primitive
(`copy_one` / `clean_one` / `verify_one`), and via
`bail!(TableMigrateError::Mismatch { table, src, dst })` when row counts
disagree. Wrapping happens in the boundary helper, not in the inner
generic primitive — same pattern as the registry helpers.

Implementation: copy the macro bodies of `copy_all`, `clean_all`,
`verify_all` from
`crates/core/controller-runtime/src/db_migrate/tables.rs` (lines 73–139,
147–208, 214–277). Adjustments:

- Remove the proxmox row (already done in Wave B but the line numbers
  might shift; ensure `proxmox_host_mappings` is absent from the moved
  `CORE_COPY_ORDER`).
- Change error type from `Result<_, Report<DbMigrateError>>` to
  `Result<_, Report<TableMigrateError>>` (the alias `Result<T>` declared
  in Wave A's module). Core helpers preserve the table name via
  `report!(TableMigrateError::Db { table, err })` and
  `bail!(TableMigrateError::Mismatch { table, src, dst })`. The
  orchestrator uses `.context_to()?` (powered by the
  `impl_report_conversion!(TableMigrateError => DbMigrateError, ...)`
  block defined in Wave B) to fold both into the existing
  `DbMigrateError::TableOp` and `DbMigrateError::Mismatch` variants.

- Move generic helpers `migrate_table<E>`, `clean_table<E>`,
  `verify_table<E>` (lines 287–397 of `tables.rs`) into
  `migrate_core_tables.rs` as private helpers, adapted to return
  `Result<_, Report<TableMigrateError>>` (each helper takes the table
  name and constructs the appropriate variant on error via `report!()` /
  `bail!()`). The same logic that `plugin-infrastructure-core::db_migrate`
  uses for plugins applies here; consider extracting the per-entity
  helpers into a shared private function inside
  `shared-db::migrate_core_tables` if the duplication is meaningful — but
  YAGNI applies: a single use site is fine.

### `controller-runtime` becomes thin orchestrator

`crates/core/controller-runtime/Cargo.toml` already declares
`uptrakit-shared-db = { workspace = true, features = ["migration"] }`.
Extend the feature list with `"db-migrate"` (do not add a new `dependencies`
entry).

`crates/core/controller-runtime/src/db_migrate/tables.rs` is rewritten
(target ≤ 50 LoC):

```rust
//! Database data migration — orchestrator over core tables (in `shared-db`)
//! and plugin tables (registered via `PluginDescriptor::db_migrate_tables`).

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;

use super::error::{DbMigrateError, Result};

pub(crate) async fn copy_all(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64> {
    let mut total = uptrakit_shared_db::migrate_core_tables::copy(src, dst, batch_size)
        .await
        .context_to()?;
    total += uptrakit_plugin_infrastructure_registry::copy_plugin_tables(src, dst, batch_size)
        .await
        .context_to()?;
    Ok(total)
}

pub(crate) async fn clean_all(dst: &DatabaseConnection) -> Result<()> {
    // Plugin tables first (FK leaves of the core graph).
    uptrakit_plugin_infrastructure_registry::clean_plugin_tables(dst)
        .await
        .context_to()?;
    uptrakit_shared_db::migrate_core_tables::clean(dst)
        .await
        .context_to()?;
    Ok(())
}

pub(crate) async fn verify_all(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
) -> Result<u64> {
    let mut total = uptrakit_shared_db::migrate_core_tables::verify(src, dst)
        .await
        .context_to()?;
    total += uptrakit_plugin_infrastructure_registry::verify_plugin_tables(src, dst)
        .await
        .context_to()?;
    Ok(total)
}
```

Both core and plugin paths return `Report<TableMigrateError>`. The single
`impl_report_conversion!(TableMigrateError => DbMigrateError, ...)` block
defined alongside `DbMigrateError` (Wave B) folds variants back into
the existing `DbMigrateError::TableOp` and `DbMigrateError::Mismatch` —
no new variants are introduced and no variant becomes orphaned.

### Update `migration_coverage_complete`

After Wave C, `COPY_ORDER` no longer exists in `tables.rs` — it lives at
`uptrakit_shared_db::migrate_core_tables::CORE_COPY_ORDER`. Update the
test to import from there:

```rust
let mut covered: HashSet<String> =
    uptrakit_shared_db::migrate_core_tables::CORE_COPY_ORDER
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
```

(Move the test to `crates/shared/db/src/migrate_core_tables.rs` if more
natural. Decide during implementation.)

### Acceptance

- `crates/core/controller-runtime/src/db_migrate/tables.rs` ≤ 50 LoC
  (excluding the file's `mod tests` block, which can grow).
- `crates/shared/db/src/migrate_core_tables.rs` exists under `db-migrate`
  feature flag.
- `cargo check --no-default-features --features db-sqlite` clean.
- `cargo check --all-features` clean.
- `cargo clippy --all-targets --all-features` clean.
- `migrate_sqlite_to_sqlite_roundtrip` (integration test, `--ignored`)
  passes.
- `migration_coverage_complete` (integration test, `--ignored`) passes.

---

## Full acceptance criteria

| Check                                                                                              | Expected                       |
| -------------------------------------------------------------------------------------------------- | ------------------------------ |
| `crates/shared/db/src/entity/proxmox_host_mapping.rs`                                              | Does not exist                 |
| `pub mod proxmox_host_mapping` in `shared-db/entity/mod.rs`                                        | Removed                        |
| `ProxmoxHostMapping` re-export in `shared-db/entity/prelude.rs`                                    | Removed                        |
| `impl TenantScoped for proxmox_host_mapping::Entity` in `shared-db`                                | Removed                        |
| `use uptrakit_shared_db::entity::proxmox_host_mapping` in `crates/plugins/infrastructure/proxmox/` | Zero                           |
| `ProxmoxHostMapping` / `proxmox_host_mappings` in `controller-runtime/db_migrate/tables.rs`        | Zero                           |
| `controller-runtime/db_migrate/tables.rs` LoC                                                      | ≤ 50 (excluding `mod tests`)   |
| `ci/check_plugin_semantic_boundary.sh`                                                             | Does not exist                 |
| `.github/workflows/ci.yml` references the shell script                                             | Zero                           |
| `RULE_PLUGIN_ENTITY_IN_SHARED_DB` filename + module-token rules                                    | Both fire on injected fixtures |
| `migration_coverage_complete` structural test                                                      | Passes                         |
| `migrate_sqlite_to_sqlite_roundtrip` integration test                                              | Passes                         |
| `cargo check --all-features`                                                                       | Clean                          |
| `cargo check --no-default-features --features db-sqlite`                                           | Clean                          |
| `cargo clippy --all-targets --all-features`                                                        | Clean                          |
| `cargo test --all-features`                                                                        | Passes                         |
| `cargo deny check`                                                                                 | Clean                          |
| `python3 ci/check_plugin_semantic_boundary.py`                                                     | Exits 0                        |

## Notes

- **Predecessor audit grep.** The predecessor's Wave 6 example
  (`grep -rn ... | grep "use "`) silently skipped macro-style references
  like `copy!(ProxmoxHostMapping, ...)`. This is a documentation defect in
  the predecessor; future audit runs should drop the `| grep "use "`
  pipeline. We do not modify the predecessor spec file — record this here
  for traceability.
- **Inter-plugin-table FK.** `proxmox_protection_audit.mapping_id` →
  `proxmox_host_mappings.id` (`SetNull`) is the only inter-plugin-table FK
  in the codebase today. Plugin's `db_migrate_tables` Vec must place
  `host_mapping` before `protection_audit`. The doc comment on
  `DbMigrateTablesFn` states this rule generically (parents before
  children); the Proxmox plugin's comment cites the specific FK.
- **Cross-plugin ordering caveat.** Inter-plugin order in the registry
  helpers is registration order in `all_descriptors()`. We have no
  cross-plugin FKs today; the same TODO already documented for
  `reset_plugin_tenant_data` covers the future case where a `migration_order`
  hint becomes necessary.
- **Agent-side proxmox tables.** `proxmox_host_state` and
  `proxmox_pending_matches` (defined in
  `crates/shared/db/src/migration/m20260331_000001_ssh_agent_tables.rs` and
  `crates/core/agent-ssh/src/db/migration/m20260307_000002_pending_proxmox_matches.rs`)
  are agent-side SSH features and unrelated to the controller-side Proxmox
  plugin. They are deliberately not in scope. The new
  `RULE_PLUGIN_ENTITY_IN_SHARED_DB` rule scans
  `crates/shared/db/src/entity/` only — migration files are not affected.

## Self-review

After writing the spec, verify:

1. Each wave acceptance is independently CI-green; no wave depends on a
   later wave to compile.
2. Wave order A → B → D → C is consistent: D's new rules would fail before
   B; C is independent of D.
3. The FK-ordering rule appears on the `DbMigrateTablesFn` doc comment with
   a concrete example.
4. `for_entity::<E>()` trait bounds match those on `migrate_table<E>` in
   today's `controller-runtime/db_migrate/tables.rs`.
5. No pseudocode placeholders remain in registry helpers, error mapping,
   or test fixtures (beyond the explicit `todo!()` markers in `db_migrate.rs`
   that direct the implementation plan to the original bodies).
6. The predecessor relationship is explicit in Background; this spec does
   not contradict the predecessor's already-landed commitments.
