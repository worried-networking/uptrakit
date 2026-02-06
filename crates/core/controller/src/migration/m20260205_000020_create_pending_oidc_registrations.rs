use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PendingOidcRegistrations::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::RegistrationCode)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::ProviderId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::OidcSubject)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::Email)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingOidcRegistrations::FirstName).text())
                    .col(ColumnDef::new(PendingOidcRegistrations::LastName).text())
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::MappedRoles)
                            .json()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_oidc_registrations_expires_at")
                    .table(PendingOidcRegistrations::Table)
                    .col(PendingOidcRegistrations::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(PendingOidcRegistrations::Table)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum PendingOidcRegistrations {
    Table,
    RegistrationCode,
    ProviderId,
    OidcSubject,
    Email,
    FirstName,
    LastName,
    MappedRoles,
    CreatedAt,
    ExpiresAt,
}
