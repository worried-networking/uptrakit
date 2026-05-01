use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("software_items"))
                    .add_column_if_not_exists(
                        ColumnDef::new(Alias::new("awaiting_restart_timeout"))
                            .integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("software_items"))
                    .drop_column(Alias::new("awaiting_restart_timeout"))
                    .to_owned(),
            )
            .await
    }
}
