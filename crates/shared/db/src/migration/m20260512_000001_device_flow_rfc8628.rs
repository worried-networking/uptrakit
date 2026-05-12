use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260512_000001_device_flow_rfc8628"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .add_column(
                        ColumnDef::new(PendingDeviceFlows::LastPolledAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .add_column(
                        ColumnDef::new(PendingDeviceFlows::Interval)
                            .integer()
                            .not_null()
                            .default(5),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .add_column(ColumnDef::new(PendingDeviceFlows::Scope).text().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .add_column(ColumnDef::new(PendingDeviceFlows::DeniedBy).uuid().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .drop_column(PendingDeviceFlows::DeniedBy)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .drop_column(PendingDeviceFlows::Scope)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .drop_column(PendingDeviceFlows::Interval)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .drop_column(PendingDeviceFlows::LastPolledAt)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum PendingDeviceFlows {
    Table,
    LastPolledAt,
    Interval,
    Scope,
    DeniedBy,
}
