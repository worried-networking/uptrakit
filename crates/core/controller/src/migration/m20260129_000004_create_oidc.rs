use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // --- oidc_providers ---
        manager
            .create_table(
                Table::create()
                    .table(OidcProviders::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(OidcProviders::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(string(OidcProviders::Name))
                    .col(string_uniq(OidcProviders::Slug))
                    .col(string_null(OidcProviders::LogoUrl))
                    .col(string(OidcProviders::IssuerUrl))
                    .col(string(OidcProviders::ClientId))
                    .col(string(OidcProviders::ClientSecret))
                    .col(string(OidcProviders::Scopes))
                    .col(boolean(OidcProviders::AutoCreateUsers))
                    .col(string_null(OidcProviders::RoleClaimPath))
                    .col(ColumnDef::new(OidcProviders::RoleMapping).json().not_null())
                    .col(boolean(OidcProviders::IsActive))
                    .col(timestamp(OidcProviders::CreatedAt))
                    .col(timestamp(OidcProviders::UpdatedAt))
                    .col(timestamp_null(OidcProviders::DeletedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_oidc_providers_slug")
                    .table(OidcProviders::Table)
                    .col(OidcProviders::Slug)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_oidc_providers_is_active")
                    .table(OidcProviders::Table)
                    .col(OidcProviders::IsActive)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_oidc_providers_deleted_at")
                    .table(OidcProviders::Table)
                    .col(OidcProviders::DeletedAt)
                    .to_owned(),
            )
            .await?;

        // --- user_oidc_links ---
        manager
            .create_table(
                Table::create()
                    .table(UserOidcLinks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserOidcLinks::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UserOidcLinks::UserId).uuid().not_null())
                    .col(ColumnDef::new(UserOidcLinks::ProviderId).uuid().not_null())
                    .col(string(UserOidcLinks::OidcSubject))
                    .col(timestamp(UserOidcLinks::LinkedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_oidc_links_user_id")
                            .from(UserOidcLinks::Table, UserOidcLinks::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_oidc_links_provider_id")
                            .from(UserOidcLinks::Table, UserOidcLinks::ProviderId)
                            .to(OidcProviders::Table, OidcProviders::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_oidc_links_provider_subject")
                    .table(UserOidcLinks::Table)
                    .col(UserOidcLinks::ProviderId)
                    .col(UserOidcLinks::OidcSubject)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_oidc_links_user_provider")
                    .table(UserOidcLinks::Table)
                    .col(UserOidcLinks::UserId)
                    .col(UserOidcLinks::ProviderId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_oidc_links_user_id")
                    .table(UserOidcLinks::Table)
                    .col(UserOidcLinks::UserId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UserOidcLinks::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(OidcProviders::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum OidcProviders {
    Table,
    Id,
    Name,
    Slug,
    LogoUrl,
    IssuerUrl,
    ClientId,
    ClientSecret,
    Scopes,
    AutoCreateUsers,
    RoleClaimPath,
    RoleMapping,
    IsActive,
    CreatedAt,
    UpdatedAt,
    DeletedAt,
}

#[derive(DeriveIden)]
enum UserOidcLinks {
    Table,
    Id,
    UserId,
    ProviderId,
    OidcSubject,
    LinkedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
