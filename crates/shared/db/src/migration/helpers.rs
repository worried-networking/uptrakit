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
///
/// **Important**: This helper is intended for the SQLite table recreation
/// pattern where FK enforcement is disabled via `set_foreign_keys(false)`.
/// On PostgreSQL/MySQL, prefer `ALTER TABLE` instead of table recreation
/// to avoid FK constraint issues. If table recreation is unavoidable on
/// PG/MySQL, the caller must handle dependent FK constraints manually.
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

/// Drop an index, ignoring "does not exist" errors.
///
/// Uses `IF EXISTS` on SQLite/PostgreSQL and raw `DROP INDEX` on MySQL
/// (sea-query panics on `IF EXISTS` for MySQL's `DROP INDEX`).
pub async fn drop_index_if_exists(
    manager: &SchemaManager<'_>,
    index_name: &str,
    table_name: &str,
) -> Result<(), DbErr> {
    if manager.get_database_backend() == DbBackend::MySql {
        // MySQL/MariaDB: DROP INDEX without IF EXISTS; ignore "doesn't exist".
        let sql = format!("DROP INDEX `{index_name}` ON `{table_name}`");
        match manager.get_connection().execute_unprepared(&sql).await {
            Ok(_) => Ok(()),
            Err(DbErr::Exec(ref e)) if e.to_string().contains("1091") => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        manager
            .drop_index(
                Index::drop()
                    .name(index_name)
                    .table(Alias::new(table_name))
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

/// Drop all foreign key constraints on a MySQL/MariaDB table.
///
/// Returns the FK metadata (constraint name, column, referenced table, referenced
/// column, delete rule) so they can be recreated later via
/// [`recreate_mysql_foreign_keys`].
///
/// This is a no-op on non-MySQL backends (returns an empty vec).
///
/// # Why
///
/// MariaDB/InnoDB implicitly uses user-created indexes as the backing index for
/// FK constraints. Attempting to drop such an index fails with error 1553
/// ("Cannot drop index: needed in a foreign key constraint"). The safest
/// workaround is to temporarily drop all FKs on the table, perform index
/// operations, then recreate the FKs.
pub async fn drop_mysql_foreign_keys(
    manager: &SchemaManager<'_>,
    table_name: &str,
) -> Result<Vec<FkInfo>, DbErr> {
    if manager.get_database_backend() != DbBackend::MySql {
        return Ok(vec![]);
    }

    let sql = format!(
        "SELECT kcu.CONSTRAINT_NAME, kcu.COLUMN_NAME, kcu.REFERENCED_TABLE_NAME, \
         kcu.REFERENCED_COLUMN_NAME, rc.DELETE_RULE \
         FROM information_schema.KEY_COLUMN_USAGE kcu \
         JOIN information_schema.REFERENTIAL_CONSTRAINTS rc \
           ON rc.CONSTRAINT_SCHEMA = kcu.CONSTRAINT_SCHEMA \
          AND rc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
         WHERE kcu.TABLE_SCHEMA = DATABASE() \
           AND kcu.TABLE_NAME = '{table_name}' \
           AND kcu.REFERENCED_TABLE_NAME IS NOT NULL"
    );

    let rows = manager
        .get_connection()
        .query_all_raw(sea_orm::Statement::from_string(DbBackend::MySql, sql))
        .await?;

    let mut fks = Vec::new();
    for row in &rows {
        let constraint_name: String = row.try_get("", "CONSTRAINT_NAME")?;
        let column_name: String = row.try_get("", "COLUMN_NAME")?;
        let ref_table: String = row.try_get("", "REFERENCED_TABLE_NAME")?;
        let ref_column: String = row.try_get("", "REFERENCED_COLUMN_NAME")?;
        let delete_rule: String = row.try_get("", "DELETE_RULE")?;

        fks.push(FkInfo {
            constraint_name,
            column_name,
            referenced_table: ref_table,
            referenced_column: ref_column,
            delete_rule,
        });
    }

    // Drop all found FKs.
    for fk in &fks {
        manager
            .drop_foreign_key(
                ForeignKey::drop()
                    .name(&fk.constraint_name)
                    .table(Alias::new(table_name))
                    .to_owned(),
            )
            .await?;
    }

    Ok(fks)
}

/// Recreate foreign keys previously dropped by [`drop_mysql_foreign_keys`].
///
/// No-op if `fks` is empty.
pub async fn recreate_mysql_foreign_keys(
    manager: &SchemaManager<'_>,
    table_name: &str,
    fks: &[FkInfo],
) -> Result<(), DbErr> {
    for fk in fks {
        let on_delete = match fk.delete_rule.as_str() {
            "CASCADE" => ForeignKeyAction::Cascade,
            "SET NULL" => ForeignKeyAction::SetNull,
            "SET DEFAULT" => ForeignKeyAction::SetDefault,
            "NO ACTION" | "RESTRICT" => ForeignKeyAction::Restrict,
            _ => ForeignKeyAction::Restrict,
        };

        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name(&fk.constraint_name)
                    .from(Alias::new(table_name), Alias::new(&fk.column_name))
                    .to(
                        Alias::new(&fk.referenced_table),
                        Alias::new(&fk.referenced_column),
                    )
                    .on_delete(on_delete)
                    .to_owned(),
            )
            .await?;
    }

    Ok(())
}

/// Foreign key metadata for temporary FK drop/recreate on MySQL.
#[derive(Debug, Clone)]
pub struct FkInfo {
    pub constraint_name: String,
    pub column_name: String,
    pub referenced_table: String,
    pub referenced_column: String,
    pub delete_rule: String,
}

/// Create a NOT NULL timestamp-with-time-zone column.
///
/// Shadows `sea_orm_migration::schema::timestamp()` so that all timestamp
/// columns are created as `TIMESTAMPTZ` on PostgreSQL, matching the entity
/// models that use `time::OffsetDateTime`.
pub fn timestamp<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col)
        .timestamp_with_time_zone()
        .not_null()
        .take()
}

/// Create a nullable timestamp-with-time-zone column.
///
/// Shadows `sea_orm_migration::schema::timestamp_null()` — see [`timestamp`].
pub fn timestamp_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).timestamp_with_time_zone().null().take()
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
