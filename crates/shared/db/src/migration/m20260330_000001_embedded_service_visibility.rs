use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Services::Table)
                    .add_column(
                        ColumnDef::new(Services::IsEmbedded)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Services::Table)
                    .add_column(ColumnDef::new(Services::EmbeddedOwnerKey).uuid().null())
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(SystemServices::Table)
                    .add_column(
                        ColumnDef::new(SystemServices::IsEmbedded)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(SystemServices::Table)
                    .add_column(
                        ColumnDef::new(SystemServices::EmbeddedOwnerKey)
                            .uuid()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE services SET is_embedded = TRUE WHERE enrollment_secret_hash LIKE 'embedded:%'",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE system_services SET is_embedded = TRUE WHERE enrollment_secret_hash LIKE 'embedded:%'",
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_services_embedded_owner")
                    .table(Services::Table)
                    .col(Services::TenantId)
                    .col(Services::ServiceAppName)
                    .col(Services::EmbeddedOwnerKey)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_system_services_embedded_owner")
                    .table(SystemServices::Table)
                    .col(SystemServices::ServiceAppName)
                    .col(SystemServices::EmbeddedOwnerKey)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(EmbeddedServiceRuntimeStates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(EmbeddedServiceRuntimeStates::ServiceId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(EmbeddedServiceRuntimeStates::YieldedToJson)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(EmbeddedServiceRuntimeStates::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(EmbeddedServiceRuntimeStates::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("uq_system_services_embedded_owner")
                    .table(SystemServices::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("uq_services_embedded_owner")
                    .table(Services::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(SystemServices::Table)
                    .drop_column(SystemServices::IsEmbedded)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Services::Table)
                    .drop_column(Services::IsEmbedded)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(SystemServices::Table)
                    .drop_column(SystemServices::EmbeddedOwnerKey)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Services::Table)
                    .drop_column(Services::EmbeddedOwnerKey)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    TenantId,
    ServiceAppName,
    IsEmbedded,
    EmbeddedOwnerKey,
}

#[derive(DeriveIden)]
enum SystemServices {
    Table,
    ServiceAppName,
    IsEmbedded,
    EmbeddedOwnerKey,
}

#[derive(DeriveIden)]
enum EmbeddedServiceRuntimeStates {
    Table,
    ServiceId,
    YieldedToJson,
    UpdatedAt,
}
