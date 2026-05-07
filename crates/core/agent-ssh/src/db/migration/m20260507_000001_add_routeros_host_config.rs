use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260507_000001_add_routeros_host_config"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE routeros_host_config (
                    ssh_host_id  BLOB    NOT NULL PRIMARY KEY
                                         REFERENCES ssh_hosts(id) ON DELETE CASCADE,
                    allow_reboot INTEGER NOT NULL DEFAULT 0
                )",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS routeros_host_config")
            .await?;
        Ok(())
    }
}
