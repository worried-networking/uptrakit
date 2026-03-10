use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260319_000001_software_items_sort_index"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // sea_query Index::create() does not support expression columns (functional indexes);
        // raw SQL required. All three backends support LOWER() functional indexes; MySQL 8+
        // requires double parens around the expression per its CREATE INDEX syntax.
        let sql = if manager.get_database_backend() == sea_orm::DatabaseBackend::MySql {
            "CREATE INDEX idx_software_items_tenant_lower_name \
             ON software_items (tenant_id, (lower(name)))"
        } else {
            "CREATE INDEX idx_software_items_tenant_lower_name \
             ON software_items (tenant_id, lower(name))"
        };
        manager.get_connection().execute_unprepared(sql).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_software_items_tenant_lower_name")
                    .table(Alias::new("software_items"))
                    .to_owned(),
            )
            .await
    }
}
