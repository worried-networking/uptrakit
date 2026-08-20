use sea_orm_migration::prelude::*;

/// Drop the `is_pve_node` column from `ssh_hosts`.
///
/// This column was always `false` — PVE node detection state is tracked in
/// the Proxmox plugin's own `proxmox_host_state` table.
///
/// The earlier `m20260308_000003` table-rebuild migration already omits
/// this column, so on fresh databases it no longer exists; the drop guards
/// on `pragma_table_info` to stay idempotent. This is a frozen merged
/// migration: builder-expressible, but rewriting a shipped migration body
/// risks live-vs-fresh-install divergence.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Check whether the column still exists (it may already be gone after
        // the table-rebuild in m20260308_000003).
        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
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
            #[expect(
                clippy::disallowed_methods,
                reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
            )]
            db.execute_unprepared("ALTER TABLE ssh_hosts DROP COLUMN is_pve_node")
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
        db.execute_unprepared(
            "ALTER TABLE ssh_hosts ADD COLUMN is_pve_node BOOLEAN NOT NULL DEFAULT FALSE",
        )
        .await?;
        Ok(())
    }
}
