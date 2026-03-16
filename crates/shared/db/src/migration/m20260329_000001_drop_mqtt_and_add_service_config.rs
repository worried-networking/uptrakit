use sea_orm_migration::prelude::*;

/// Drop MQTT tables and add generic service config store tables.
///
/// ## Dropped tables
/// - `mqtt_leases`: MQTT lease coordination (moved to service-owned state)
/// - `mqtt_clients`: MQTT client configuration (moved to service config store)
///
/// ## New tables
/// - `tenant_service_config`: Tenant-scoped service config entries (key/value)
/// - `global_service_config`: Global (cross-tenant) service config entries
///
/// Both new tables support sensitive values stored as EncryptedString (TEXT),
/// flagged by the `is_sensitive` column. The controller decrypts before delivery.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop MQTT tables (in FK-safe order: leases before clients).
        manager
            .drop_table(
                Table::drop()
                    .table(MqttLeases::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(MqttClients::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        // Create tenant_service_config table.
        manager
            .create_table(
                Table::create()
                    .table(TenantServiceConfig::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TenantServiceConfig::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(TenantServiceConfig::ServiceName)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TenantServiceConfig::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(TenantServiceConfig::Key).string().not_null())
                    .col(ColumnDef::new(TenantServiceConfig::Value).text().not_null())
                    .col(
                        ColumnDef::new(TenantServiceConfig::IsSensitive)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(TenantServiceConfig::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TenantServiceConfig::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(TenantServiceConfig::Table, TenantServiceConfig::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_tenant_service_config_unique")
                    .table(TenantServiceConfig::Table)
                    .col(TenantServiceConfig::ServiceName)
                    .col(TenantServiceConfig::TenantId)
                    .col(TenantServiceConfig::Key)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create global_service_config table.
        manager
            .create_table(
                Table::create()
                    .table(GlobalServiceConfig::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(GlobalServiceConfig::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(GlobalServiceConfig::ServiceName)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(GlobalServiceConfig::Key).string().not_null())
                    .col(ColumnDef::new(GlobalServiceConfig::Value).text().not_null())
                    .col(
                        ColumnDef::new(GlobalServiceConfig::IsSensitive)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(GlobalServiceConfig::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GlobalServiceConfig::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_global_service_config_unique")
                    .table(GlobalServiceConfig::Table)
                    .col(GlobalServiceConfig::ServiceName)
                    .col(GlobalServiceConfig::Key)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_global_service_config_unique")
                    .table(GlobalServiceConfig::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(Table::drop().table(GlobalServiceConfig::Table).to_owned())
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_tenant_service_config_unique")
                    .table(TenantServiceConfig::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(Table::drop().table(TenantServiceConfig::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum MqttClients {
    Table,
}

#[derive(DeriveIden)]
enum MqttLeases {
    Table,
}

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum TenantServiceConfig {
    Table,
    Id,
    ServiceName,
    TenantId,
    Key,
    Value,
    IsSensitive,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum GlobalServiceConfig {
    Table,
    Id,
    ServiceName,
    Key,
    Value,
    IsSensitive,
    CreatedAt,
    UpdatedAt,
}
