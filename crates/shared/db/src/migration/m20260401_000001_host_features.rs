use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Hosts::Table)
                    .add_column(ColumnDef::new(Hosts::HostFeatures).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Hosts::Table)
                    .drop_column(Hosts::HostFeatures)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    HostFeatures,
}
