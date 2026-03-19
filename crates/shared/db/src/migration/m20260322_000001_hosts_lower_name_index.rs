use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260322_000001_hosts_lower_name_index"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // sea_query Index::create() does not support expression columns (functional indexes);
        // raw SQL required. SQLite and PostgreSQL support LOWER() functional indexes natively.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX idx_hosts_tenant_lower_friendly_name \
                 ON hosts (tenant_id, lower(friendly_name))",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_hosts_tenant_lower_friendly_name")
                    .table(Alias::new("hosts"))
                    .to_owned(),
            )
            .await
    }
}
