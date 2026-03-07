use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PendingProxmoxMatches::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingProxmoxMatches::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingProxmoxMatches::HostId)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingProxmoxMatches::MappingId)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingProxmoxMatches::CreatedAt)
                            .text()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(PendingProxmoxMatches::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum PendingProxmoxMatches {
    Table,
    Id,
    HostId,
    MappingId,
    CreatedAt,
}
