use rootcause::prelude::*;
use thiserror::Error;

/// Errors that can occur during a database migration.
#[non_exhaustive]
#[derive(Debug, Error)]
pub(crate) enum DbMigrateError {
    /// Failed to connect to the source or target database, or a setup step
    /// (master key, URL validation) failed.
    #[error("database connection error: {0}")]
    Connection(String),

    /// Failed to run schema migrations on the target database.
    #[error("target database migration failed: {0}")]
    TargetMigration(String),

    /// Target database already contains user data; use `--force` to override.
    #[error("target database is not empty (use --force to override)")]
    TargetNotEmpty,

    /// User declined the confirmation prompt.
    #[error("migration aborted by user")]
    MigrationAborted,

    /// A database operation on a specific table failed.
    ///
    /// Used for read, write, delete, and count operations during migration.
    #[error("table `{table}` operation failed: {db_err}")]
    TableOp {
        table: &'static str,
        #[source]
        db_err: sea_orm::DbErr,
    },

    /// Row count mismatch between source and target after migration.
    #[error("row count mismatch for table `{table}`: source={src}, target={dst}")]
    Mismatch {
        table: &'static str,
        src: u64,
        dst: u64,
    },
}

pub(crate) type Result<T> = std::result::Result<T, Report<DbMigrateError>>;

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
