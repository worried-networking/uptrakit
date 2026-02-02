use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m20260129_000001_initial::Tenants;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Settings::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Settings::TenantId).uuid().not_null())
                    .col(ColumnDef::new(Settings::Key).string().not_null())
                    .col(ColumnDef::new(Settings::Value).json().not_null())
                    .col(timestamp(Settings::UpdatedAt))
                    .primary_key(Index::create().col(Settings::TenantId).col(Settings::Key))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_settings_tenant")
                            .from(Settings::Table, Settings::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Settings::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Settings {
    Table,
    TenantId,
    Key,
    Value,
    UpdatedAt,
}
