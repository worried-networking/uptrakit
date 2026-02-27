use sea_orm_migration::prelude::*;

/// Add standalone indexes on columns that appear in `WHERE`, `JOIN`, or `ORDER BY`
/// clauses but lacked dedicated indexes:
///
/// - `update_history.created_at` — used for range queries and dashboard ordering
/// - `host_software_items.software_item_id` — FK used in reverse lookups
/// - `mqtt_leases.tenant_id` — FK present but unindexed
/// - `service_hosts.host_id` — FK with composite PK `(service_id, host_id)`;
///   the composite PK does not accelerate host-by-service lookups alone
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_created_at")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::CreatedAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_host_software_items_software_item_id")
                    .table(HostSoftwareItems::Table)
                    .col(HostSoftwareItems::SoftwareItemId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_leases_tenant_id")
                    .table(MqttLeases::Table)
                    .col(MqttLeases::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_service_hosts_host_id")
                    .table(ServiceHosts::Table)
                    .col(ServiceHosts::HostId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_service_hosts_host_id")
                    .table(ServiceHosts::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_mqtt_leases_tenant_id")
                    .table(MqttLeases::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_host_software_items_software_item_id")
                    .table(HostSoftwareItems::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_update_history_created_at")
                    .table(UpdateHistory::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    CreatedAt,
}

#[derive(DeriveIden)]
enum HostSoftwareItems {
    Table,
    SoftwareItemId,
}

#[derive(DeriveIden)]
enum MqttLeases {
    Table,
    TenantId,
}

#[derive(DeriveIden)]
enum ServiceHosts {
    Table,
    HostId,
}
