//! Controller-side database migrations for the Proxmox infrastructure plugin.
//!
//! These migrations create and maintain the `proxmox_host_mappings` table on the
//! controller's database.  They are contributed to the controller's migration set
//! via [`PluginBase::controller_migrations()`](uptrakit_plugin_infrastructure_core::PluginBase::controller_migrations).
//!
//! Each migration uses a manual [`MigrationName`] implementation so that the
//! name recorded in `seaql_migrations` matches the original name from when
//! these migrations lived in `crates/shared/db`.

use sea_orm_migration::prelude::*;

// ── Migration: create proxmox_host_mappings ─────────────────────────────────

/// Create the `proxmox_host_mappings` table for tracking discovered Proxmox
/// VMs/CTs and their mapping to Uptrakit hosts.
pub struct CreateProxmoxHostMappings;

impl MigrationName for CreateProxmoxHostMappings {
    fn name(&self) -> &str {
        "m20260314_000001_proxmox_host_mapping"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxHostMappings {
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

// ── Migration: add machine_id ───────────────────────────────────────────────

/// Add `machine_id` column to `proxmox_host_mappings`.
///
/// Populated best-effort during QEMU discovery via the guest agent
/// file-read endpoint (`/etc/machine-id`). LXC containers will have
/// `NULL` until the host reports its machine_id after bootstrap.
pub struct AddProxmoxHmMachineId;

impl MigrationName for AddProxmoxHmMachineId {
    fn name(&self) -> &str {
        "m20260315_000001_proxmox_hm_machine_id"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddProxmoxHmMachineId {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostMappings::Table)
                    .add_column(ColumnDef::new(ProxmoxHostMappings::MachineId).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostMappings::Table)
                    .drop_column(ProxmoxHostMappings::MachineId)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration: add pagination indexes ───────────────────────────────────────

/// Add indexes to `proxmox_host_mappings` to support paginated queries:
///
/// - `idx_proxmox_hm_config_name_vmid` on `(plugin_config_id, proxmox_name, proxmox_vmid)`:
///   supports the `handle_list` query which filters by `plugin_config_id` and orders by
///   `(proxmox_name, proxmox_vmid)`.
///
/// - `idx_proxmox_hm_tenant_host` on `(tenant_id, host_id)`:
///   supports the `handle_list_all_unmatched` query which filters by `tenant_id` and
///   `host_id IS NULL`.
pub struct AddProxmoxHmPaginationIndexes;

impl MigrationName for AddProxmoxHmPaginationIndexes {
    fn name(&self) -> &str {
        "m20260308_000003_proxmox_hm_pagination_indexes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddProxmoxHmPaginationIndexes {
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

// ── Identifiers ─────────────────────────────────────────────────────────────

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
    MachineId,
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

// ── Migration: add lower(proxmox_name) sort index ───────────────────────────

/// Add a functional index on `lower(proxmox_name)` to `proxmox_host_mappings`
/// so that case-insensitive name sorting uses an index scan rather than a
/// full-table sort.
pub struct AddProxmoxHmLowerNameIndex;

impl MigrationName for AddProxmoxHmLowerNameIndex {
    fn name(&self) -> &str {
        "m20260322_000001_proxmox_hm_lower_name_index"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddProxmoxHmLowerNameIndex {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // sea_query Index::create() does not support expression columns
        // (functional indexes); raw SQL required.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX idx_proxmox_host_mappings_lower_name \
                 ON proxmox_host_mappings (lower(proxmox_name))",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_proxmox_host_mappings_lower_name")
                    .table(ProxmoxHostMappings::Table)
                    .to_owned(),
            )
            .await
    }
}

/// Return all controller-side migrations owned by the Proxmox plugin.
///
/// Migration names are hardcoded to match the original names from when
/// these lived in `crates/shared/db`, ensuring that existing databases
/// with `seaql_migrations` entries are not affected.
pub fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
    vec![
        Box::new(CreateProxmoxHostMappings),
        Box::new(AddProxmoxHmMachineId),
        Box::new(AddProxmoxHmPaginationIndexes),
        Box::new(AddProxmoxHmLowerNameIndex),
    ]
}
