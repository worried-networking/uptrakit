use sea_orm_migration::prelude::*;

/// Create the `proxmox_host_mappings` table for tracking discovered Proxmox
/// VMs/CTs and their mapping to Uptrakit hosts.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxHostMappings::Table)
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ProxmoxHostMappings::HostId).uuid().null())
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::ProxmoxNode)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::ProxmoxVmid)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::ProxmoxType)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::ProxmoxName)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::ProxmoxStatus)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ProxmoxHostMappings::Hostname).text().null())
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::IpAddresses)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::MatchMethod)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::DiscoveredAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostMappings::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_hm_tenant_id")
                            .from(ProxmoxHostMappings::Table, ProxmoxHostMappings::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_hm_plugin_config_id")
                            .from(
                                ProxmoxHostMappings::Table,
                                ProxmoxHostMappings::PluginConfigId,
                            )
                            .to(PluginConfigs::Table, PluginConfigs::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_hm_host_id")
                            .from(ProxmoxHostMappings::Table, ProxmoxHostMappings::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        // Unique constraint: one mapping per (plugin_config, node, vmid)
        manager
            .create_index(
                Index::create()
                    .name("uix_proxmox_hm_config_node_vmid")
                    .table(ProxmoxHostMappings::Table)
                    .col(ProxmoxHostMappings::PluginConfigId)
                    .col(ProxmoxHostMappings::ProxmoxNode)
                    .col(ProxmoxHostMappings::ProxmoxVmid)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Index for tenant-scoped queries
        manager
            .create_index(
                Index::create()
                    .name("idx_proxmox_hm_tenant")
                    .table(ProxmoxHostMappings::Table)
                    .col(ProxmoxHostMappings::TenantId)
                    .to_owned(),
            )
            .await?;

        // Index for host lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_proxmox_hm_host")
                    .table(ProxmoxHostMappings::Table)
                    .col(ProxmoxHostMappings::HostId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ProxmoxHostMappings::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ProxmoxHostMappings {
    Table,
    Id,
    TenantId,
    PluginConfigId,
    HostId,
    ProxmoxNode,
    ProxmoxVmid,
    ProxmoxType,
    ProxmoxName,
    ProxmoxStatus,
    Hostname,
    IpAddresses,
    MatchMethod,
    DiscoveredAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum PluginConfigs {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    Id,
}
