use sea_orm_migration::prelude::*;

/// Add indexes to `proxmox_host_mappings` to support paginated queries:
///
/// - `idx_proxmox_hm_config_name_vmid` on `(plugin_config_id, proxmox_name, proxmox_vmid)`:
///   supports the `handle_list` query which filters by `plugin_config_id` and orders by
///   `(proxmox_name, proxmox_vmid)`.
///
/// - `idx_proxmox_hm_tenant_host` on `(tenant_id, host_id)`:
///   supports the `handle_list_all_unmatched` query which filters by `tenant_id` and
///   `host_id IS NULL`.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Composite index for paginated listing by plugin config, ordered by name + vmid.
        manager
            .create_index(
                Index::create()
                    .name("idx_proxmox_hm_config_name_vmid")
                    .table(ProxmoxHostMappings::Table)
                    .col(ProxmoxHostMappings::PluginConfigId)
                    .col(ProxmoxHostMappings::ProxmoxName)
                    .col(ProxmoxHostMappings::ProxmoxVmid)
                    .to_owned(),
            )
            .await?;

        // Composite index for unmatched-guest queries (tenant + host_id IS NULL filter).
        manager
            .create_index(
                Index::create()
                    .name("idx_proxmox_hm_tenant_host")
                    .table(ProxmoxHostMappings::Table)
                    .col(ProxmoxHostMappings::TenantId)
                    .col(ProxmoxHostMappings::HostId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_proxmox_hm_config_name_vmid")
                    .table(ProxmoxHostMappings::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_proxmox_hm_tenant_host")
                    .table(ProxmoxHostMappings::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ProxmoxHostMappings {
    Table,
    PluginConfigId,
    ProxmoxName,
    ProxmoxVmid,
    TenantId,
    HostId,
}
