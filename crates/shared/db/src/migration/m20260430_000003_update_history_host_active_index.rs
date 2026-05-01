use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP INDEX IF EXISTS uix_update_history_host_active")
            .await?;
        db.execute_unprepared(
            "CREATE UNIQUE INDEX uix_update_history_host_active \
             ON update_history (host_id) \
             WHERE status IN ('pending', 'in_progress', 'awaiting_restart')",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP INDEX IF EXISTS uix_update_history_host_active")
            .await?;
        db.execute_unprepared(
            "CREATE UNIQUE INDEX uix_update_history_host_active \
             ON update_history (host_id) \
             WHERE status IN ('pending', 'in_progress')",
        )
        .await?;
        Ok(())
    }
}
