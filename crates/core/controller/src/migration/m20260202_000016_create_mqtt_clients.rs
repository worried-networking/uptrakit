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
                    .table(MqttClients::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MqttClients::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MqttClients::TenantId).uuid().not_null())
                    .col(boolean(MqttClients::Enabled).default(true))
                    .col(string(MqttClients::Transport).default("tcp"))
                    .col(string(MqttClients::Host))
                    .col(integer(MqttClients::Port).default(1883))
                    .col(string(MqttClients::ClientId).default("uptrakit-controller"))
                    .col(string_null(MqttClients::Username))
                    .col(string_null(MqttClients::Password))
                    .col(string(MqttClients::TopicPrefix).default("uptrakit"))
                    .col(
                        ColumnDef::new(MqttClients::ConnectionStatus)
                            .string()
                            .not_null()
                            .default("offline"),
                    )
                    .col(
                        ColumnDef::new(MqttClients::StatusUpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(timestamp(MqttClients::CreatedAt))
                    .col(timestamp(MqttClients::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mqtt_clients_tenant")
                            .from(MqttClients::Table, MqttClients::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Non-unique index for efficient lookups by tenant
        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_clients_tenant_id")
                    .table(MqttClients::Table)
                    .col(MqttClients::TenantId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MqttClients::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub(super) enum MqttClients {
    Table,
    Id,
    TenantId,
    Enabled,
    Transport,
    Host,
    Port,
    ClientId,
    Username,
    Password,
    TopicPrefix,
    ConnectionStatus,
    StatusUpdatedAt,
    CreatedAt,
    UpdatedAt,
}
