//! Workspace-standard transaction opening.
//!
//! `begin_immediate()` is the only sanctioned way to open a database
//! transaction in this workspace: it opens `BEGIN IMMEDIATE` on SQLite to
//! prevent `SQLITE_BUSY_SNAPSHOT` on read-then-write transactions and is a
//! mode no-op on other backends and on nested (savepoint) transactions. All
//! other transaction-opening methods are banned via `clippy.toml`
//! `disallowed-methods`.

use sea_orm::{
    DatabaseTransaction, DbErr, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};

/// Opens a transaction with `BEGIN IMMEDIATE` on SQLite.
///
/// BEGIN IMMEDIATE prevents `SQLITE_BUSY_SNAPSHOT` on read-then-write
/// transactions (the write lock is taken at `BEGIN`, so `busy_timeout`
/// applies instead of an immediate snapshot error). On other backends and on
/// nested transactions (savepoints) the mode is silently ignored by SeaORM —
/// correct in both contexts.
pub async fn begin_immediate<C>(conn: &C) -> Result<DatabaseTransaction, DbErr>
where
    C: TransactionTrait<Transaction = DatabaseTransaction>,
{
    conn.begin_with_options(TransactionOptions {
        sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
        ..Default::default()
    })
    .await
}
