use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. pending_device_flows
        manager
            .create_table(
                Table::create()
                    .table(PendingDeviceFlows::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingDeviceFlows::DeviceCode)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingDeviceFlows::UserCode)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(PendingDeviceFlows::Status).text().not_null())
                    .col(ColumnDef::new(PendingDeviceFlows::UserId).uuid())
                    .col(ColumnDef::new(PendingDeviceFlows::ClientName).text())
                    .col(
                        ColumnDef::new(PendingDeviceFlows::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingDeviceFlows::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_device_flows_expires_at")
                    .table(PendingDeviceFlows::Table)
                    .col(PendingDeviceFlows::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // 2. pending_oidc_flows
        manager
            .create_table(
                Table::create()
                    .table(PendingOidcFlows::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingOidcFlows::CsrfState)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcFlows::ProviderId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcFlows::PkceVerifier)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingOidcFlows::Nonce).text().not_null())
                    .col(
                        ColumnDef::new(PendingOidcFlows::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcFlows::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_oidc_flows_expires_at")
                    .table(PendingOidcFlows::Table)
                    .col(PendingOidcFlows::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // 3. pending_account_links
        manager
            .create_table(
                Table::create()
                    .table(PendingAccountLinks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingAccountLinks::LinkToken)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingAccountLinks::ProviderId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingAccountLinks::OidcSubject)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingAccountLinks::Email).text().not_null())
                    .col(
                        ColumnDef::new(PendingAccountLinks::UserId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingAccountLinks::FirstName).text())
                    .col(ColumnDef::new(PendingAccountLinks::LastName).text())
                    .col(
                        ColumnDef::new(PendingAccountLinks::MappedRoles)
                            .json()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingAccountLinks::ExistingLinkProviderId).uuid())
                    .col(
                        ColumnDef::new(PendingAccountLinks::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingAccountLinks::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_account_links_expires_at")
                    .table(PendingAccountLinks::Table)
                    .col(PendingAccountLinks::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // 4. pending_oidc_token_exchanges
        manager
            .create_table(
                Table::create()
                    .table(PendingOidcTokenExchanges::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::ExchangeCode)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::UserId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::ProviderId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_oidc_token_exchanges_expires_at")
                    .table(PendingOidcTokenExchanges::Table)
                    .col(PendingOidcTokenExchanges::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(PendingOidcTokenExchanges::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(PendingAccountLinks::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(PendingOidcFlows::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(PendingDeviceFlows::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum PendingDeviceFlows {
    Table,
    DeviceCode,
    UserCode,
    Status,
    UserId,
    ClientName,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum PendingOidcFlows {
    Table,
    CsrfState,
    ProviderId,
    PkceVerifier,
    Nonce,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum PendingAccountLinks {
    Table,
    LinkToken,
    ProviderId,
    OidcSubject,
    Email,
    UserId,
    FirstName,
    LastName,
    MappedRoles,
    ExistingLinkProviderId,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum PendingOidcTokenExchanges {
    Table,
    ExchangeCode,
    UserId,
    ProviderId,
    CreatedAt,
    ExpiresAt,
}
