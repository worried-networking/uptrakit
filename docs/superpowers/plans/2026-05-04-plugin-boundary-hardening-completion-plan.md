# Plugin Boundary Hardening — Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the plugin boundary hardening from
`2026-05-03-plugin-boundary-hardening.md`: move `proxmox_host_mapping` out of
`shared-db`, route `db-migrate` through `PluginDescriptor::db_migrate_tables`,
harden the Python audit checker against shared-db plugin-entity leaks, and
consolidate the audit scripts.

**Architecture:** Four atomic CI-green phases. Phase A adds type-system
primitives in `plugin-infrastructure-core` (purely additive). Phase B moves
the entity and flips `db-migrate` dispatch through the registry in one atomic
commit. Phase D adds the new Python audit rule and replaces the brittle
`COPY_ORDER` length assertion with a schema-driven structural test, then
deletes the legacy shell checker. Phase C moves the core table list and
per-table operations into `shared-db::migrate_core_tables`, leaving
`controller-runtime/db_migrate/tables.rs` as a thin orchestrator. Order is
fixed by design: A → B → D → C.

**Tech Stack:** Rust workspace, SeaORM (sea-orm), `rootcause::Report` for
error propagation, `thiserror` for error enums, Python 3 for the boundary
checker, shell + bash for hooks. SQLite (in-memory) for integration tests.

**Spec:** `docs/superpowers/specs/2026-05-04-plugin-boundary-hardening-completion.md`

---

## Conventions

- All Rust code must pass: `cargo fmt --all`, `cargo check --all-features`,
  `cargo check --no-default-features --features db-sqlite`, `cargo clippy
--all-targets --all-features`, `cargo test --all-features`.
- Markdown must pass `npx prettier --write <file>` and `npx markdownlint --config .markdownlint.json <file>`.
- Never use `unwrap()` in production code. Use `rootcause::Report` + `report!()` / `bail!()` / `.context_to()?`.
- All extensible public enums require `#[non_exhaustive]`.
- `TenantScoped` trait method is `tenant_id_column()` (returns `Self::Column`), not `tenant_column()`.
- Project's commit conventions: `<type>(scope): subject` (Conventional
  Commits). `<type>` is one of
  `feat|fix|refactor|test|docs|chore|build|ci|perf|style`.
- Frequent commits — one logical change per commit. Each task ends with a commit.
- **Atomic-phase rule clarification.** Phase B and Phase C each land as
  a single **pushed** commit. Local staging, `git stash`, and squashed
  WIP commits during investigation are fine — the invariant is that no
  intermediate state ever reaches the remote. If mid-Phase-B you
  discover a Phase A primitive is incomplete, do **not** split Phase B;
  stash, fix Phase A as a new commit, then resume Phase B.
- **Per-task rollback rule.** If a quality gate fails inside a task,
  fix the cause in place. If the root cause is in a prior already-
  committed task, revert that commit (or land a follow-up fix) and
  redo. Never paper over with `#[allow(...)]` unless the lint is a
  documented false positive.

---

## Phase A — Type-System Primitives

**Objective:** Introduce `PluginTableDescriptor`, `DbMigrateTablesFn`, the
`db_migrate_tables` field on `PluginDescriptor`, the `TableMigrateError` enum
in `shared-db`, the `db-migrate` feature flag, the per-entity generic helpers
in `plugin-infrastructure-core::db_migrate`, and the three registry helpers
(`copy_plugin_tables` / `clean_plugin_tables` / `verify_plugin_tables`). No
plugin populates the new field yet; current `db-migrate` semantics are
unchanged.

### Task A1: Add `db-migrate` feature flag to `shared-db`

**Files:**

- Modify: `crates/shared/db/Cargo.toml`

- [ ] **Step 1: Read the current Cargo.toml**

Run: `cat crates/shared/db/Cargo.toml`
Note the existing `[features]` section.

- [ ] **Step 2: Add the new feature flag**

Edit `crates/shared/db/Cargo.toml`. Under the `[features]` section, add a new entry:

```toml
db-migrate = []
```

The flag has no dependency activations yet — `sea-orm` is already a workspace dep on the crate.

- [ ] **Step 3: Verify the change compiles**

Run: `cargo check -p uptrakit-shared-db --features db-migrate`
Expected: clean output, no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/db/Cargo.toml
git commit -m "feat(shared-db): add db-migrate feature flag"
```

### Task A2: Add `TableMigrateError` enum to `shared-db`

**Files:**

- Create: `crates/shared/db/src/migrate_core_tables.rs`
- Modify: `crates/shared/db/src/lib.rs`

- [ ] **Step 1: Verify `thiserror` and `rootcause` are deps of `shared-db`**

Run: `grep -E "rootcause|thiserror" crates/shared/db/Cargo.toml`
Expected: both listed under `[dependencies]`. If either is missing, add via `workspace = true`.

- [ ] **Step 2: Create the new module**

Create `crates/shared/db/src/migrate_core_tables.rs` with:

```rust
//! Shared types and (in Phase C) per-table operations for the
//! `db-migrate` subcommand.
//!
//! This module hosts `TableMigrateError`, returned by both the registry
//! plugin-table helpers (in `plugin-infrastructure-registry`) and the
//! core helpers (added in Phase C). Hosting it in `shared-db` avoids a
//! dependency cycle: `plugin-infrastructure-core` already takes
//! `uptrakit-shared-db` as an optional dep, so the reverse direction is
//! impossible.

#![cfg(feature = "db-migrate")]

use rootcause::prelude::*;

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
pub type Result<T> = std::result::Result<T, Report<TableMigrateError>>;
```

- [ ] **Step 3: Register the module in `lib.rs`**

Open `crates/shared/db/src/lib.rs`. Add near the other `pub mod` declarations:

```rust
#[cfg(feature = "db-migrate")]
pub mod migrate_core_tables;
```

- [ ] **Step 4: Verify the module compiles under both feature settings**

Run:

```bash
cargo check -p uptrakit-shared-db
cargo check -p uptrakit-shared-db --features db-migrate
```

Expected: both clean.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/db/src/migrate_core_tables.rs crates/shared/db/src/lib.rs
git commit -m "feat(shared-db): add TableMigrateError enum under db-migrate feature"
```

### Task A3: Wire `plugin-infrastructure-core/migrations` to pull in `shared-db/db-migrate`

**Files:**

- Modify: `crates/plugins/infrastructure/core/Cargo.toml`

- [ ] **Step 1: Read the current Cargo.toml**

Run: `cat crates/plugins/infrastructure/core/Cargo.toml`
Locate the existing `[features]` block, especially the `migrations`
feature. Note: `uptrakit-shared-db = { workspace = true, optional = true }`
is already declared. Currently `dep:uptrakit-shared-db` is activated only
by the `catalog` feature, **not** by `migrations`. Wave A needs both
the dep activation AND the `db-migrate` feature activation under
`migrations`.

- [ ] **Step 2: Extend the `migrations` feature**

Edit `crates/plugins/infrastructure/core/Cargo.toml`. The current
`migrations` feature reads roughly:

```toml
migrations = ["dep:sea-orm", "dep:sea-orm-migration"]
```

Replace with:

```toml
migrations = [
    "dep:sea-orm",
    "dep:sea-orm-migration",
    "dep:uptrakit-shared-db",
    "uptrakit-shared-db/db-migrate",
]
```

Keep all other activations the existing `migrations` feature has — the
example above is not exhaustive; verify against the actual file.

`dep:uptrakit-shared-db` is required because `migrations` needs the
optional dep itself activated (the `catalog` feature already activates
it for a different purpose; both features must independently activate
the dep). `uptrakit-shared-db/db-migrate` then activates the new
`db-migrate` feature on the dep so `TableMigrateError` resolves.

- [ ] **Step 3: Verify compilation**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-core --features migrations
```

Expected: both clean.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/Cargo.toml
git commit -m "build(plugin-infra-core): activate shared-db/db-migrate under migrations feature"
```

### Task A4: Add per-entity generic helpers in `plugin-infrastructure-core::db_migrate`

**Files:**

- Create: `crates/plugins/infrastructure/core/src/db_migrate.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`

- [ ] **Step 1: Create the new module**

Create `crates/plugins/infrastructure/core/src/db_migrate.rs`:

```rust
//! Generic per-table operations for the `db-migrate` subcommand.
//!
//! Plugins do not call these directly — they construct descriptors via
//! [`crate::PluginTableDescriptor::for_entity`], which captures `E` and
//! produces type-erased fn pointers wrapping these helpers.
//!
//! Each helper returns `Result<_, sea_orm::DbErr>` (no table name in
//! scope). The boundary helper (in `plugin-infrastructure-registry` or
//! `shared-db::migrate_core_tables`) wraps with `report!()` to attach
//! the table name as `TableMigrateError::Db { table, err }`.

#![cfg(feature = "migrations")]

use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, DatabaseConnection, DbErr, EntityTrait,
    IntoActiveModel, PaginatorTrait, QuerySelect,
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
        let active: Vec<_> = batch
            .into_iter()
            .map(IntoActiveModel::into_active_model)
            .collect();
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

pub(crate) async fn verify_one<E>(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
) -> Result<(u64, u64), DbErr>
where
    E: EntityTrait + 'static,
    E::Model: Send + Sync + 'static,
{
    let src_count = E::find().count(src).await?;
    let dst_count = E::find().count(dst).await?;
    Ok((src_count, dst_count))
}
```

- [ ] **Step 2: Register the module in `lib.rs`**

Open `crates/plugins/infrastructure/core/src/lib.rs`. Add near the other `mod`/`pub mod` declarations:

```rust
#[cfg(feature = "migrations")]
pub(crate) mod db_migrate;
```

(The module is `pub(crate)` — only `PluginTableDescriptor::for_entity` references it.)

- [ ] **Step 3: Verify compilation under both feature settings**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-core --no-default-features
cargo check -p uptrakit-plugin-infrastructure-core --features migrations
```

Expected: both clean.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/src/db_migrate.rs crates/plugins/infrastructure/core/src/lib.rs
git commit -m "feat(plugin-infra-core): add generic db-migrate per-table helpers"
```

### Task A5: Add `PluginTableDescriptor` + `DbMigrateTablesFn` to `descriptor.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`

- [ ] **Step 1: Read the existing `ResetTenantDataFn` block**

Run: `sed -n '243,260p' crates/plugins/infrastructure/core/src/descriptor.rs`
Note the dual-alias pattern (real definition under `#[cfg(feature = "migrations")]`, placeholder under `not`).

- [ ] **Step 2: Add `PluginTableDescriptor` and `DbMigrateTablesFn`**

Insert after the `ResetTenantDataFn` definitions (around line 258), before the `Role slots` section header:

````rust
/// Per-table copy/clean/verify operations for the `db-migrate` subcommand.
///
/// Constructed via [`PluginTableDescriptor::for_entity`].
///
/// Type erasure: the closures monomorphise the generic `copy_one` /
/// `clean_one` / `verify_one` helpers (in `crate::db_migrate`) per
/// entity `E`. Each `for_entity::<E>(...)` call produces a descriptor
/// with the same shape regardless of `E`.
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
    /// disagreement — the descriptor itself does not carry the table
    /// name into the closure body.
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
/// fail with a FK violation, but clean might succeed by accident
/// depending on FK action.
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
///         // ... independents ...
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

- [ ] **Step 3: Add the `db_migrate_tables` field to `PluginDescriptor`**

Locate the `PluginDescriptor` struct. After the existing `pub reset_tenant_data: Option<ResetTenantDataFn>,` field, insert:

```rust
/// Plugin-owned tables registered for the `db-migrate` subcommand.
/// `None` for plugins with no own tables.
///
/// Real type only meaningful under `migrations` feature; outside it,
/// `DbMigrateTablesFn` is a `fn()` placeholder.
pub db_migrate_tables: Option<DbMigrateTablesFn>,
```

- [ ] **Step 4: Re-export `PluginTableDescriptor` and `DbMigrateTablesFn` from `lib.rs`**

In `crates/plugins/infrastructure/core/src/lib.rs`, find the section that
re-exports `descriptor::*` items (or specific names). Add
`PluginTableDescriptor`, `DbMigrateTablesFn` to the public surface so plugin
crates can `use uptrakit_plugin_infrastructure_core::PluginTableDescriptor`.

If the existing pattern is `pub use crate::descriptor::*;`, no change is needed. If it's selective, append the two new names.

- [ ] **Step 5: Verify compilation**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-core --features migrations
cargo check --workspace
```

Expected: clean. The `cargo check --workspace` will fail until Task A6 updates the macro — that is **not** expected here. Run only the first two now.

Adjust: only run

```bash
cargo check -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-core --features migrations
```

The workspace check is deferred to A6.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/core/src/descriptor.rs crates/plugins/infrastructure/core/src/lib.rs
git commit -m "feat(plugin-infra-core): add PluginTableDescriptor and db_migrate_tables field"
```

### Task A6: Extend `declare_plugin!` macro with optional `db_migrate_tables` parameter

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/macros.rs`

- [ ] **Step 1: Locate the optional parameter list**

Run: `sed -n '50,65p' crates/plugins/infrastructure/core/src/macros.rs`
Note the existing optional `migrations:` and `reset_tenant_data:` patterns at lines 60–61.

- [ ] **Step 2: Add the new optional parameter**

Edit `crates/plugins/infrastructure/core/src/macros.rs`. After the `reset_tenant_data` optional parameter line (currently around line 61):

```rust
$(, db_migrate_tables: $db_migrate_fn:expr )?
```

- [ ] **Step 3: Emit the field in the generated struct literal**

Locate the struct literal section (currently around lines 268–269 —
`migrations:` and `reset_tenant_data:` lines). After the `reset_tenant_data:`
line, add:

```rust
db_migrate_tables: $crate::__option_expr!( $( $db_migrate_fn )? ),
```

- [ ] **Step 4: Verify the workspace builds end-to-end**

Run:

```bash
cargo check --workspace
cargo check --workspace --all-features
```

Expected: clean. Every existing `declare_plugin!` invocation defaults to `None` for the new field.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/macros.rs
git commit -m "feat(plugin-infra-core): add db_migrate_tables to declare_plugin! macro"
```

### Task A7: Add direct `shared-db` dep to `plugin-infrastructure-registry`

**Files:**

- Modify: `crates/plugins/infrastructure/registry/Cargo.toml`

- [ ] **Step 1: Read the current Cargo.toml**

Run: `cat crates/plugins/infrastructure/registry/Cargo.toml`
Locate the `[dependencies]` and `[features]` blocks.

- [ ] **Step 2: Add the optional dep entry**

Under `[dependencies]`, add (or update if already present in some other form):

```toml
uptrakit-shared-db = { workspace = true, optional = true }
```

- [ ] **Step 3: Activate it under `migrations` feature**

In the `[features]` block, locate the `migrations` feature. Add the activations:

```toml
migrations = [
    # ... existing activations ...
    "dep:uptrakit-shared-db",
    "uptrakit-shared-db/db-migrate",
]
```

If the registry already pulls `migrations` indirectly via `plugin-infrastructure-core`, keep that pattern and just append the two new entries.

- [ ] **Step 4: Verify compilation**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-registry
cargo check -p uptrakit-plugin-infrastructure-registry --features migrations
```

Expected: both clean.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/registry/Cargo.toml
git commit -m "build(plugin-infra-registry): add direct shared-db dep gated on migrations"
```

### Task A8: Add `copy_plugin_tables` / `clean_plugin_tables` / `verify_plugin_tables` registry helpers

**Files:**

- Modify: `crates/plugins/infrastructure/registry/src/lib.rs`

- [ ] **Step 1: Read the existing `reset_plugin_tenant_data` helper**

Run: `sed -n '95,120p' crates/plugins/infrastructure/registry/src/lib.rs`
Note the existing `#[cfg(feature = "migrations")]` gate and the registration-order iteration pattern.

- [ ] **Step 2: Add the three new helpers**

Insert after `reset_plugin_tenant_data`:

```rust
// ── db-migrate dispatch ────────────────────────────────────────────────────

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
) -> std::result::Result<u64, rootcause::Report<uptrakit_shared_db::migrate_core_tables::TableMigrateError>>
{
    use rootcause::prelude::*;
    use uptrakit_shared_db::migrate_core_tables::TableMigrateError;

    let mut total = 0u64;
    for descriptor in all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            for table in tables_fn() {
                let copied = (table.copy_batch)(src, dst, batch_size)
                    .await
                    .map_err(|err| {
                        report!(TableMigrateError::Db { table: table.name, err })
                    })?;
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
) -> std::result::Result<(), rootcause::Report<uptrakit_shared_db::migrate_core_tables::TableMigrateError>>
{
    use rootcause::prelude::*;
    use uptrakit_shared_db::migrate_core_tables::TableMigrateError;

    for descriptor in all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            // Reverse for FK-safe deletion (children before parents).
            for table in tables_fn().into_iter().rev() {
                (table.clean)(dst)
                    .await
                    .map_err(|err| {
                        report!(TableMigrateError::Db { table: table.name, err })
                    })?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "migrations")]
pub async fn verify_plugin_tables(
    src: &sea_orm::DatabaseConnection,
    dst: &sea_orm::DatabaseConnection,
) -> std::result::Result<u64, rootcause::Report<uptrakit_shared_db::migrate_core_tables::TableMigrateError>>
{
    use rootcause::prelude::*;
    use uptrakit_shared_db::migrate_core_tables::TableMigrateError;

    let mut total = 0u64;
    for descriptor in all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            for table in tables_fn() {
                let (src_count, dst_count) = (table.verify)(src, dst)
                    .await
                    .map_err(|err| {
                        report!(TableMigrateError::Db { table: table.name, err })
                    })?;
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

- [ ] **Step 3: Verify compilation**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-registry --features migrations
cargo check --workspace --all-features
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/registry/src/lib.rs
git commit -m "feat(plugin-infra-registry): add copy/clean/verify_plugin_tables helpers"
```

### Task A9: Phase A acceptance gate

**Files:** none (verification only).

- [ ] **Step 1: Run the full quality gate**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

Expected: every command exits 0. No new test failures introduced.

If clippy fires `dead_code` warnings on the new public items
(`PluginTableDescriptor`, `for_entity`, `copy_plugin_tables`,
`clean_plugin_tables`, `verify_plugin_tables`) because no caller exists
yet, **do not** add `#[allow(dead_code)]` — those callers arrive in
Phase B. Confirm the warnings are actually present (`cargo clippy
--all-targets --all-features 2>&1 | grep -i dead_code`); if any fire,
land Phase B immediately to silence them. The window between A8 and
B's commit should be short.

- [ ] **Step 2: Verify doc comments render**

Run:

```bash
cargo doc --no-deps -p uptrakit-plugin-infrastructure-core \
                    -p uptrakit-shared-db \
                    -p uptrakit-plugin-infrastructure-registry
```

Expected: no warnings about broken doc links or invalid doc syntax.

- [ ] **Step 3: Confirm no plugin populates the new field yet**

Run: `git grep -n "db_migrate_tables:" crates/plugins/`
Expected: zero hits in plugin source files (only in
`core/src/descriptor.rs` and `core/src/macros.rs`).

---

## Phase B — Move `proxmox_host_mapping` + Flip Dispatch

**Objective:** Atomic commit that moves the Proxmox `proxmox_host_mapping`
entity into the plugin crate, registers Proxmox's `db_migrate_tables`, and
replaces the macro-based `tables.rs` invocations with calls to the new
registry helpers. After this phase, `shared-db` carries no Proxmox-specific
code, and `tables.rs` references no plugin-named identifiers.

### Task B1: Create the new entity file in the Proxmox plugin

**Files:**

- Create: `crates/plugins/infrastructure/proxmox/src/entity/proxmox_host_mapping.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/entity/mod.rs`

- [ ] **Step 1: Read the source entity definition**

Run: `cat crates/shared/db/src/entity/proxmox_host_mapping.rs`
Note the full content — `Model`, `Relation`, `Related` impls, `ActiveModelBehavior`.

- [ ] **Step 2: Create the new file**

Create
`crates/plugins/infrastructure/proxmox/src/entity/proxmox_host_mapping.rs`
with the full body of the source file, with `super::tenant`,
`super::plugin_config`, `super::host` paths replaced as follows: keep
`super::` references but the new file lives in the plugin's `entity/` module.
Plugin's `entity/` module does **not** own `tenant`, `plugin_config`, or
`host` modules — those still live in `shared-db`. So the references must be
rewritten to:

```rust
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "proxmox_host_mappings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub plugin_config_id: Uuid,
    pub host_id: Option<Uuid>,
    pub proxmox_node: String,
    pub proxmox_vmid: i32,
    pub proxmox_type: String,
    pub proxmox_name: Option<String>,
    pub proxmox_status: String,
    pub hostname: Option<String>,
    pub ip_addresses: Option<String>,
    pub machine_id: Option<String>,
    pub match_method: Option<String>,
    pub discovered_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "uptrakit_shared_db::entity::tenant::Entity",
        from = "Column::TenantId",
        to = "uptrakit_shared_db::entity::tenant::Column::Id"
    )]
    Tenant,
    #[sea_orm(
        belongs_to = "uptrakit_shared_db::entity::plugin_config::Entity",
        from = "Column::PluginConfigId",
        to = "uptrakit_shared_db::entity::plugin_config::Column::Id"
    )]
    PluginConfig,
    #[sea_orm(
        belongs_to = "uptrakit_shared_db::entity::host::Entity",
        from = "Column::HostId",
        to = "uptrakit_shared_db::entity::host::Column::Id"
    )]
    Host,
}

impl Related<uptrakit_shared_db::entity::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<uptrakit_shared_db::entity::plugin_config::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PluginConfig.def()
    }
}

impl Related<uptrakit_shared_db::entity::host::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Host.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

// ── TenantScoped impl (moved from shared-db tenant_scoped.rs) ──────────

impl uptrakit_tenant_db::TenantScoped for Entity {
    fn tenant_id_column() -> Self::Column {
        Column::TenantId
    }
}
```

- [ ] **Step 3: Register the module**

Open `crates/plugins/infrastructure/proxmox/src/entity/mod.rs`. Add:

```rust
pub mod proxmox_host_mapping;
```

Keep alphabetical or grouping order consistent with the existing entries.

- [ ] **Step 4: Verify SeaORM accepts cross-crate `belongs_to` paths**

The four already-moved Proxmox entities (`proxmox_protection_audit`,
`proxmox_protection_default`, `proxmox_protection_item_override`,
`proxmox_backup_target_cache`) all have `pub enum Relation {}` with no
relations, so they cannot serve as precedent. The new
`proxmox_host_mapping` is the first plugin-owned entity with cross-crate
`belongs_to` relations.

Run: `cargo check -p uptrakit-plugin-infrastructure-proxmox 2>&1 | head -50`
Expected: compiles, OR fails with a `belongs_to` parser error indicating
SeaORM does not accept absolute crate-qualified paths.

If the parser rejects absolute paths, fall back to the documented
SeaORM-style:

```rust
use uptrakit_shared_db::entity::{tenant, plugin_config, host};

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(belongs_to = "tenant::Entity", from = "Column::TenantId", to = "tenant::Column::Id")]
    Tenant,
    // ... etc.
}
```

— i.e. import the modules at the top, then reference them by short name
in `belongs_to`. SeaORM resolves the type via Rust's path lookup at the
expansion site.

Some duplicate-symbol errors against `shared-db`'s copy are expected here
and resolve in B2. Distinguish them from real `belongs_to` parser errors.

- [ ] **Step 5: Do NOT commit yet**

This task is part of Phase B's atomic commit (final commit at Task B8).
All B-tasks build up to one commit.

### Task B2: Delete `proxmox_host_mapping` from `shared-db`

**Files:**

- Delete: `crates/shared/db/src/entity/proxmox_host_mapping.rs`
- Modify: `crates/shared/db/src/entity/mod.rs`
- Modify: `crates/shared/db/src/entity/prelude.rs`
- Modify: `crates/shared/db/src/entity/tenant_scoped.rs`

- [ ] **Step 1: Delete the entity file**

```bash
rm crates/shared/db/src/entity/proxmox_host_mapping.rs
```

- [ ] **Step 2: Remove the `pub mod` declaration**

Open `crates/shared/db/src/entity/mod.rs`. Delete the line:

```rust
pub mod proxmox_host_mapping;
```

(currently around line 62).

- [ ] **Step 3: Remove the prelude re-export**

Open `crates/shared/db/src/entity/prelude.rs`. Delete the three-line block (currently lines 56–58):

```rust
pub use super::proxmox_host_mapping::{
    Entity as ProxmoxHostMapping, Model as ProxmoxHostMappingModel,
};
```

- [ ] **Step 4: Remove the `TenantScoped` impl**

Open `crates/shared/db/src/entity/tenant_scoped.rs`. Delete:

a) `proxmox_host_mapping` from the import list at line 6 (the `use super::{...}` block).

b) The impl block (currently lines 137–141):

```rust
impl TenantScoped for proxmox_host_mapping::Entity {
    fn tenant_id_column() -> Self::Column {
        proxmox_host_mapping::Column::TenantId
    }
}
```

- [ ] **Step 5: Do NOT compile or commit yet**

Compilation will fail until Task B3 updates plugin imports. This is part of the atomic commit.

### Task B3: Update plugin-internal imports of `proxmox_host_mapping`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/surfaces.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/reset.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/discovery.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/matching.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/protection_store.rs`

For every site, replace `use uptrakit_shared_db::entity::proxmox_host_mapping`
with `use crate::entity::proxmox_host_mapping`. Other entities imported
alongside (`host`, `plugin_config`) remain `uptrakit_shared_db::entity::*`.

- [ ] **Step 1: Update `surfaces.rs`**

Open `crates/plugins/infrastructure/proxmox/src/surfaces.rs`. The references at lines 643, 924, 978, 1970–1971 need updating:

- Line 643 currently reads roughly:

  ```rust
  use uptrakit_shared_db::entity::{host, plugin_config, proxmox_host_mapping};
  ```

  Change to:

  ```rust
  use uptrakit_shared_db::entity::{host, plugin_config};
  use crate::entity::proxmox_host_mapping;
  ```

- Lines 924 and 978 currently read:

  ```rust
  use uptrakit_shared_db::entity::proxmox_host_mapping;
  ```

  Change to:

  ```rust
  use crate::entity::proxmox_host_mapping;
  ```

- Line 1970 (return type of `mock_proxmox_host_mapping`) currently:

  ```rust
  ) -> uptrakit_shared_db::entity::proxmox_host_mapping::Model {
  ```

  Change to:

  ```rust
  ) -> crate::entity::proxmox_host_mapping::Model {
  ```

- Line 1971 currently inside the helper body:

  ```rust
  use uptrakit_shared_db::entity::proxmox_host_mapping;
  ```

  Change to:

  ```rust
  use crate::entity::proxmox_host_mapping;
  ```

After editing, run `git grep -n
"uptrakit_shared_db::entity::proxmox_host_mapping"
crates/plugins/infrastructure/proxmox/src/surfaces.rs`. Expected: zero hits.

- [ ] **Step 2: Update `reset.rs`**

Open `crates/plugins/infrastructure/proxmox/src/reset.rs`. Line 23:

```rust
use uptrakit_shared_db::entity::proxmox_host_mapping;
```

Change to:

```rust
use crate::entity::proxmox_host_mapping;
```

- [ ] **Step 3: Update `discovery.rs`**

Open `crates/plugins/infrastructure/proxmox/src/discovery.rs`. Line 260:

```rust
use uptrakit_shared_db::entity::proxmox_host_mapping;
```

Change to:

```rust
use crate::entity::proxmox_host_mapping;
```

- [ ] **Step 4: Update `matching.rs`**

Open `crates/plugins/infrastructure/proxmox/src/matching.rs`. Line 15 imports both `host` and `proxmox_host_mapping`:

```rust
use uptrakit_shared_db::entity::{host, proxmox_host_mapping};
```

Change to:

```rust
use uptrakit_shared_db::entity::host;
use crate::entity::proxmox_host_mapping;
```

- [ ] **Step 5: Update `protection_store.rs`**

Open `crates/plugins/infrastructure/proxmox/src/protection_store.rs`. Line 22 currently:

```rust
use uptrakit_shared_db::entity::{plugin_config, prelude::*, proxmox_host_mapping};
```

The `prelude::*` glob brings `ProxmoxHostMapping` into scope. After Phase B that re-export is gone. Two changes:

- Replace the existing line with:

  ```rust
  use uptrakit_shared_db::entity::{plugin_config, prelude::*};
  use crate::entity::proxmox_host_mapping;
  use crate::entity::proxmox_host_mapping::Entity as ProxmoxHostMapping;
  ```

- Verify line 159 still references `ProxmoxHostMapping::find()` — the local alias keeps it valid.

- [ ] **Step 6: Verify there are no other plugin-internal references**

Run: `git grep -n "uptrakit_shared_db::entity::proxmox_host_mapping" crates/plugins/infrastructure/proxmox/`
Expected: zero hits.

Also run: `git grep -n "proxmox_host_mapping::Model" crates/plugins/infrastructure/proxmox/`
Expected: only `crate::entity::proxmox_host_mapping::Model` (and shorter
`proxmox_host_mapping::Model` in test fixtures referencing the local
`crate::entity` import). No
`uptrakit_shared_db::entity::proxmox_host_mapping::Model` remaining.

- [ ] **Step 7: Do NOT commit yet — proceed to Task B4**

### Task B4: Add Proxmox `db_migrate.rs` registration

**Files:**

- Create: `crates/plugins/infrastructure/proxmox/src/db_migrate.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/lib.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs`

- [ ] **Step 1: Create the `db_migrate.rs` module**

Create `crates/plugins/infrastructure/proxmox/src/db_migrate.rs`:

```rust
//! Plugin-owned tables registered for the `db-migrate` subcommand.
//!
//! Order is FK-safe: parents before children. The only inter-plugin-table
//! FK in the codebase today is
//! `proxmox_protection_audit.mapping_id → proxmox_host_mappings.id`
//! (`SetNull`); `host_mapping` therefore precedes `protection_audit`.
//! The other three Proxmox tables FK only into core tables, so their
//! relative position within this list does not matter for FK safety.

#[cfg(feature = "migrations")]
pub(crate) fn proxmox_db_migrate_tables(
) -> Vec<uptrakit_plugin_infrastructure_core::PluginTableDescriptor> {
    use uptrakit_plugin_infrastructure_core::PluginTableDescriptor;

    use crate::entity::{
        proxmox_backup_target_cache, proxmox_host_mapping, proxmox_protection_audit,
        proxmox_protection_default, proxmox_protection_item_override,
    };

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

/// No-op stub used when `migrations` feature is inactive.
#[cfg(not(feature = "migrations"))]
#[allow(dead_code)]
pub(crate) fn proxmox_db_migrate_tables() {}
```

- [ ] **Step 2: Register the module in `lib.rs`**

Open `crates/plugins/infrastructure/proxmox/src/lib.rs`. Add near the other module declarations:

```rust
pub(crate) mod db_migrate;
```

(no `#[cfg]` gate — the module itself contains the gate so the stub function is always defined).

- [ ] **Step 3: Wire into `declare_plugin!`**

Open `crates/plugins/infrastructure/proxmox/src/plugin.rs`. Locate the
`declare_plugin!` invocation at line 891. Append a new line after
`reset_tenant_data:` (currently last line before `});`):

Before:

```rust
    migrations: __proxmox_migrations,
    reset_tenant_data: crate::reset::proxmox_reset_tenant_data,
});
```

After:

```rust
    migrations: __proxmox_migrations,
    reset_tenant_data: crate::reset::proxmox_reset_tenant_data,
    db_migrate_tables: crate::db_migrate::proxmox_db_migrate_tables,
});
```

- [ ] **Step 4: Verify table-name strings match the entity attributes**

Run:

```bash
grep -rn "table_name" crates/plugins/infrastructure/proxmox/src/entity/*.rs
```

Expected output (table-name strings that must match `proxmox_db_migrate_tables`):

```text
proxmox_backup_target_cache.rs:..."proxmox_backup_target_cache"
proxmox_host_mapping.rs:..."proxmox_host_mappings"
proxmox_protection_audit.rs:..."proxmox_protection_audit"
proxmox_protection_default.rs:..."proxmox_protection_defaults"
proxmox_protection_item_override.rs:..."proxmox_protection_item_overrides"
```

Cross-check against the literals in `db_migrate.rs`. If any string disagrees, fix in `db_migrate.rs`.

- [ ] **Step 5: Do NOT commit yet — proceed to B5**

### Task B5: Add `#[non_exhaustive]` to `DbMigrateError` and the `ReportConversion` impl

**Files:**

- Modify: `crates/core/controller-runtime/src/db_migrate/error.rs`

- [ ] **Step 1: Add `#[non_exhaustive]` to the enum**

Open `crates/core/controller-runtime/src/db_migrate/error.rs`. Locate the
`DbMigrateError` enum (currently `#[derive(Debug, Error)] pub(crate) enum
DbMigrateError {`). Add `#[non_exhaustive]` immediately above the existing
attributes:

```rust
#[non_exhaustive]
#[derive(Debug, Error)]
pub(crate) enum DbMigrateError {
    // ... existing variants unchanged ...
}
```

- [ ] **Step 2: Add the `impl_report_conversion!` block**

At the bottom of the file (after the `pub(crate) type Result<T> = ...` alias), add:

```rust
use uptrakit_shared_db::migrate_core_tables::TableMigrateError;
use uptrakit_shared_macros::impl_report_conversion;

// Folds `TableMigrateError` (returned by both registry and shared-db core
// helpers) into the existing `DbMigrateError::TableOp` and
// `DbMigrateError::Mismatch` variants — no new variants needed.
// `TableMigrateError` is `#[non_exhaustive]` and lives in another crate
// (`shared-db`), so the closure's match must include a wildcard arm.
// The wildcard maps unknown variants conservatively to `TableOp` with a
// `DbErr::Custom` carrying the Debug rendering — guaranteed to fire only
// when `shared-db` adds a new variant we have not yet handled here.
impl_report_conversion!(TableMigrateError => DbMigrateError, |e| match e {
    TableMigrateError::Db { table, err } => {
        DbMigrateError::TableOp { table, db_err: err }
    }
    TableMigrateError::Mismatch { table, src, dst } => {
        DbMigrateError::Mismatch { table, src, dst }
    }
    other => DbMigrateError::TableOp {
        table: "<unknown>",
        db_err: sea_orm::DbErr::Custom(format!("{other:?}")),
    },
});
```

- [ ] **Step 3: Verify `controller-runtime/Cargo.toml` already has `uptrakit-shared-macros`**

Run: `grep "uptrakit-shared-macros" crates/core/controller-runtime/Cargo.toml`
Expected: at least one match. If missing, add `uptrakit-shared-macros = { workspace = true }` under `[dependencies]`.

Also verify `uptrakit-shared-db` is already there with the `migration`
feature. The new `impl_report_conversion!` line uses `TableMigrateError` from
`migrate_core_tables`, which requires the `db-migrate` feature. Update the
feature list:

```toml
uptrakit-shared-db = { workspace = true, features = ["migration", "db-migrate"] }
```

- [ ] **Step 4: Do NOT commit yet — proceed to B6**

### Task B6: Strip plugin macros from `tables.rs` and call registry helpers

**Files:**

- Modify: `crates/core/controller-runtime/src/db_migrate/tables.rs`

- [ ] **Step 1: Remove `proxmox_host_mappings` from `COPY_ORDER`**

Open `crates/core/controller-runtime/src/db_migrate/tables.rs`. At line 67, delete:

```rust
    "proxmox_host_mappings",
```

- [ ] **Step 2: Remove the proxmox `copy!` invocation**

At line 137, delete:

```rust
    copy!(ProxmoxHostMapping, "proxmox_host_mappings");
```

- [ ] **Step 3: Remove the proxmox `clean!` invocation**

At line 154, delete:

```rust
    clean!(ProxmoxHostMapping, "proxmox_host_mappings");
```

- [ ] **Step 4: Remove the proxmox `verify!` invocation**

At line 274, delete:

```rust
    verify!(ProxmoxHostMapping, "proxmox_host_mappings");
```

- [ ] **Step 5: Insert registry-helper calls**

In `copy_all`, at the very end (after the last `copy!` macro invocation, before `Ok(total)`):

```rust
    total += uptrakit_plugin_infrastructure_registry::copy_plugin_tables(src, dst, batch_size)
        .await
        .context_to()?;
```

In `clean_all`, at the very start (before the first `clean!` macro invocation):

```rust
    uptrakit_plugin_infrastructure_registry::clean_plugin_tables(dst)
        .await
        .context_to()?;
```

In `verify_all`, at the very end (after the last `verify!` macro invocation, before `Ok(total)`):

```rust
    total += uptrakit_plugin_infrastructure_registry::verify_plugin_tables(src, dst)
        .await
        .context_to()?;
```

The `.context_to()?` call requires the `ResultExt` trait — the existing `use rootcause::prelude::*;` at line 1 provides it.

- [ ] **Step 6: Update the `COPY_ORDER` length assertion**

Locate the `#[cfg(test)] mod tests { ... }` block at line 399. Update the literal:

```rust
#[test]
fn copy_order_has_all_tables() {
    assert_eq!(
        COPY_ORDER.len(),
        48,
        "COPY_ORDER must list all 48 core app tables; plugin tables register via PluginDescriptor"
    );
}
```

(Phase D replaces this test entirely with a structural check.)

- [ ] **Step 7: Verify compilation**

Run:

```bash
cargo check -p uptrakit-controller --all-features
cargo check -p uptrakit-controller --no-default-features --features db-sqlite
```

Expected: clean.

- [ ] **Step 8: Do NOT commit yet — proceed to B7**

### Task B7: Verify Phase B end-to-end with the existing integration test

**Files:** none (verification only).

- [ ] **Step 1: Run the workspace quality gate**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

Expected: every command exits 0.

- [ ] **Step 2: Run the `db-migrate` integration test**

```bash
cargo test -p uptrakit-controller db_migrate -- --ignored
```

Expected: PASS. This exercises copy + verify of every table including the now-plugin-owned proxmox tables.

- [ ] **Step 3: Confirm boundary state**

Run:

```bash
git grep -n "ProxmoxHostMapping\|proxmox_host_mappings" crates/core/controller-runtime/src/db_migrate/tables.rs
```

Expected: zero hits.

```bash
git grep -n "proxmox_host_mapping\|ProxmoxHostMapping" crates/shared/db/
```

Expected: zero hits.

```bash
git grep -n "uptrakit_shared_db::entity::proxmox_host_mapping" crates/plugins/infrastructure/proxmox/
```

Expected: zero hits.

### Task B8: Phase B atomic commit

**Files:** all files modified across B1–B7.

- [ ] **Step 1: Stage everything**

```bash
git add \
  crates/plugins/infrastructure/proxmox/src/entity/proxmox_host_mapping.rs \
  crates/plugins/infrastructure/proxmox/src/entity/mod.rs \
  crates/shared/db/src/entity/mod.rs \
  crates/shared/db/src/entity/prelude.rs \
  crates/shared/db/src/entity/tenant_scoped.rs \
  crates/plugins/infrastructure/proxmox/src/surfaces.rs \
  crates/plugins/infrastructure/proxmox/src/reset.rs \
  crates/plugins/infrastructure/proxmox/src/discovery.rs \
  crates/plugins/infrastructure/proxmox/src/matching.rs \
  crates/plugins/infrastructure/proxmox/src/protection_store.rs \
  crates/plugins/infrastructure/proxmox/src/db_migrate.rs \
  crates/plugins/infrastructure/proxmox/src/lib.rs \
  crates/plugins/infrastructure/proxmox/src/plugin.rs \
  crates/core/controller-runtime/src/db_migrate/error.rs \
  crates/core/controller-runtime/src/db_migrate/tables.rs \
  crates/core/controller-runtime/Cargo.toml
```

If `git status` shows the deleted
`crates/shared/db/src/entity/proxmox_host_mapping.rs`, also run `git add -u
crates/shared/db/src/entity/proxmox_host_mapping.rs` to record the deletion.

- [ ] **Step 2: Verify nothing else is staged**

Run: `git status`
Expected: only the listed files. If anything unexpected appears (e.g., editor cruft), unstage it.

- [ ] **Step 3: Commit**

```bash
git commit -m "$(cat <<'EOF'
refactor(plugin-boundary): move proxmox_host_mapping + flip db-migrate dispatch

Atomic move of the last Proxmox entity out of shared-db, plus the dispatch
flip from macro-named tables.rs invocations to registry helpers.

- proxmox_host_mapping entity moves to plugins/infrastructure/proxmox/src/entity/
- shared-db loses proxmox_host_mapping mod / prelude re-export / TenantScoped impl
- Proxmox plugin registers db_migrate_tables (5 entities, FK-safe order)
- controller-runtime/db_migrate/tables.rs replaces ProxmoxHostMapping macro lines
  with copy/clean/verify_plugin_tables registry calls
- DbMigrateError gains #[non_exhaustive] and impl_report_conversion! to fold
  TableMigrateError into existing TableOp + Mismatch variants

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Phase D — Audit Hardening + Script Consolidation

**Objective:** Add the new Python rule that detects plugin-owned entity files
and re-exports inside `shared-db`, replace the brittle `COPY_ORDER` length
assertion with a schema-driven structural completeness test, and delete the
legacy shell checker.

### Task D1: Add `RULE_PLUGIN_ENTITY_IN_SHARED_DB` to the Python checker

**Files:**

- Modify: `ci/check_plugin_semantic_boundary.py`

- [ ] **Step 1: Read the existing rule registration**

Run: `sed -n '25,60p' ci/check_plugin_semantic_boundary.py`
Note the `RULE_*` constants, `KNOWN_RULE_IDS` set, and `RULE_MATCH_KINDS` dict (lines 25–56).

- [ ] **Step 2: Register the new rule constant + kinds**

Edit `ci/check_plugin_semantic_boundary.py`. After the existing `RULE_PLUGIN_TRANSPORT_ESCAPE` line (around line 31), add:

```python
RULE_PLUGIN_ENTITY_IN_SHARED_DB = "plugin-entity-in-shared-db"
```

In `KNOWN_RULE_IDS`, add `RULE_PLUGIN_ENTITY_IN_SHARED_DB,` to the set.

In `RULE_MATCH_KINDS`, add an entry:

```python
RULE_PLUGIN_ENTITY_IN_SHARED_DB: {"file_path", "module_token"},
```

- [ ] **Step 3: Add the dynamic-discovery helper**

Find a section near the existing `_is_*` / file-classification helpers (around `looks_like_production_code`). Add a new function:

```python
def _plugin_owned_entity_stems(repo_root: Path) -> frozenset[str]:
    """Return entity-file stems owned by plugins.

    Scans every `crates/plugins/**/src/entity/*.rs` file and returns
    the set of stems. These stems must not appear under
    `crates/shared/db/src/entity/`.
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

- [ ] **Step 4: Add the rule scanner**

Find the section that collects `Finding` results from per-rule scanners
(likely a `scan_plugin_entities_in_shared_db` would slot near other `scan_*`
functions, or be invoked from a top-level `run_all_rules` / `main` function).
Add:

```python
def scan_plugin_entity_leaks_in_shared_db(repo_root: Path) -> list[Finding]:
    """Detect plugin-owned entities re-exported by or hosted in shared-db.

    Two complementary checks:
      (a) filename collision — any `crates/shared/db/src/entity/<stem>.rs`
          whose stem matches a stem in `crates/plugins/**/src/entity/`.
      (b) module-token re-export — `pub mod <stem>;` or
          `pub use super::<stem>::...;` inside
          `crates/shared/db/src/entity/{mod,prelude}.rs` matching a
          plugin-owned stem.
    """
    findings: list[Finding] = []
    plugin_stems = _plugin_owned_entity_stems(repo_root)
    if not plugin_stems:
        return findings  # No plugins yet; nothing to check.

    shared_entity_dir = repo_root / "crates" / "shared" / "db" / "src" / "entity"
    skip = {"mod.rs", "prelude.rs", "tenant_scoped.rs"}

    # Check (a): filename collisions.
    for path in shared_entity_dir.glob("*.rs"):
        if path.name in skip:
            continue
        if path.stem in plugin_stems:
            rel = posix_rel(path, repo_root)
            findings.append(
                Finding(
                    rule_id=RULE_PLUGIN_ENTITY_IN_SHARED_DB,
                    path=rel,
                    line=1,
                    match_kind="file_path",
                    match_value=path.stem,
                    excerpt=f"plugin-owned entity stem `{path.stem}` hosted in shared-db",
                )
            )

    # Check (b): module-token re-exports inside mod.rs / prelude.rs.
    pub_mod_re = re.compile(r"^\s*pub\s+mod\s+([A-Za-z0-9_]+)\s*;")
    pub_use_re = re.compile(r"^\s*pub\s+use\s+super::([A-Za-z0-9_]+)\s*::")
    for filename in ("mod.rs", "prelude.rs"):
        path = shared_entity_dir / filename
        if not path.is_file():
            continue
        rel = posix_rel(path, repo_root)
        for line_no, line in enumerate(path.read_text().splitlines(), start=1):
            for pattern in (pub_mod_re, pub_use_re):
                m = pattern.match(line)
                if m and m.group(1) in plugin_stems:
                    findings.append(
                        Finding(
                            rule_id=RULE_PLUGIN_ENTITY_IN_SHARED_DB,
                            path=rel,
                            line=line_no,
                            match_kind="module_token",
                            match_value=m.group(1),
                            excerpt=line.rstrip(),
                        )
                    )
    return findings
```

- [ ] **Step 5: Wire the scanner into the main run**

Locate the function that aggregates findings for the run (commonly `main()` or
`run_checks()`). Append a call to
`scan_plugin_entity_leaks_in_shared_db(repo_root)` and extend the findings
list with its return value.

- [ ] **Step 6: Run the checker against the current repo**

```bash
python3 ci/check_plugin_semantic_boundary.py
```

Expected: exits 0 (after Phase B, no plugin entity hosted in shared-db; no
plugin re-export in mod/prelude). If it fails with
`RULE_PLUGIN_ENTITY_IN_SHARED_DB`, Phase B is incomplete — go back and finish.

- [ ] **Step 7: Test the rule fires on a fixture (filename collision)**

Verify the working tree is clean first to avoid mixing fixture cleanup
with real edits:

```bash
git status --porcelain
```

Expected: empty (or only the staged D1 changes).

Create the fixture, run the checker, then immediately restore:

```bash
touch crates/shared/db/src/entity/proxmox_host_mapping.rs
echo "// fixture" > crates/shared/db/src/entity/proxmox_host_mapping.rs
python3 ci/check_plugin_semantic_boundary.py || echo "Checker fired as expected"
rm crates/shared/db/src/entity/proxmox_host_mapping.rs
git status --porcelain crates/shared/db/src/entity/proxmox_host_mapping.rs
```

Expected: checker exits non-zero, reports
`RULE_PLUGIN_ENTITY_IN_SHARED_DB` with `match_kind="file_path"`. Final
`git status --porcelain` line is empty (file fully removed, no stray
new file in the index).

- [ ] **Step 8: Test the module-token check fires**

Same discipline. Verify the working tree is clean first:

```bash
git status --porcelain crates/shared/db/src/entity/prelude.rs
```

Expected: empty.

Append a fixture line, run the checker, restore via `git checkout`:

```bash
echo 'pub use super::proxmox_host_mapping::Foo;' >> crates/shared/db/src/entity/prelude.rs
python3 ci/check_plugin_semantic_boundary.py || echo "Checker fired as expected"
git checkout -- crates/shared/db/src/entity/prelude.rs
git status --porcelain crates/shared/db/src/entity/prelude.rs
```

Expected: checker exits non-zero, reports
`RULE_PLUGIN_ENTITY_IN_SHARED_DB` with `match_kind="module_token"`.
Final `git status --porcelain` line is empty.

- [ ] **Step 9: Commit**

```bash
git add ci/check_plugin_semantic_boundary.py
git commit -m "feat(ci): add RULE_PLUGIN_ENTITY_IN_SHARED_DB to boundary checker"
```

### Task D2: Replace `COPY_ORDER` length assertion with structural completeness test

**Files:**

- Modify: `crates/core/controller-runtime/src/db_migrate/tables.rs`

- [ ] **Step 1: Locate the existing test**

Run: `sed -n '399,412p' crates/core/controller-runtime/src/db_migrate/tables.rs`
Note the `#[cfg(test)] mod tests { ... }` block.

- [ ] **Step 2: Replace the test body**

Open the file. Replace the existing `mod tests { ... }` block with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Schema-driven completeness check.
    ///
    /// Every live application table (after running migrations) must be
    /// covered by either `COPY_ORDER` (core tables) or a registered
    /// plugin's `db_migrate_tables` entry.
    ///
    /// Failure modes caught:
    /// - New entity migration without registering the table for db-migrate.
    /// - Stale entry in `COPY_ORDER` or a plugin descriptor pointing at a
    ///   dropped table.
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

        let mut covered: HashSet<String> = COPY_ORDER
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
}
```

The old `copy_order_has_all_tables` test (asserting length 48) is removed — `migration_coverage_complete` subsumes it.

- [ ] **Step 3: Verify the test compiles**

```bash
cargo test -p uptrakit-controller --all-features --no-run
```

Expected: clean.

- [ ] **Step 4: Run the test**

```bash
cargo test -p uptrakit-controller migration_coverage_complete -- --ignored
```

Expected: PASS. If it fails with non-empty `missing`, an entity exists in
migrations but is not registered. If `extra` is non-empty, `COPY_ORDER` or a
plugin descriptor has a stale entry.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/db_migrate/tables.rs
git commit -m "test(db-migrate): replace length assertion with structural completeness check"
```

### Task D3: Verify each shell rule has Python equivalent (audit only)

**Files:** none (verification only).

- [ ] **Step 1: List shell rules**

Run: `grep -E "deny_in|deny_plugin_ids" ci/check_plugin_semantic_boundary.sh`

Expected output (5 rules):

- `dashboard-icons bespoke surface` (`settings_dashboard_icons|dashboard_icons\.enabled`)
- `PluginTypeId semantic helper callsites/uses`
- `PluginTypeId semantic helper definitions`
- `identity-specific helpers`
- `plugin_ids token references in non-plugin production code`

- [ ] **Step 2: Confirm each shell rule has a Python equivalent**

For each shell pattern, verify the Python checker fires on the same input.
Quickest way: introduce a single fixture line that should trigger the rule and
observe the Python checker.

Fixture script (run each fragment manually, restore the file after each):

a) `dashboard-icons bespoke surface` →

```bash
git stash --include-untracked
cat >> crates/ui/web-api/src/router.rs <<'EOF'
const _: &str = "settings_dashboard_icons";
EOF
python3 ci/check_plugin_semantic_boundary.py || echo "Python rule fired"
git checkout -- crates/ui/web-api/src/router.rs
```

Expected: Python checker reports the violation (rule should be `RULE_LEGACY_DASHBOARD_BESPOKE_SURFACE`).

b–e) Similar one-line fixtures targeting each shell pattern. Each must trigger the Python checker.

If any shell pattern is missing in Python, the implementation plan adds it
before deletion. (Current expectation per spec: all five have Python
equivalents.)

- [ ] **Step 3: Document outcome**

Note in the next commit message which shell rules verified to have Python
equivalents. If any rule needs porting, do it as a separate sub-task before
D4.

### Task D4: Delete the shell checker

**Files:**

- Delete: `ci/check_plugin_semantic_boundary.sh`
- Modify (verify only): `.github/workflows/ci.yml`
- Modify (if referenced): `docs/development/plugin-guidelines.md`, `docs/development/coding-standards.md`

- [ ] **Step 1: Search for shell-checker references in CI / docs**

```bash
git grep -n "check_plugin_semantic_boundary.sh" -- .github/ docs/
```

Note every hit.

- [ ] **Step 2: Verify CI does not call the shell script**

Open `.github/workflows/ci.yml`. The predecessor's Wave 7 proposed
adding the shell script to CI but the spec consolidates that step away.
If a CI step calls the shell script, delete that step. Keep the existing
Python checker step (line 57 of `ci.yml` per spec).

- [ ] **Step 3: Update docs that reference the shell checker**

For each doc match from Step 1, replace the reference with
`python3 ci/check_plugin_semantic_boundary.py` or remove the mention if
redundant.

- [ ] **Step 3b: Document the plugin-prefix entity naming convention**

Open `docs/development/plugin-guidelines.md`. Find a section about
plugin entities or DB tables (or add one if absent). Append:

```markdown
### Plugin entity file naming

Plugin-owned SeaORM entity files in `crates/plugins/<family>/<name>/src/entity/`
must use a stem that does not collide with any core entity file in
`crates/shared/db/src/entity/`. Convention: prefix with the plugin
family or name (e.g. `proxmox_host_mappings.rs`, `docker_image.rs`).
The boundary checker (`ci/check_plugin_semantic_boundary.py` rule
`RULE_PLUGIN_ENTITY_IN_SHARED_DB`) auto-discovers plugin entity stems
and fires when shared-db hosts a file or re-export with the same stem.
A collision is interpreted as the plugin entity being mistakenly hosted
in shared-db; the resolution is to rename the plugin entity, not to
weaken the rule.
```

If `plugin-guidelines.md` already covers naming, integrate the section
above into the existing structure rather than duplicating headings.

- [ ] **Step 4: Delete the shell file**

```bash
rm ci/check_plugin_semantic_boundary.sh
```

- [ ] **Step 5: Verify nothing else references the deleted file**

```bash
git grep -n "check_plugin_semantic_boundary.sh"
```

Expected: zero hits.

- [ ] **Step 6: Run the Python checker once more**

```bash
python3 ci/check_plugin_semantic_boundary.py
```

Expected: exits 0.

- [ ] **Step 7: Commit**

```bash
git add ci/check_plugin_semantic_boundary.sh .github/workflows/ci.yml docs/development/
# (only stage docs files actually modified)
git commit -m "chore(ci): consolidate boundary checker — delete legacy shell script"
```

### Task D5: Phase D acceptance gate

**Files:** none (verification only).

- [ ] **Step 1: Full quality gate**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

Expected: every command exits 0.

- [ ] **Step 2: Boundary checker green**

```bash
python3 ci/check_plugin_semantic_boundary.py
```

Expected: exits 0.

- [ ] **Step 3: Structural test passes**

```bash
cargo test -p uptrakit-controller migration_coverage_complete -- --ignored
```

Expected: PASS.

- [ ] **Step 4: Markdownlint clean**

```bash
npx markdownlint --config .markdownlint.json '**/*.md'
```

Expected: clean.

---

## Phase C — Move Core Table List to `shared-db`

**Objective:** Move `COPY_ORDER`, `copy_all`/`clean_all`/`verify_all`
core-table logic, and the generic per-table helpers from
`controller-runtime/db_migrate/tables.rs` into a new
`shared-db::migrate_core_tables` module. `tables.rs` shrinks to a thin
orchestrator (≤ 50 LoC excluding tests). The `TableMigrateError` enum
(added in Phase A) becomes the shared return type for both core and
plugin paths.

### Task C1: Extend `migrate_core_tables` with `copy` / `clean` / `verify` + `CORE_COPY_ORDER`

**Files:**

- Modify: `crates/shared/db/src/migrate_core_tables.rs`

- [ ] **Step 1: Read the current state of the module**

Run: `cat crates/shared/db/src/migrate_core_tables.rs`
Confirm only `TableMigrateError` and the `Result` alias are present.

- [ ] **Step 2: Add the per-entity generic helpers**

Append to the file:

```rust
// ── Per-table generic helpers (moved from controller-runtime/db_migrate/tables.rs) ─

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, DatabaseConnection, DbErr, EntityTrait,
    IntoActiveModel, PaginatorTrait, QuerySelect,
};

async fn migrate_table<E>(
    name: &'static str,
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64>
where
    E: EntityTrait + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Send + Sync + 'static,
    E::ActiveModel: ActiveModelTrait<Entity = E> + ActiveModelBehavior + Send + 'static,
{
    let total = E::find()
        .count(src)
        .await
        .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;

    let mut copied = 0u64;
    let mut offset = 0u64;
    loop {
        let batch = E::find()
            .offset(offset)
            .limit(batch_size)
            .all(src)
            .await
            .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;
        if batch.is_empty() {
            break;
        }
        let n = batch.len() as u64;
        let active: Vec<_> = batch
            .into_iter()
            .map(IntoActiveModel::into_active_model)
            .collect();
        E::insert_many(active)
            .exec(dst)
            .await
            .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;
        copied += n;
        offset += n;
        eprintln!("  {name}: {copied}/{total} rows");
    }
    if total == 0 {
        eprintln!("  {name}: 0 rows (empty)");
    }
    Ok(copied)
}

async fn clean_table<E: EntityTrait>(
    name: &'static str,
    dst: &DatabaseConnection,
) -> Result<()> {
    E::delete_many()
        .exec(dst)
        .await
        .map(|_| ())
        .map_err(|err| report!(TableMigrateError::Db { table: name, err }))
}

async fn verify_table<E>(
    name: &'static str,
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
) -> Result<u64>
where
    E: EntityTrait + 'static,
    E::Model: Send + Sync + 'static,
{
    let src_count = E::find()
        .count(src)
        .await
        .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;
    let dst_count = E::find()
        .count(dst)
        .await
        .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;
    if src_count != dst_count {
        bail!(TableMigrateError::Mismatch {
            table: name,
            src: src_count,
            dst: dst_count,
        });
    }
    Ok(src_count)
}
```

- [ ] **Step 3: Add `CORE_COPY_ORDER`**

Append to the file:

```rust
/// FK-safe order of all **core** application tables (no plugin tables).
///
/// Used by [`copy`] / [`clean`] / [`verify`] and by the
/// `migration_coverage_complete` integration test in
/// `controller-runtime`.
pub const CORE_COPY_ORDER: &[&str] = &[
    // Copy verbatim from the existing `crates/core/controller-runtime/
    // src/db_migrate/tables.rs::COPY_ORDER`, EXCLUDING
    // "proxmox_host_mappings" (already removed in Phase B).
    //
    // 48 entries total.
];
```

(The actual 48 strings are filled in below as part of Step 4.)

- [ ] **Step 4: Populate `CORE_COPY_ORDER`**

Run: `sed -n '18,68p' crates/core/controller-runtime/src/db_migrate/tables.rs`
Copy every string literal between the brackets (currently 48 entries
after Phase B removed `proxmox_host_mappings`). Paste into the body of
`CORE_COPY_ORDER` in `migrate_core_tables.rs`. Preserve order exactly.

- [ ] **Step 5: Add `copy`, `clean`, `verify` boundary functions**

Append to the file:

```rust
/// Copy every core table from `src` to `dst`. Returns total rows.
pub async fn copy(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64> {
    use crate::entity::prelude::*;

    let mut total = 0u64;

    macro_rules! copy {
        ($entity:ty, $name:literal) => {
            total += migrate_table::<$entity>($name, src, dst, batch_size).await?;
        };
    }

    // Body: copy verbatim from controller-runtime's existing `copy_all`
    // (lines 73–139), with the `proxmox_host_mappings` line already
    // removed in Phase B. Each `copy!(EntityName, "table_name");` line
    // is preserved exactly. Replace the local `copy!` macro to delegate
    // to `migrate_table::<$entity>`.

    // ... 48 copy! invocations using crate::entity::prelude entries ...

    Ok(total)
}

/// Delete every core table on `dst` in reverse FK-safe order.
pub async fn clean(dst: &DatabaseConnection) -> Result<()> {
    use crate::entity::prelude::*;

    macro_rules! clean {
        ($entity:ty, $name:literal) => {
            clean_table::<$entity>($name, dst).await?;
        };
    }

    // Body: copy verbatim from `clean_all` (lines 147–208), minus the
    // `proxmox_host_mappings` line.

    Ok(())
}

/// Verify row counts match between `src` and `dst` for every core table.
/// Returns total rows verified.
pub async fn verify(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
) -> Result<u64> {
    use crate::entity::prelude::*;

    let mut total = 0u64;

    macro_rules! verify {
        ($entity:ty, $name:literal) => {
            total += verify_table::<$entity>($name, src, dst).await?;
        };
    }

    // Body: copy verbatim from `verify_all` (lines 214–277), minus the
    // `proxmox_host_mappings` line.

    Ok(total)
}
```

The `crate::entity::prelude::*` glob (within `shared-db`) provides every
entity type alias used by the macros. Each function (`copy`, `clean`,
`verify`) imports it locally at the top of its body.

- [ ] **Step 6: Verify the module compiles**

```bash
cargo check -p uptrakit-shared-db --features db-migrate
```

Expected: clean.

- [ ] **Step 7: Do not commit yet**

Wave C is one logical change — final commit at C4.

### Task C2: Reduce `controller-runtime/db_migrate/tables.rs` to a thin orchestrator

**Files:**

- Modify: `crates/core/controller-runtime/src/db_migrate/tables.rs`

- [ ] **Step 1: Replace the entire file body**

Open `crates/core/controller-runtime/src/db_migrate/tables.rs` and
replace the **entire** contents — including the `mod tests` block —
with the template below. The template re-emits the
`migration_coverage_complete` test verbatim from Phase D so the test is
preserved across the C2 rewrite. Do not try to merge the existing test
in by hand; the template already contains it.

```rust
//! Database data migration — orchestrator over core tables (in
//! `shared-db::migrate_core_tables`) and plugin tables (registered via
//! `PluginDescriptor::db_migrate_tables`).

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;

use super::error::{DbMigrateError, Result};

#[cfg(test)]
pub(crate) use uptrakit_shared_db::migrate_core_tables::CORE_COPY_ORDER as COPY_ORDER;

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

#[cfg(test)]
mod tests {
    // The previous `migration_coverage_complete` test (added in Phase D)
    // imports `super::COPY_ORDER`, which now re-exports
    // `shared-db::migrate_core_tables::CORE_COPY_ORDER`. The test body
    // is unchanged.

    use super::*;

    /// Schema-driven completeness check.
    ///
    /// Every live application table (after running migrations) must be
    /// covered by either `COPY_ORDER` (core tables) or a registered
    /// plugin's `db_migrate_tables` entry.
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

        let mut covered: HashSet<String> = COPY_ORDER
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
}
```

- [ ] **Step 2: Verify line count**

```bash
sed -n '/^#\[cfg(test)\]/,$d' crates/core/controller-runtime/src/db_migrate/tables.rs | wc -l
```

Expected: ≤ 50.

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p uptrakit-controller --all-features
cargo check -p uptrakit-controller --no-default-features --features db-sqlite
```

Expected: clean.

- [ ] **Step 4: Do not commit yet — proceed to C3**

### Task C3: Run the integration tests + full quality gate

**Files:** none (verification only).

- [ ] **Step 1: Full quality gate**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

Expected: every command exits 0.

- [ ] **Step 2: Run the integration tests**

```bash
cargo test -p uptrakit-controller db_migrate -- --ignored
```

Expected: `migrate_sqlite_to_sqlite_roundtrip` and `migration_coverage_complete` both PASS.

- [ ] **Step 3: Boundary checker still clean**

```bash
python3 ci/check_plugin_semantic_boundary.py
```

Expected: exits 0.

### Task C4: Phase C atomic commit

**Files:** all files modified across C1–C3.

- [ ] **Step 1: Stage**

```bash
git add \
  crates/shared/db/src/migrate_core_tables.rs \
  crates/core/controller-runtime/src/db_migrate/tables.rs \
  crates/core/controller-runtime/Cargo.toml
```

- [ ] **Step 2: Verify staged set**

```bash
git status
```

Expected: only the three files. `Cargo.toml` will appear if Phase B
already added the `db-migrate` feature flag activation; if it shows
additional changes here, double-check.

- [ ] **Step 3: Commit**

```bash
git commit -m "$(cat <<'EOF'
refactor(db-migrate): move core table list to shared-db, reduce orchestrator to ~50 LoC

Wave C of the boundary-hardening completion. Per-table copy/clean/verify
generic helpers and CORE_COPY_ORDER move from controller-runtime/db_migrate/
tables.rs into shared-db::migrate_core_tables (under db-migrate feature).
controller-runtime's tables.rs shrinks to a thin orchestrator that calls
shared-db::migrate_core_tables::{copy,clean,verify} and the registry's
{copy,clean,verify}_plugin_tables, mapping errors via .context_to()? on
the impl_report_conversion! defined alongside DbMigrateError.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Final Acceptance

After all four phases land:

- [ ] **All commands exit 0:**

```bash
cargo fmt --all -- --check
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo test -p uptrakit-controller db_migrate -- --ignored
cargo deny check
python3 ci/check_plugin_semantic_boundary.py
npx markdownlint --config .markdownlint.json '**/*.md'
```

- [ ] **Boundary state:**

```bash
git grep -n "ProxmoxHostMapping\|proxmox_host_mappings" crates/core/ crates/shared/db/
```

Expected: zero hits.

```bash
git grep -n "uptrakit_shared_db::entity::proxmox_host_mapping" crates/plugins/infrastructure/proxmox/
```

Expected: zero hits.

- [ ] **File deletions:**

```bash
test ! -e crates/shared/db/src/entity/proxmox_host_mapping.rs && echo OK
test ! -e ci/check_plugin_semantic_boundary.sh && echo OK
```

Expected: both `OK`.

- [ ] **Orchestrator size:**

```bash
sed -n '/^#\[cfg(test)\]/,$d' crates/core/controller-runtime/src/db_migrate/tables.rs | wc -l
```

Expected: ≤ 50.

---

## Notes

- **Predecessor audit grep gap.** The predecessor's Wave 6 grep used
  `| grep "use "` which silently skipped macro-style references. Future
  audits should drop that filter. This plan does not modify the
  predecessor spec; the gap is documented in the completion spec's
  Notes section.
- **Inter-plugin-table FK.** `proxmox_protection_audit.mapping_id →
proxmox_host_mappings.id` (`SetNull`) is the only inter-plugin-table
  FK in the codebase. The Proxmox plugin's `db_migrate_tables` Vec
  preserves FK order: `host_mapping` precedes `protection_audit`. The
  doc comment on `DbMigrateTablesFn` records this rule for future
  plugin authors.
- **Naming convention from Wave D.** The dynamic-discovery rule
  (`RULE_PLUGIN_ENTITY_IN_SHARED_DB`) makes plugin-prefix entity naming
  a hard requirement. Plugin entity stems must not collide with core
  entity names like `host`, `service`, `tag`. Document this in
  `docs/development/plugin-guidelines.md` if not already covered.
- **No plugin populates `db_migrate_tables` between Phase A and
  Phase B.** During this brief window the new infrastructure is dormant.
  CI stays green because the registry helpers iterate descriptors and
  skip `None` entries.
