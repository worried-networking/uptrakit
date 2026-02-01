use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create provider_configs table
        manager
            .create_table(
                Table::create()
                    .table(ProviderConfigs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ProviderConfigs::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(string(ProviderConfigs::Name))
                    .col(string(ProviderConfigs::ProviderType))
                    .col(json(ProviderConfigs::Config))
                    .col(boolean(ProviderConfigs::Enabled).default(true))
                    .col(timestamp(ProviderConfigs::CreatedAt))
                    .col(timestamp(ProviderConfigs::UpdatedAt))
                    .col(timestamp_null(ProviderConfigs::DeactivatedAt))
                    .to_owned(),
            )
            .await?;

        // Index on provider_type for filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_provider_configs_provider_type")
                    .table(ProviderConfigs::Table)
                    .col(ProviderConfigs::ProviderType)
                    .to_owned(),
            )
            .await?;

        // Index on deactivated_at for soft-delete filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_provider_configs_deactivated_at")
                    .table(ProviderConfigs::Table)
                    .col(ProviderConfigs::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ProviderConfigs::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum ProviderConfigs {
    Table,
    Id,
    Name,
    ProviderType,
    Config,
    Enabled,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}
