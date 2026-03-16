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
        // MariaDB does not support expression indexes — fall back to a plain column index
        // (MariaDB collations are case-insensitive by default).
        let sql = if manager.get_database_backend() == sea_orm::DatabaseBackend::MySql {
            "CREATE INDEX idx_hosts_tenant_lower_friendly_name \
             ON hosts (tenant_id, friendly_name)"
        } else {
            "CREATE INDEX idx_hosts_tenant_lower_friendly_name \
             ON hosts (tenant_id, lower(friendly_name))"
        };
        manager.get_connection().execute_unprepared(sql).await?;
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
