use sea_orm_migration::prelude::*;

/// Add `allow_private_network_issuers` to `oidc_providers`.
///
/// The column controls whether OIDC issuer hostnames may resolve to private,
/// loopback, or other non-public network addresses. It defaults to `TRUE` so
/// existing single-tenant deployments keep working after the migration.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(OidcProviders::Table)
                    .add_column(
                        ColumnDef::new(OidcProviders::AllowPrivateNetworkIssuers)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(OidcProviders::Table)
                    .drop_column(OidcProviders::AllowPrivateNetworkIssuers)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum OidcProviders {
    Table,
    AllowPrivateNetworkIssuers,
}
