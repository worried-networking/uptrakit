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
