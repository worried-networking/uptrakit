use sea_orm_migration::prelude::*;

/// Add `service_app_name` (nullable TEXT) to `services` and `system_services`.
///
/// Stores the binary/crate name of the service (e.g., `"uptrakit-agent-ssh"`).
/// Nullable because existing enrolled services won't have this value until they
/// re-enroll. New enrollments always provide it.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Services::Table)
                    .add_column(ColumnDef::new(Services::ServiceAppName).text().null())
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(SystemServices::Table)
                    .add_column(ColumnDef::new(SystemServices::ServiceAppName).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Services::Table)
                    .drop_column(Services::ServiceAppName)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(SystemServices::Table)
                    .drop_column(SystemServices::ServiceAppName)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    ServiceAppName,
}

#[derive(DeriveIden)]
enum SystemServices {
    Table,
    ServiceAppName,
}
