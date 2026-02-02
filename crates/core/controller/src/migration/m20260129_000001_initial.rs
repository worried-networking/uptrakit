use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create tenants table
        manager
            .create_table(
                Table::create()
                    .table(Tenants::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Tenants::Id).uuid().not_null().primary_key())
                    .col(string(Tenants::Name))
                    .col(string_uniq(Tenants::Slug))
                    .col(boolean(Tenants::IsDefault).default(false))
                    .col(timestamp(Tenants::CreatedAt))
                    .col(timestamp(Tenants::UpdatedAt))
                    .col(timestamp_null(Tenants::DeactivatedAt))
                    .to_owned(),
            )
            .await?;

        // Index on slug for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_tenants_slug")
                    .table(Tenants::Table)
                    .col(Tenants::Slug)
                    .to_owned(),
            )
            .await?;

        // Index on deactivated_at for active-tenant filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_tenants_deactivated_at")
                    .table(Tenants::Table)
                    .col(Tenants::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // Seed default tenant
        let now = time::OffsetDateTime::now_utc();
        manager
            .exec_stmt(
                Query::insert()
                    .into_table(Tenants::Table)
                    .columns([
                        Tenants::Id,
                        Tenants::Name,
                        Tenants::Slug,
                        Tenants::IsDefault,
                        Tenants::CreatedAt,
                        Tenants::UpdatedAt,
                    ])
                    .values_panic([
                        Uuid::now_v7().into(),
                        "Default".into(),
                        "default".into(),
                        true.into(),
                        now.into(),
                        now.into(),
                    ])
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Tenants::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum Tenants {
    Table,
    Id,
    Name,
    Slug,
    IsDefault,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}
