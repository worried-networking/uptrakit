//! Shared helpers for SQLite table-recreation migrations.
//!
//! SQLite does not support `ALTER TABLE DROP COLUMN` in all cases, nor does it
//! support `ALTER TABLE ALTER COLUMN` at all. The standard workaround is the
//! **table recreation** pattern: create a new table with the desired schema,
//! copy data from the old table, drop the old table, and rename the new one.
//!
//! This module provides reusable building blocks for that pattern with built-in
//! crash recovery. See the [database migrations guide] for full documentation.
//!
//! [database migrations guide]: ../../../../docs/development/database-migrations.md
//!
//! ## When to use table recreation
//!
//! Use table recreation when:
//!
//! - Dropping a column (SQLite `ALTER TABLE DROP COLUMN` fails if the column
//!   is referenced by an index, FK, trigger, or view)
//! - Changing a column's type or nullability
//! - Adding a `GENERATED ALWAYS AS` (stored) column
//! - Restructuring a table's schema in ways `ALTER TABLE` cannot express
//!
//! For simple `ADD COLUMN` operations, use `ALTER TABLE` directly — no
//! recreation needed.
//!
//! ## Usage pattern
//!
//! ```rust,ignore
//! use crate::migration::helpers::{self, CrashRecoveryState};
//!
//! async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
//!     helpers::set_foreign_keys(manager, false).await?;
//!
//!     let state = helpers::check_crash_recovery(
//!         manager, "my_table", "my_table_new",
//!     ).await?;
//!
//!     if state == CrashRecoveryState::Normal {
//!         // 1. Create the new table with the desired schema.
//!         manager.create_table(build_new_schema()).await?;
//!
//!         // 2. Copy data (sea_query INSERT...SELECT or execute_unprepared for
//!         //    complex transformations like CASE expressions).
//!         copy_data(manager).await?;
//!
//!         // 3. Drop the old table (indexes are dropped implicitly).
//!         helpers::drop_original(manager, "my_table").await?;
//!     }
//!     // If state == RenameOnly, skip create/copy/drop — the temp table
//!     // already holds the complete dataset.
//!
//!     // 4. Rename the temp table to the canonical name.
//!     helpers::rename_temp(manager, "my_table_new", "my_table").await?;
//!
//!     // 5. Recreate indexes (they were dropped with the old table).
//!     create_indexes(manager).await?;
//!
//!     helpers::set_foreign_keys(manager, true).await?;
//!     Ok(())
//! }
//! ```

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;

/// State of a table recreation after a potential partial previous run.
///
/// The crash recovery model has three states:
///
/// - **State A** (`Normal`): Only the original table exists. The migration
///   should proceed with the full create → copy → drop → rename sequence.
///
/// - **State B** (recovered → `Normal`): Both the original and temp tables
///   exist. A previous run created the temp table but crashed before completing.
///   The original data is still intact. [`check_crash_recovery`] automatically
///   drops the partial temp table and returns `Normal`.
///
/// - **State C** (`RenameOnly`): Only the temp table exists. A previous run
///   copied all data and dropped the original, but crashed before the rename.
///   The migration should skip directly to the rename step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashRecoveryState {
    /// Only the original table exists (or both existed and the partial temp
    /// was automatically discarded). Proceed with the full recreation sequence.
    Normal,
    /// Only the temp table exists. Skip to the rename step.
    RenameOnly,
}

/// Suspend or resume FK enforcement on SQLite.
///
/// On PostgreSQL and MySQL the `PRAGMA` statement is not recognised; this
/// function is a no-op for those backends.
///
/// # When to use
///
/// Call `set_foreign_keys(manager, false)` at the **start** of a table
/// recreation migration and `set_foreign_keys(manager, true)` at the **end**.
/// This prevents FK constraint violations during the brief window when the
/// old table has been dropped but the replacement has not yet been renamed.
///
/// **Never use this to skip inserting required parent rows in tests or normal
/// migrations.** It is only appropriate during table recreation.
pub async fn set_foreign_keys(manager: &SchemaManager<'_>, enabled: bool) -> Result<(), DbErr> {
    if manager.get_database_backend() == DbBackend::Sqlite {
        let pragma = if enabled {
            "PRAGMA foreign_keys = ON"
        } else {
            "PRAGMA foreign_keys = OFF"
        };
        // `PRAGMA foreign_keys` is a SQLite-specific statement with no sea_query equivalent.
        manager.get_connection().execute_unprepared(pragma).await?;
    }
    Ok(())
}

/// Check the crash recovery state for a table recreation migration.
///
/// Inspects whether the original and/or temp tables exist and returns the
/// appropriate [`CrashRecoveryState`]. If both tables exist (State B), the
/// partial temp table is automatically dropped before returning `Normal`.
///
/// See [`CrashRecoveryState`] for details on the three-state model.
pub async fn check_crash_recovery(
    manager: &SchemaManager<'_>,
    table_name: &str,
    temp_table_name: &str,
) -> Result<CrashRecoveryState, DbErr> {
    let temp_exists = manager.has_table(temp_table_name).await?;
    let orig_exists = manager.has_table(table_name).await?;

    if temp_exists && orig_exists {
        // State B: discard the incomplete temp table, restart from scratch.
        manager
            .drop_table(Table::drop().table(Alias::new(temp_table_name)).to_owned())
            .await?;
        return Ok(CrashRecoveryState::Normal);
    }

    if !orig_exists && temp_exists {
        // State C: only the temp table exists; skip to rename.
        return Ok(CrashRecoveryState::RenameOnly);
    }

    // State A: normal path.
    Ok(CrashRecoveryState::Normal)
}

/// Drop the original table during a table recreation.
///
/// Call this after creating the temp table and copying data into it. The
/// original table's indexes are dropped implicitly by SQLite.
pub async fn drop_original(manager: &SchemaManager<'_>, table_name: &str) -> Result<(), DbErr> {
    manager
        .drop_table(Table::drop().table(Alias::new(table_name)).to_owned())
        .await
}

/// Rename the temp table to the canonical name after a table recreation.
///
/// Call this after [`drop_original`] (or directly when
/// [`check_crash_recovery`] returns [`CrashRecoveryState::RenameOnly`]).
pub async fn rename_temp(
    manager: &SchemaManager<'_>,
    temp_table_name: &str,
    canonical_table_name: &str,
) -> Result<(), DbErr> {
    manager
        .rename_table(
            Table::rename()
                .table(
                    Alias::new(temp_table_name),
                    Alias::new(canonical_table_name),
                )
                .to_owned(),
        )
        .await
}

/// Check whether the current database backend is SQLite.
///
/// Useful in migrations that need different paths for SQLite (table recreation)
/// vs. PostgreSQL/MySQL (`ALTER TABLE`).
pub fn is_sqlite(manager: &SchemaManager<'_>) -> bool {
    manager.get_database_backend() == DbBackend::Sqlite
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};

    async fn setup_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    #[tokio::test]
    async fn set_foreign_keys_toggle() {
        let db = setup_db().await;
        // SchemaManager is not easily constructed in tests without a migration
        // context; verify the underlying SQL works directly.
        db.execute_unprepared("PRAGMA foreign_keys = OFF")
            .await
            .expect("disable FKs");
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .expect("re-enable FKs");
    }

    #[tokio::test]
    async fn crash_recovery_state_a() {
        let db = setup_db().await;
        db.execute_unprepared("CREATE TABLE test_table (id INTEGER PRIMARY KEY)")
            .await
            .unwrap();

        let has_orig = db
            .execute_unprepared("SELECT 1 FROM test_table LIMIT 0")
            .await
            .is_ok();
        assert!(has_orig, "original table should exist");

        let has_temp = db
            .execute_unprepared("SELECT 1 FROM test_table_new LIMIT 0")
            .await
            .is_ok();
        assert!(!has_temp, "temp table should not exist");
    }
}
