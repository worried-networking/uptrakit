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
/// 2. Collect the `role_id` values from any `role_permissions` rows that
///    reference the TEXT `permission_id`.
/// 3. Delete those `role_permissions` rows (the TEXT FK value will no longer
///    exist after step 4).
/// 4. Update `permissions.id` TEXT→BLOB.
/// 5. Re-insert the `role_permissions` rows with the corrected BLOB
///    `permission_id`.
///
/// This delete-fix-reinsert sequence never violates FK constraints at any
/// intermediate step, so it is safe to run inside the transaction that
/// sea-orm-migration implicitly wraps around every `up()` call.
/// (`PRAGMA foreign_keys = OFF` is silently ignored inside an active
/// transaction and must NOT be used here.)
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

        let txn = db.begin().await?;

        for (id_str, name) in to_fix {
            let uuid = Uuid::parse_str(&id_str).map_err(|e| {
                DbErr::Custom(format!(
                    "repair migration: invalid UUID text '{id_str}' for permission '{name}': {e}"
                ))
            })?;
            let blob = Value::Bytes(Some(uuid.as_bytes().to_vec()));

            // 1. Collect the role_ids that reference the TEXT permission_id so
            //    we can re-insert them after fixing the parent row.
            let role_rows = txn
                .query_all(
                    &Query::select()
                        .from(Alias::new("role_permissions"))
                        .column(Alias::new("role_id"))
                        .and_where(
                            Expr::col(Alias::new("permission_id"))
                                .eq(Value::String(Some(id_str.clone()))),
                        )
                        .to_owned(),
                )
                .await?;

            let mut role_ids: Vec<Vec<u8>> = Vec::with_capacity(role_rows.len());
            for row in &role_rows {
                use sea_orm::TryGetable as _;
                let role_id: Vec<u8> = Vec::<u8>::try_get_by_index(row, 0).map_err(|e| {
                    DbErr::Custom(format!("failed to read role_id as bytes: {e:?}"))
                })?;
                role_ids.push(role_id);
            }

            // 2. Delete child rows referencing the TEXT permission_id.
            //    After this the TEXT value is no longer referenced by any FK.
            txn.execute(
                &Query::delete()
                    .from_table(Alias::new("role_permissions"))
                    .and_where(
                        Expr::col(Alias::new("permission_id"))
                            .eq(Value::String(Some(id_str.clone()))),
                    )
                    .to_owned(),
            )
            .await?;

            // 3. Fix the parent row: TEXT id → BLOB id.
            txn.execute(
                &Query::update()
                    .table(Alias::new("permissions"))
                    .value(Alias::new("id"), blob.clone())
                    .and_where(Expr::col(Alias::new("name")).eq(name.as_str()))
                    .to_owned(),
            )
            .await?;

            // 4. Re-insert the child rows with the corrected BLOB permission_id.
            for role_id_bytes in role_ids {
                txn.execute(
                    &Query::insert()
                        .into_table(Alias::new("role_permissions"))
                        .columns([Alias::new("role_id"), Alias::new("permission_id")])
                        .values_panic([
                            Expr::val(Value::Bytes(Some(role_id_bytes))),
                            Expr::val(blob.clone()),
                        ])
                        .on_conflict(
                            OnConflict::columns([
                                Alias::new("role_id"),
                                Alias::new("permission_id"),
                            ])
                            .do_nothing()
                            .to_owned(),
                        )
                        .to_owned(),
                )
                .await?;
            }
        }

        txn.commit().await?;

        Ok(())
    }

    /// No-op: re-introducing TEXT storage would re-introduce the parsing bug.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
