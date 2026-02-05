use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m20260129_000001_initial::Tenants;
use super::m20260129_000007_create_agents::Services;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create hosts table
        manager
            .create_table(
                Table::create()
                    .table(Hosts::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Hosts::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Hosts::TenantId).uuid().not_null())
                    .col(string(Hosts::MachineId))
                    .col(string(Hosts::Hostname))
                    .col(string(Hosts::FriendlyName))
                    .col(string_null(Hosts::OsType))
                    .col(string_null(Hosts::OsVersion))
                    .col(string_null(Hosts::Architecture))
                    .col(string_null(Hosts::IpAddress))
                    .col(timestamp_null(Hosts::LastSeenAt))
                    .col(timestamp(Hosts::CreatedAt))
                    .col(timestamp(Hosts::UpdatedAt))
                    .col(timestamp_null(Hosts::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hosts_tenant")
                            .from(Hosts::Table, Hosts::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Index on tenant_id
        manager
            .create_index(
                Index::create()
                    .name("idx_hosts_tenant_id")
                    .table(Hosts::Table)
                    .col(Hosts::TenantId)
                    .to_owned(),
            )
            .await?;

        // Unique constraint: (tenant_id, machine_id)
        manager
            .create_index(
                Index::create()
                    .name("uq_hosts_tenant_machine_id")
                    .table(Hosts::Table)
                    .col(Hosts::TenantId)
                    .col(Hosts::MachineId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Index on deactivated_at for soft-delete filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_hosts_deactivated_at")
                    .table(Hosts::Table)
                    .col(Hosts::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // Create service_hosts junction table
        manager
            .create_table(
                Table::create()
                    .table(ServiceHosts::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(ServiceHosts::ServiceId).uuid().not_null())
                    .col(ColumnDef::new(ServiceHosts::HostId).uuid().not_null())
                    .col(timestamp(ServiceHosts::LinkedAt))
                    .primary_key(
                        Index::create()
                            .col(ServiceHosts::ServiceId)
                            .col(ServiceHosts::HostId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_service_hosts_service")
                            .from(ServiceHosts::Table, ServiceHosts::ServiceId)
                            .to(Services::Table, Services::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_service_hosts_host")
                            .from(ServiceHosts::Table, ServiceHosts::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ServiceHosts::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Hosts::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    Id,
    TenantId,
    MachineId,
    Hostname,
    FriendlyName,
    OsType,
    OsVersion,
    Architecture,
    IpAddress,
    LastSeenAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum ServiceHosts {
    Table,
    ServiceId,
    HostId,
    LinkedAt,
}
