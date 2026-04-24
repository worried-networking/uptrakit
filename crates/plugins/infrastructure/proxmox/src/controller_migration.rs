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

#[derive(DeriveIden)]
enum ProxmoxBackupTargetCache {
    Table,
    Id,
    TenantId,
    PluginConfigId,
    ProxmoxNode,
    StorageId,
    StorageType,
    TargetKey,
    DiscoveredAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ProxmoxProtectionDefaults {
    Table,
    TenantId,
    PluginConfigId,
    Mode,
    BackupTargetKey,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ProxmoxProtectionItemOverrides {
    Table,
    SoftwareItemId,
    PluginConfigId,
    Mode,
    BackupTargetKey,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ProxmoxProtectionAudit {
    Table,
    UpdateHistoryId,
    TenantId,
    HostId,
    SoftwareItemId,
    PluginConfigId,
    MappingId,
    Mode,
    Status,
    ArtifactKind,
    ArtifactRef,
    BackupTargetKey,
    Detail,
    ErrorMessage,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum SoftwareItems {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum UpdateHistory {
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

// ── Migration: enforce one host per mapping row set ─────────────────────────

/// Ensure one `host_id` can be assigned to at most one Proxmox mapping row.
///
/// The migration first clears historical duplicate assignments (keeping the
/// most recently updated mapping per host) and then adds a unique index on
/// `host_id`. `host_id` is nullable; supported DBs allow multiple NULL values.
pub struct AddProxmoxHmUniqueHostIdIndex;

impl MigrationName for AddProxmoxHmUniqueHostIdIndex {
    fn name(&self) -> &str {
        "m20260417_000004_proxmox_hm_unique_host_id"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddProxmoxHmUniqueHostIdIndex {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Keep the newest mapping (updated_at desc, id desc) for each host_id
        // and clear host_id + match_method on older duplicates so the index
        // can be created safely.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE proxmox_host_mappings
                 SET host_id = NULL,
                     match_method = NULL
                 WHERE id IN (
                     SELECT id FROM (
                         SELECT id,
                                ROW_NUMBER() OVER (
                                    PARTITION BY host_id
                                    ORDER BY updated_at DESC, id DESC
                                ) AS rn
                         FROM proxmox_host_mappings
                         WHERE host_id IS NOT NULL
                     ) ranked
                     WHERE rn > 1
                 )",
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uix_proxmox_hm_host_unique")
                    .table(ProxmoxHostMappings::Table)
                    .col(ProxmoxHostMappings::HostId)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("uix_proxmox_hm_host_unique")
                    .table(ProxmoxHostMappings::Table)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration: backup target cache ──────────────────────────────────────────

/// Create cache table for node-aware backup targets discovered from Proxmox.
pub struct CreateProxmoxBackupTargetCache;

impl MigrationName for CreateProxmoxBackupTargetCache {
    fn name(&self) -> &str {
        "m20260417_000001_proxmox_backup_target_cache"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxBackupTargetCache {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxBackupTargetCache::Table)
                    .col(
                        ColumnDef::new(ProxmoxBackupTargetCache::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxBackupTargetCache::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxBackupTargetCache::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxBackupTargetCache::ProxmoxNode)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxBackupTargetCache::StorageId)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxBackupTargetCache::StorageType)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxBackupTargetCache::TargetKey)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxBackupTargetCache::DiscoveredAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxBackupTargetCache::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_btc_tenant")
                            .from(
                                ProxmoxBackupTargetCache::Table,
                                ProxmoxBackupTargetCache::TenantId,
                            )
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_btc_plugin_config")
                            .from(
                                ProxmoxBackupTargetCache::Table,
                                ProxmoxBackupTargetCache::PluginConfigId,
                            )
                            .to(PluginConfigs::Table, PluginConfigs::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uix_proxmox_btc_config_target")
                    .table(ProxmoxBackupTargetCache::Table)
                    .col(ProxmoxBackupTargetCache::PluginConfigId)
                    .col(ProxmoxBackupTargetCache::TargetKey)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_proxmox_btc_tenant")
                    .table(ProxmoxBackupTargetCache::Table)
                    .col(ProxmoxBackupTargetCache::TenantId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxBackupTargetCache::Table)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration: protection policy tables ─────────────────────────────────────

/// Create global/default and per-item override tables for Proxmox protection policies.
pub struct CreateProxmoxProtectionPolicyTables;

impl MigrationName for CreateProxmoxProtectionPolicyTables {
    fn name(&self) -> &str {
        "m20260417_000002_proxmox_protection_policy"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxProtectionPolicyTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxProtectionDefaults::Table)
                    .col(
                        ColumnDef::new(ProxmoxProtectionDefaults::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionDefaults::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionDefaults::Mode)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionDefaults::BackupTargetKey)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionDefaults::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionDefaults::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(ProxmoxProtectionDefaults::TenantId)
                            .col(ProxmoxProtectionDefaults::PluginConfigId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_pd_tenant")
                            .from(
                                ProxmoxProtectionDefaults::Table,
                                ProxmoxProtectionDefaults::TenantId,
                            )
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_pd_plugin_config")
                            .from(
                                ProxmoxProtectionDefaults::Table,
                                ProxmoxProtectionDefaults::PluginConfigId,
                            )
                            .to(PluginConfigs::Table, PluginConfigs::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .col(
                        ColumnDef::new(ProxmoxProtectionItemOverrides::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionItemOverrides::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionItemOverrides::Mode)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionItemOverrides::BackupTargetKey)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionItemOverrides::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionItemOverrides::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(ProxmoxProtectionItemOverrides::SoftwareItemId)
                            .col(ProxmoxProtectionItemOverrides::PluginConfigId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_pio_software_item")
                            .from(
                                ProxmoxProtectionItemOverrides::Table,
                                ProxmoxProtectionItemOverrides::SoftwareItemId,
                            )
                            .to(SoftwareItems::Table, SoftwareItems::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_pio_plugin_config")
                            .from(
                                ProxmoxProtectionItemOverrides::Table,
                                ProxmoxProtectionItemOverrides::PluginConfigId,
                            )
                            .to(PluginConfigs::Table, PluginConfigs::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_proxmox_pio_plugin_config")
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .col(ProxmoxProtectionItemOverrides::PluginConfigId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxProtectionDefaults::Table)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration: protection audit ─────────────────────────────────────────────

/// Create per-update protection audit rows keyed by `update_history_id`.
pub struct CreateProxmoxProtectionAudit;

impl MigrationName for CreateProxmoxProtectionAudit {
    fn name(&self) -> &str {
        "m20260417_000003_proxmox_protection_audit"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxProtectionAudit {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxProtectionAudit::Table)
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::UpdateHistoryId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::HostId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::MappingId)
                            .uuid()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::Mode)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::Status)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::ArtifactKind)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::ArtifactRef)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::BackupTargetKey)
                            .text()
                            .null(),
                    )
                    .col(ColumnDef::new(ProxmoxProtectionAudit::Detail).text().null())
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::ErrorMessage)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionAudit::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_pa_update_history")
                            .from(
                                ProxmoxProtectionAudit::Table,
                                ProxmoxProtectionAudit::UpdateHistoryId,
                            )
                            .to(UpdateHistory::Table, UpdateHistory::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_pa_plugin_config")
                            .from(
                                ProxmoxProtectionAudit::Table,
                                ProxmoxProtectionAudit::PluginConfigId,
                            )
                            .to(PluginConfigs::Table, PluginConfigs::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_proxmox_pa_mapping")
                            .from(
                                ProxmoxProtectionAudit::Table,
                                ProxmoxProtectionAudit::MappingId,
                            )
                            .to(ProxmoxHostMappings::Table, ProxmoxHostMappings::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_proxmox_pa_tenant")
                    .table(ProxmoxProtectionAudit::Table)
                    .col(ProxmoxProtectionAudit::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_proxmox_pa_plugin_config")
                    .table(ProxmoxProtectionAudit::Table)
                    .col(ProxmoxProtectionAudit::PluginConfigId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxProtectionAudit::Table)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration: vmid unique per config (node-agnostic) ───────────────────────

/// Change the unique constraint on `proxmox_host_mappings` from
/// `(plugin_config_id, proxmox_node, proxmox_vmid)` to
/// `(plugin_config_id, proxmox_vmid)`.
///
/// In Proxmox VE, VMIDs are unique cluster-wide regardless of which node
/// currently hosts the guest.  The old node-scoped constraint caused a new
/// row to be inserted whenever a guest was live-migrated to a different node,
/// duplicating the mapping and losing the host association.
///
/// The migration first removes any duplicate rows that arose from node
/// migrations (keeping the matched row, or the most recently updated one),
/// then replaces the unique index.
pub struct ProxmoxHmVmidUniquePerConfig;

impl MigrationName for ProxmoxHmVmidUniquePerConfig {
    fn name(&self) -> &str {
        "m20260424_000001_proxmox_vmid_unique_per_config"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ProxmoxHmVmidUniquePerConfig {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Remove duplicate rows that arose from guests migrating between nodes.
        // Keep the "best" row per (plugin_config_id, proxmox_vmid): prefer the
        // row with a host_id (already matched), then the most recently updated.
        conn.execute_unprepared(
            "DELETE FROM proxmox_host_mappings
             WHERE id NOT IN (
                 SELECT id FROM (
                     SELECT id,
                            ROW_NUMBER() OVER (
                                PARTITION BY plugin_config_id, proxmox_vmid
                                ORDER BY CASE WHEN host_id IS NOT NULL THEN 0 ELSE 1 END,
                                         updated_at DESC
                            ) AS rn
                     FROM proxmox_host_mappings
                 ) ranked
                 WHERE rn = 1
             )",
        )
        .await?;

        // Drop the old node-scoped unique index.
        manager
            .drop_index(
                Index::drop()
                    .name("uix_proxmox_hm_config_node_vmid")
                    .table(ProxmoxHostMappings::Table)
                    .to_owned(),
            )
            .await?;

        // Create cluster-scoped unique index.
        manager
            .create_index(
                Index::create()
                    .name("uix_proxmox_hm_config_vmid")
                    .table(ProxmoxHostMappings::Table)
                    .col(ProxmoxHostMappings::PluginConfigId)
                    .col(ProxmoxHostMappings::ProxmoxVmid)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("uix_proxmox_hm_config_vmid")
                    .table(ProxmoxHostMappings::Table)
                    .to_owned(),
            )
            .await?;

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
        Box::new(CreateProxmoxBackupTargetCache),
        Box::new(CreateProxmoxProtectionPolicyTables),
        Box::new(CreateProxmoxProtectionAudit),
        Box::new(AddProxmoxHmUniqueHostIdIndex),
        Box::new(ProxmoxHmVmidUniquePerConfig),
    ]
}
