use sea_orm_migration::prelude::*;

/// Add a partial unique index on `update_history(host_id, software_item_id)`
/// scoped to active rows (status IN ('queued', 'pending', 'in_progress',
/// 'awaiting_restart')).
///
/// This complements the existing `uix_update_history_host_active` (host-level
/// serialisation) with a narrower invariant: no two rows for the same
/// (host, software item) pair may exist in any non-terminal status.
///
/// Allows batch updates to create Queued rows for different software items on
/// the same host while preventing duplicate triggers for the same item.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        #[expect(
            clippy::disallowed_methods,
            reason = "builder limitation: partial index with a WHERE clause is not expressible via sea_query's CREATE INDEX builder"
        )]
        db.execute_unprepared(
            "CREATE UNIQUE INDEX uix_update_history_host_software_item_active \
             ON update_history (host_id, software_item_id) \
             WHERE status IN ('queued', 'pending', 'in_progress', 'awaiting_restart')",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
        db.execute_unprepared("DROP INDEX IF EXISTS uix_update_history_host_software_item_active")
            .await?;
        Ok(())
    }
}
