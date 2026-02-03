use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m20260129_000001_initial::Tenants;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MqttLeases::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MqttLeases::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MqttLeases::TenantId).uuid().not_null())
                    .col(string(MqttLeases::InstanceId))
                    .col(timestamp(MqttLeases::HeartbeatAt))
                    .col(timestamp(MqttLeases::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mqtt_leases_tenant")
                            .from(MqttLeases::Table, MqttLeases::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // One lease per tenant (prevents duplicate claims)
        manager
            .create_index(
                Index::create()
                    .name("uq_mqtt_leases_tenant_id")
                    .table(MqttLeases::Table)
                    .col(MqttLeases::TenantId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Fast release-all queries by instance
        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_leases_instance_id")
                    .table(MqttLeases::Table)
                    .col(MqttLeases::InstanceId)
                    .to_owned(),
            )
            .await?;

        // Stale lease detection
        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_leases_heartbeat_at")
                    .table(MqttLeases::Table)
                    .col(MqttLeases::HeartbeatAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MqttLeases::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum MqttLeases {
    Table,
    Id,
    TenantId,
    InstanceId,
    HeartbeatAt,
    CreatedAt,
}
