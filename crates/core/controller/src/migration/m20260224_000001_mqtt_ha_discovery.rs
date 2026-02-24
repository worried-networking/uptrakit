use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ── update_history: replace initiated_by with actor_type + actor_id ──

        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .add_column(
                        ColumnDef::new(UpdateHistory::ActorType)
                            .string()
                            .not_null()
                            .default("legacy"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .add_column(
                        ColumnDef::new(UpdateHistory::ActorId)
                            .string()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .drop_column(UpdateHistory::InitiatedBy)
                    .to_owned(),
            )
            .await?;

        // ── mqtt_clients: add HA discovery columns ──────────────────────────

        manager
            .alter_table(
                Table::alter()
                    .table(MqttClients::Table)
                    .add_column(
                        ColumnDef::new(MqttClients::HaDiscovery)
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
                    .table(MqttClients::Table)
                    .add_column(
                        ColumnDef::new(MqttClients::HaDiscoveryPrefix)
                            .string()
                            .not_null()
                            .default("homeassistant"),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Restore initiated_by on update_history
        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .add_column(
                        ColumnDef::new(UpdateHistory::InitiatedBy)
                            .string()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .drop_column(UpdateHistory::ActorType)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .drop_column(UpdateHistory::ActorId)
                    .to_owned(),
            )
            .await?;

        // Remove HA discovery columns from mqtt_clients
        manager
            .alter_table(
                Table::alter()
                    .table(MqttClients::Table)
                    .drop_column(MqttClients::HaDiscovery)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(MqttClients::Table)
                    .drop_column(MqttClients::HaDiscoveryPrefix)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    InitiatedBy,
    ActorType,
    ActorId,
}

#[derive(DeriveIden)]
enum MqttClients {
    Table,
    HaDiscovery,
    HaDiscoveryPrefix,
}
