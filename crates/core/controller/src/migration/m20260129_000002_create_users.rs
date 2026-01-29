use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Users::Id).uuid().not_null().primary_key())
                    .col(string_uniq(Users::Email))
                    .col(string(Users::FirstName))
                    .col(string(Users::LastName))
                    .col(string_null(Users::PasswordHash))
                    .col(boolean(Users::IsActive).default(true))
                    .col(timestamp_null(Users::DeactivatedAt))
                    .col(timestamp(Users::CreatedAt))
                    .col(timestamp(Users::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_users_email")
                    .table(Users::Table)
                    .col(Users::Email)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    FirstName,
    LastName,
    PasswordHash,
    IsActive,
    DeactivatedAt,
    CreatedAt,
    UpdatedAt,
}
