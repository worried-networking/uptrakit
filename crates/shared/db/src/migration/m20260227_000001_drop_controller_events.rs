use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("controller_events"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Recreate the table for rollback. Columns match the original
        // m20260209_000001_initial migration.
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("controller_events"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .big_integer()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("source_controller_id"))
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Alias::new("target_service_id")).uuid())
                    .col(ColumnDef::new(Alias::new("target_capability")).text())
                    .col(ColumnDef::new(Alias::new("message_json")).json().not_null())
                    .col(
                        ColumnDef::new(Alias::new("created_at"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }
}
