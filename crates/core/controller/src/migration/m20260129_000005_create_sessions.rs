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
                    .table(Sessions::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Sessions::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Sessions::UserId).uuid().not_null())
                    .col(string_uniq(Sessions::TokenHash))
                    .col(string(Sessions::AuthMethod))
                    .col(timestamp(Sessions::CreatedAt))
                    .col(timestamp(Sessions::ExpiresAt))
                    .col(timestamp(Sessions::LastActivityAt))
                    .col(string_null(Sessions::UserAgent))
                    .col(string_null(Sessions::IpAddress))
                    .col(ColumnDef::new(Sessions::OidcProviderId).uuid().null())
                    .foreign_key(
                        ForeignKey::create()
                            .from(Sessions::Table, Sessions::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_sessions_oidc_provider_id")
                            .from(Sessions::Table, Sessions::OidcProviderId)
                            .to(OidcProviders::Table, OidcProviders::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on token_hash for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_token_hash")
                    .table(Sessions::Table)
                    .col(Sessions::TokenHash)
                    .to_owned(),
            )
            .await?;

        // Create index on user_id for user session queries
        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_user_id")
                    .table(Sessions::Table)
                    .col(Sessions::UserId)
                    .to_owned(),
            )
            .await?;

        // Create index on expires_at for cleanup queries
        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_expires_at")
                    .table(Sessions::Table)
                    .col(Sessions::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Sessions::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
    UserId,
    TokenHash,
    AuthMethod,
    CreatedAt,
    ExpiresAt,
    LastActivityAt,
    UserAgent,
    IpAddress,
    OidcProviderId,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum OidcProviders {
    Table,
    Id,
}
