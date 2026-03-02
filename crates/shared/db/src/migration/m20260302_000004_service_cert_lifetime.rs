use sea_orm_migration::prelude::*;

/// Add `cert_lifetime_hours` (nullable INTEGER) to `services`.
///
/// This column stores a per-service override for the certificate lifetime
/// in hours. `NULL` means "use the global default".
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Services::Table)
                    .add_column(ColumnDef::new(Services::CertLifetimeHours).integer().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Services::Table)
                    .drop_column(Services::CertLifetimeHours)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    CertLifetimeHours,
}
