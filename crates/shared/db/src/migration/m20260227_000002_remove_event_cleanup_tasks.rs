use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum ScheduledTasks {
    Table,
    TaskType,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let stmt = Query::delete()
            .from_table(ScheduledTasks::Table)
            .and_where(Expr::col(ScheduledTasks::TaskType).eq("event_cleanup"))
            .to_owned();
        manager.get_connection().execute(&stmt).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // EventCleanup rows were obsolete; no rollback needed.
        Ok(())
    }
}
