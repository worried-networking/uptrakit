use sea_orm_migration::prelude::*;

/// Drop the `is_pve_node` column from `ssh_hosts`.
///
/// This column was always `false` — PVE node detection state is tracked in
/// the Proxmox plugin's own `proxmox_host_state` table.
///
/// Uses raw SQL for two reasons:
/// 1. sea-query's `ALTER TABLE DROP COLUMN` double-quotes the identifier on
///    SQLite, causing a spurious "no such column" error.
/// 2. The earlier `m20260308_000003` table-rebuild migration already omits
///    this column, so on fresh databases it no longer exists. We guard with
///    `pragma_table_info` to make the drop idempotent.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Check whether the column still exists (it may already be gone after
        // the table-rebuild in m20260308_000003).
        let has_col = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS cnt FROM pragma_table_info('ssh_hosts') WHERE name = 'is_pve_node'",
            ))
            .await?;

        let col_exists = has_col
            .as_ref()
            .and_then(|r| {
                use sea_orm::TryGetable as _;
                i32::try_get_by_index(r, 0).ok()
            })
            .unwrap_or(0)
            > 0;

        if col_exists {
            db.execute_unprepared("ALTER TABLE ssh_hosts DROP COLUMN is_pve_node")
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "ALTER TABLE ssh_hosts ADD COLUMN is_pve_node BOOLEAN NOT NULL DEFAULT FALSE",
        )
        .await?;
        Ok(())
    }
}
