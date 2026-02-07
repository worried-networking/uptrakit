use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ApiRateLimits::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ApiRateLimits::Key)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ApiRateLimits::RequestCount)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ApiRateLimits::WindowStart)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ApiRateLimits::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_api_rate_limits_expires_at")
                    .table(ApiRateLimits::Table)
                    .col(ApiRateLimits::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ApiRateLimits::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum ApiRateLimits {
    Table,
    Key,
    RequestCount,
    WindowStart,
    ExpiresAt,
}
