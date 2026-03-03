use sea_orm::{ConnectionTrait as _, DatabaseBackend, Statement, TransactionTrait as _};
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Repair SQLite databases where migrations m20260307_000002 and
/// m20260308_000001 inserted permission UUIDs as 36-character TEXT strings
/// instead of 16-byte BLOBs.
///
/// ## Background
///
/// SeaORM/sqlx reads `uuid`-typed columns via `sqlite3_column_blob()`.  When
/// a UUID is stored as TEXT (e.g. `'018f1234-…'`) rather than as a 16-byte
/// BLOB, the read fails with `ParseByteLength { len: 36 }`.
///
/// The root cause was that those migrations used
/// `execute_unprepared(&format!("INSERT … VALUES ('{id}', …)"))`, which
/// embeds the UUID as a SQL literal string instead of letting sea-query bind
/// it as a BLOB parameter.
///
/// ## What this migration does
///
/// For each `permissions` row whose `id` column has `typeof(id) = 'text'`:
///
/// 1. Parse the stored 36-char UUID string into its 16-byte binary form.
/// 2. Update `role_permissions.permission_id` TEXT→BLOB for that permission.
/// 3. Update `permissions.id` TEXT→BLOB for that permission.
///
/// All changes run inside a single transaction with FK enforcement temporarily
/// disabled (the updates break and then restore FK integrity atomically).
///
/// ## Scope
///
/// SQLite-only.  PostgreSQL and MySQL store UUIDs differently and are not
/// affected by this issue.
///
/// ## Down
///
/// A no-op: re-introducing TEXT storage would re-introduce the parsing bug.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // This fix is SQLite-only.
        if db.get_database_backend() != DatabaseBackend::Sqlite {
            return Ok(());
        }

        // Find all permissions whose id is stored as TEXT.
        //
        // `typeof()` is a SQLite-specific function with no sea_query equivalent;
        // using query_all_raw with a Statement is the approved exception for this
        // pattern.  See docs/development/database-migrations.md.
        let broken_rows = db
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT id, name FROM permissions WHERE typeof(id) = 'text'",
            ))
            .await?;

        if broken_rows.is_empty() {
            return Ok(());
        }

        // Collect (id_str, name) pairs before starting the transaction so we
        // don't hold query results across the txn boundary.
        let mut to_fix: Vec<(String, String)> = Vec::with_capacity(broken_rows.len());
        for row in &broken_rows {
            use sea_orm::TryGetable as _;
            // The TEXT-stored id comes back as a plain String, not a Uuid.
            let id_str: String = String::try_get_by_index(row, 0).map_err(|e| {
                DbErr::Custom(format!("failed to read permission id as string: {e:?}"))
            })?;
            let name: String = String::try_get_by_index(row, 1).map_err(|e| {
                DbErr::Custom(format!("failed to read permission name: {e:?}"))
            })?;
            to_fix.push((id_str, name));
        }

        // Disable FK enforcement before the transaction.
        //
        // Both `role_permissions.permission_id → permissions.id` and the
        // reverse parent-key-update check would fire mid-conversion.  We turn
        // FKs off, atomically fix all rows, then re-enable.  SQLite requires
        // the PRAGMA to be changed outside an active transaction.
        //
        // `PRAGMA foreign_keys` has no sea_query equivalent;
        // execute_unprepared is the approved exception for PRAGMA statements.
        db.execute_unprepared("PRAGMA foreign_keys = OFF").await?;

        let txn = db.begin().await?;

        for (id_str, name) in to_fix {
            let uuid = Uuid::parse_str(&id_str).map_err(|e| {
                DbErr::Custom(format!(
                    "repair migration: invalid UUID text '{id_str}' for permission '{name}': {e}"
                ))
            })?;
            let bytes: Vec<u8> = uuid.as_bytes().to_vec();

            // Fix any role_permissions rows that reference the TEXT uuid.
            txn.execute(
                &Query::update()
                    .table(Alias::new("role_permissions"))
                    .value(Alias::new("permission_id"), Value::Bytes(Some(bytes.clone())))
                    .and_where(
                        Expr::col(Alias::new("permission_id"))
                            .eq(Value::String(Some(id_str.clone()))),
                    )
                    .to_owned(),
            )
            .await?;

            // Fix the permissions row itself.
            txn.execute(
                &Query::update()
                    .table(Alias::new("permissions"))
                    .value(Alias::new("id"), Value::Bytes(Some(bytes)))
                    .and_where(Expr::col(Alias::new("name")).eq(name.as_str()))
                    .to_owned(),
            )
            .await?;
        }

        txn.commit().await?;

        // Re-enable FK enforcement now that all rows are consistent.
        db.execute_unprepared("PRAGMA foreign_keys = ON").await?;

        Ok(())
    }

    /// No-op: re-introducing TEXT storage would re-introduce the parsing bug.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
