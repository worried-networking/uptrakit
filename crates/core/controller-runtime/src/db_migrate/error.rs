use rootcause::prelude::*;
use thiserror::Error;

/// Errors that can occur during a database migration.
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
