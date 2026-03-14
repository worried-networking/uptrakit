use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260322_000001_ssh_hosts_lower_name_index"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // sea_query Index::create() does not support expression columns (functional indexes);
        // raw SQL required. The SSH agent uses SQLite exclusively, so no backend branching
        // is needed.
        manager
            .get_connection()
            .execute_unprepared("CREATE INDEX idx_ssh_hosts_lower_name ON ssh_hosts (lower(name))")
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_ssh_hosts_lower_name")
                    .table(Alias::new("ssh_hosts"))
                    .to_owned(),
            )
            .await
    }
}
