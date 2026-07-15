//! Controller-side database migrations for the Proxmox infrastructure plugin.
//!
//! These migrations create and maintain the `proxmox_host_mappings` table on the
//! controller's database.  They are contributed to the controller's migration set
//! via [`PluginBase::controller_migrations()`](uptrakit_plugin_infrastructure_core::PluginBase::controller_migrations).
//!
//! Each migration uses a manual [`MigrationName`] implementation so that the
//! name recorded in `seaql_migrations` matches the original name from when
//! these migrations lived in `crates/shared/db`.

use sea_orm::{ConnectionTrait as _, TransactionTrait as _, TryGetable as _};
use sea_orm_migration::prelude::*;
use uuid::Uuid;

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
    SnapshotTimeoutSeconds,
    BackupTimeoutSeconds,
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
    SnapshotTimeoutSeconds,
    BackupTimeoutSeconds,
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
                        ColumnDef::new(ProxmoxProtectionDefaults::SnapshotTimeoutSeconds)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionDefaults::BackupTimeoutSeconds)
                            .big_integer()
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
                        ColumnDef::new(ProxmoxProtectionItemOverrides::SnapshotTimeoutSeconds)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxProtectionItemOverrides::BackupTimeoutSeconds)
                            .big_integer()
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

// ── Migration: add timeout columns to protection policy tables ───────────────

/// Add `snapshot_timeout_seconds` and `backup_timeout_seconds` columns to
/// `proxmox_protection_defaults` and `proxmox_protection_item_overrides`.
///
/// Uses `has_column` guards so the migration is a no-op on fresh databases
/// that were created after `CreateProxmoxProtectionPolicyTables` already
/// included these columns.
pub struct AddProxmoxProtectionTimeoutColumns;

impl MigrationName for AddProxmoxProtectionTimeoutColumns {
    fn name(&self) -> &str {
        "m20260426_000001_proxmox_protection_timeouts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddProxmoxProtectionTimeoutColumns {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_column("proxmox_protection_defaults", "snapshot_timeout_seconds")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(ProxmoxProtectionDefaults::Table)
                        .add_column(
                            ColumnDef::new(ProxmoxProtectionDefaults::SnapshotTimeoutSeconds)
                                .big_integer()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
        }

        if !manager
            .has_column("proxmox_protection_defaults", "backup_timeout_seconds")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(ProxmoxProtectionDefaults::Table)
                        .add_column(
                            ColumnDef::new(ProxmoxProtectionDefaults::BackupTimeoutSeconds)
                                .big_integer()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
        }

        if !manager
            .has_column(
                "proxmox_protection_item_overrides",
                "snapshot_timeout_seconds",
            )
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(ProxmoxProtectionItemOverrides::Table)
                        .add_column(
                            ColumnDef::new(ProxmoxProtectionItemOverrides::SnapshotTimeoutSeconds)
                                .big_integer()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
        }

        if !manager
            .has_column(
                "proxmox_protection_item_overrides",
                "backup_timeout_seconds",
            )
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(ProxmoxProtectionItemOverrides::Table)
                        .add_column(
                            ColumnDef::new(ProxmoxProtectionItemOverrides::BackupTimeoutSeconds)
                                .big_integer()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .drop_column(ProxmoxProtectionDefaults::SnapshotTimeoutSeconds)
                    .drop_column(ProxmoxProtectionDefaults::BackupTimeoutSeconds)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .drop_column(ProxmoxProtectionItemOverrides::SnapshotTimeoutSeconds)
                    .drop_column(ProxmoxProtectionItemOverrides::BackupTimeoutSeconds)
                    .to_owned(),
            )
            .await?;

        Ok(())
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

// ── Migration: add resource scaling policy columns ──────────────────────────

/// Add `update_cores` and `update_memory_mb` to both Proxmox protection policy
/// tables. NULL means no scaling configured for that policy row.
pub struct AddProxmoxResourceScalingPolicyColumns;

impl MigrationName for AddProxmoxResourceScalingPolicyColumns {
    fn name(&self) -> &str {
        "m20260503_000001_proxmox_resource_scaling_policy"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddProxmoxResourceScalingPolicyColumns {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingPolicyCols::UpdateCores)
                            .integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                            .integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingPolicyCols::UpdateCores)
                            .integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                            .integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateCores)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateCores)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ProxmoxResourceScalingPolicyCols {
    UpdateCores,
    UpdateMemoryMb,
}

#[derive(DeriveIden)]
enum ProxmoxResourceScalingRecords {
    Table,
    UpdateHistoryId,
    TenantId,
    HostId,
    SoftwareItemId,
    PluginConfigId,
    MappingId,
    VmType,
    OriginalCores,
    OriginalMemoryMb,
    ScaledCores,
    ScaledMemoryMb,
    ScaleStatus,
    RestoreStatus,
    ErrorMessage,
    CreatedAt,
    UpdatedAt,
    ScalingModeUsed,
}

// ── Scaling tables DeriveIden enums ────────────────────────────────────────

#[derive(DeriveIden)]
enum ProxmoxScalingDefaults {
    Table,
    Id,
    TenantId,
    PluginConfigId,
    ScalingMode,
    AbsoluteCores,
    AbsoluteMemoryMb,
    DeltaCores,
    DeltaMemoryMb,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ProxmoxScalingItemOverrides {
    Table,
    Id,
    TenantId,
    SoftwareItemId,
    PluginConfigId,
    ScalingMode,
    AbsoluteCores,
    AbsoluteMemoryMb,
    DeltaCores,
    DeltaMemoryMb,
    CreatedAt,
    UpdatedAt,
}

// ── Migration A: create proxmox_scaling_defaults ────────────────────────────

pub struct CreateProxmoxScalingDefaults;

impl MigrationName for CreateProxmoxScalingDefaults {
    fn name(&self) -> &str {
        "m20260504_000001_proxmox_scaling_defaults"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxScalingDefaults {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Uuid columns use the `.uuid()` builder (uuid_text on SQLite, uuid on
        // Postgres) like every sibling proxmox table — the entity layer binds
        // Value::Uuid, which a raw TEXT column rejects on Postgres.
        // Deliberately no foreign keys: the shipped schema had none, and adding
        // them now would drift from already-applied SQLite databases (which
        // never re-run this migration).
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxScalingDefaults::Table)
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::ScalingMode)
                            .string_len(16)
                            .not_null()
                            .default("none"),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::AbsoluteCores)
                            .integer()
                            .null()
                            .check(
                                Expr::col(ProxmoxScalingDefaults::AbsoluteCores)
                                    .is_null()
                                    .or(Expr::col(ProxmoxScalingDefaults::AbsoluteCores).gte(1)),
                            ),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::AbsoluteMemoryMb)
                            .integer()
                            .null()
                            .check(
                                Expr::col(ProxmoxScalingDefaults::AbsoluteMemoryMb)
                                    .is_null()
                                    .or(Expr::col(ProxmoxScalingDefaults::AbsoluteMemoryMb).gte(1)),
                            ),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::DeltaCores)
                            .integer()
                            .null()
                            .check(
                                Expr::col(ProxmoxScalingDefaults::DeltaCores)
                                    .is_null()
                                    .or(Expr::col(ProxmoxScalingDefaults::DeltaCores).gte(1)),
                            ),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::DeltaMemoryMb)
                            .integer()
                            .null()
                            .check(
                                Expr::col(ProxmoxScalingDefaults::DeltaMemoryMb)
                                    .is_null()
                                    .or(Expr::col(ProxmoxScalingDefaults::DeltaMemoryMb).gte(1)),
                            ),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingDefaults::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Preserves the shipped `UNIQUE (tenant_id, plugin_config_id)`.
        manager
            .create_index(
                Index::create()
                    .name("uq_proxmox_scaling_defaults_tenant_config")
                    .table(ProxmoxScalingDefaults::Table)
                    .col(ProxmoxScalingDefaults::TenantId)
                    .col(ProxmoxScalingDefaults::PluginConfigId)
                    .unique()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxScalingDefaults::Table)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration B: create proxmox_scaling_item_overrides ──────────────────────

pub struct CreateProxmoxScalingItemOverrides;

impl MigrationName for CreateProxmoxScalingItemOverrides {
    fn name(&self) -> &str {
        "m20260504_000002_proxmox_scaling_item_overrides"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxScalingItemOverrides {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Uuid columns use the `.uuid()` builder (uuid_text on SQLite, uuid on
        // Postgres) like every sibling proxmox table — the entity layer binds
        // Value::Uuid, which a raw TEXT column rejects on Postgres.
        // Deliberately no foreign keys: the shipped schema had none, and adding
        // them now would drift from already-applied SQLite databases (which
        // never re-run this migration).
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxScalingItemOverrides::Table)
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::ScalingMode)
                            .string_len(16)
                            .not_null()
                            .default("none"),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::AbsoluteCores)
                            .integer()
                            .null()
                            .check(
                                Expr::col(ProxmoxScalingItemOverrides::AbsoluteCores)
                                    .is_null()
                                    .or(Expr::col(ProxmoxScalingItemOverrides::AbsoluteCores)
                                        .gte(1)),
                            ),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::AbsoluteMemoryMb)
                            .integer()
                            .null()
                            .check(
                                Expr::col(ProxmoxScalingItemOverrides::AbsoluteMemoryMb)
                                    .is_null()
                                    .or(Expr::col(ProxmoxScalingItemOverrides::AbsoluteMemoryMb)
                                        .gte(1)),
                            ),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::DeltaCores)
                            .integer()
                            .null()
                            .check(
                                Expr::col(ProxmoxScalingItemOverrides::DeltaCores)
                                    .is_null()
                                    .or(Expr::col(ProxmoxScalingItemOverrides::DeltaCores).gte(1)),
                            ),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::DeltaMemoryMb)
                            .integer()
                            .null()
                            .check(
                                Expr::col(ProxmoxScalingItemOverrides::DeltaMemoryMb)
                                    .is_null()
                                    .or(Expr::col(ProxmoxScalingItemOverrides::DeltaMemoryMb)
                                        .gte(1)),
                            ),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxScalingItemOverrides::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_proxmox_scaling_item_overrides_item_config")
                    .table(ProxmoxScalingItemOverrides::Table)
                    .col(ProxmoxScalingItemOverrides::SoftwareItemId)
                    .col(ProxmoxScalingItemOverrides::PluginConfigId)
                    .unique()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxScalingItemOverrides::Table)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration C: migrate scaling config from protection tables ──────────────

pub struct MigrateProxmoxScalingFromProtectionTables;

impl MigrationName for MigrateProxmoxScalingFromProtectionTables {
    fn name(&self) -> &str {
        "m20260504_000003_migrate_scaling_from_protection_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for MigrateProxmoxScalingFromProtectionTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Wrap all five statements in an explicit transaction.
        // Without this, if C.2 fails after C.1 succeeds, the migration is permanently
        // broken: on retry, C.1 hits the UNIQUE constraint and fails again with no recovery path.
        let txn = manager.get_connection().begin().await?;

        // C.1 — copy proxmox_protection_defaults → proxmox_scaling_defaults.
        // Ids are generated in Rust (Uuid::now_v7()) and bound as Value::Uuid:
        // the previous SQLite-only random-blob-hex id concatenation trick
        // fails to parse on Postgres, and a .to_string() text id is
        // unreadable through sqlx's blob-only SQLite uuid codec. Columns are
        // read by ordinal index
        // against the explicit SELECT order. Deliberate exception to the
        // batch-queries rule: this runs once at migration time over a bounded
        // handful of rows, and the per-row insert is the empty-batch guard (a
        // zero-row Query::insert() emits INSERT with no VALUES clause — a
        // syntax error on both backends).
        let defaults_select = Query::select()
            .column(Alias::new("tenant_id"))
            .column(Alias::new("plugin_config_id"))
            .column(Alias::new("update_cores"))
            .column(Alias::new("update_memory_mb"))
            .column(Alias::new("created_at"))
            .column(Alias::new("updated_at"))
            .from(Alias::new("proxmox_protection_defaults"))
            .and_where(
                Expr::col(Alias::new("update_cores"))
                    .is_not_null()
                    .or(Expr::col(Alias::new("update_memory_mb")).is_not_null()),
            )
            .to_owned();
        let default_rows = txn.query_all(&defaults_select).await?;
        for row in &default_rows {
            let tenant_id = Uuid::try_get_by_index(row, 0)
                .map_err(|e| DbErr::Custom(format!("migration C.1: read tenant_id: {e:?}")))?;
            let plugin_config_id = Uuid::try_get_by_index(row, 1).map_err(|e| {
                DbErr::Custom(format!("migration C.1: read plugin_config_id: {e:?}"))
            })?;
            let update_cores = Option::<i32>::try_get_by_index(row, 2)
                .map_err(|e| DbErr::Custom(format!("migration C.1: read update_cores: {e:?}")))?;
            let update_memory_mb = Option::<i32>::try_get_by_index(row, 3).map_err(|e| {
                DbErr::Custom(format!("migration C.1: read update_memory_mb: {e:?}"))
            })?;
            let created_at = time::OffsetDateTime::try_get_by_index(row, 4)
                .map_err(|e| DbErr::Custom(format!("migration C.1: read created_at: {e:?}")))?;
            let updated_at = time::OffsetDateTime::try_get_by_index(row, 5)
                .map_err(|e| DbErr::Custom(format!("migration C.1: read updated_at: {e:?}")))?;

            txn.execute(
                &Query::insert()
                    .into_table(ProxmoxScalingDefaults::Table)
                    .columns([
                        ProxmoxScalingDefaults::Id,
                        ProxmoxScalingDefaults::TenantId,
                        ProxmoxScalingDefaults::PluginConfigId,
                        ProxmoxScalingDefaults::ScalingMode,
                        ProxmoxScalingDefaults::AbsoluteCores,
                        ProxmoxScalingDefaults::AbsoluteMemoryMb,
                        ProxmoxScalingDefaults::DeltaCores,
                        ProxmoxScalingDefaults::DeltaMemoryMb,
                        ProxmoxScalingDefaults::CreatedAt,
                        ProxmoxScalingDefaults::UpdatedAt,
                    ])
                    .values_panic([
                        Uuid::now_v7().into(),
                        tenant_id.into(),
                        plugin_config_id.into(),
                        "absolute".into(),
                        update_cores.into(),
                        update_memory_mb.into(),
                        Option::<i32>::None.into(),
                        Option::<i32>::None.into(),
                        created_at.into(),
                        updated_at.into(),
                    ])
                    .to_owned(),
            )
            .await?;
        }

        // C.2 — copy proxmox_protection_item_overrides → proxmox_scaling_item_overrides.
        // tenant_id is resolved by joining plugin_configs (as the original
        // SELECT did). Both joined tables expose id/created_at/updated_at, so
        // every column is read by ordinal index, never by name.
        let overrides_select = Query::select()
            .expr(Expr::col((Alias::new("pc"), Alias::new("tenant_id"))))
            .expr(Expr::col((
                Alias::new("pio"),
                Alias::new("software_item_id"),
            )))
            .expr(Expr::col((
                Alias::new("pio"),
                Alias::new("plugin_config_id"),
            )))
            .expr(Expr::col((Alias::new("pio"), Alias::new("update_cores"))))
            .expr(Expr::col((
                Alias::new("pio"),
                Alias::new("update_memory_mb"),
            )))
            .expr(Expr::col((Alias::new("pio"), Alias::new("created_at"))))
            .expr(Expr::col((Alias::new("pio"), Alias::new("updated_at"))))
            .from_as(
                Alias::new("proxmox_protection_item_overrides"),
                Alias::new("pio"),
            )
            .join_as(
                JoinType::InnerJoin,
                Alias::new("plugin_configs"),
                Alias::new("pc"),
                Expr::col((Alias::new("pc"), Alias::new("id")))
                    .equals((Alias::new("pio"), Alias::new("plugin_config_id"))),
            )
            .and_where(
                Expr::col((Alias::new("pio"), Alias::new("update_cores")))
                    .is_not_null()
                    .or(
                        Expr::col((Alias::new("pio"), Alias::new("update_memory_mb")))
                            .is_not_null(),
                    ),
            )
            .to_owned();
        let override_rows = txn.query_all(&overrides_select).await?;
        for row in &override_rows {
            let tenant_id = Uuid::try_get_by_index(row, 0)
                .map_err(|e| DbErr::Custom(format!("migration C.2: read tenant_id: {e:?}")))?;
            let software_item_id = Uuid::try_get_by_index(row, 1).map_err(|e| {
                DbErr::Custom(format!("migration C.2: read software_item_id: {e:?}"))
            })?;
            let plugin_config_id = Uuid::try_get_by_index(row, 2).map_err(|e| {
                DbErr::Custom(format!("migration C.2: read plugin_config_id: {e:?}"))
            })?;
            let update_cores = Option::<i32>::try_get_by_index(row, 3)
                .map_err(|e| DbErr::Custom(format!("migration C.2: read update_cores: {e:?}")))?;
            let update_memory_mb = Option::<i32>::try_get_by_index(row, 4).map_err(|e| {
                DbErr::Custom(format!("migration C.2: read update_memory_mb: {e:?}"))
            })?;
            let created_at = time::OffsetDateTime::try_get_by_index(row, 5)
                .map_err(|e| DbErr::Custom(format!("migration C.2: read created_at: {e:?}")))?;
            let updated_at = time::OffsetDateTime::try_get_by_index(row, 6)
                .map_err(|e| DbErr::Custom(format!("migration C.2: read updated_at: {e:?}")))?;

            txn.execute(
                &Query::insert()
                    .into_table(ProxmoxScalingItemOverrides::Table)
                    .columns([
                        ProxmoxScalingItemOverrides::Id,
                        ProxmoxScalingItemOverrides::TenantId,
                        ProxmoxScalingItemOverrides::SoftwareItemId,
                        ProxmoxScalingItemOverrides::PluginConfigId,
                        ProxmoxScalingItemOverrides::ScalingMode,
                        ProxmoxScalingItemOverrides::AbsoluteCores,
                        ProxmoxScalingItemOverrides::AbsoluteMemoryMb,
                        ProxmoxScalingItemOverrides::DeltaCores,
                        ProxmoxScalingItemOverrides::DeltaMemoryMb,
                        ProxmoxScalingItemOverrides::CreatedAt,
                        ProxmoxScalingItemOverrides::UpdatedAt,
                    ])
                    .values_panic([
                        Uuid::now_v7().into(),
                        tenant_id.into(),
                        software_item_id.into(),
                        plugin_config_id.into(),
                        "absolute".into(),
                        update_cores.into(),
                        update_memory_mb.into(),
                        Option::<i32>::None.into(),
                        Option::<i32>::None.into(),
                        created_at.into(),
                        updated_at.into(),
                    ])
                    .to_owned(),
            )
            .await?;
        }

        // C.3 — null out source columns (D will drop them; C leaves DB coherent if D fails)
        txn.execute_unprepared(
            "UPDATE proxmox_protection_defaults SET update_cores = NULL, update_memory_mb = NULL",
        )
        .await?;
        txn.execute_unprepared(
            "UPDATE proxmox_protection_item_overrides SET update_cores = NULL, update_memory_mb = NULL",
        )
        .await?;

        txn.commit().await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DELETE FROM proxmox_scaling_defaults")
            .await?;
        manager
            .get_connection()
            .execute_unprepared("DELETE FROM proxmox_scaling_item_overrides")
            .await?;
        Ok(())
    }
}

// ── Migration D: drop scaling columns from protection tables ────────────────

pub struct DropProxmoxScalingColumnsFromProtectionTables;

impl MigrationName for DropProxmoxScalingColumnsFromProtectionTables {
    fn name(&self) -> &str {
        "m20260504_000004_drop_scaling_columns_from_protection_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for DropProxmoxScalingColumnsFromProtectionTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateCores)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionDefaults::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateCores)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .drop_column(ProxmoxResourceScalingPolicyCols::UpdateMemoryMb)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

// ── Migration E: add scaling_mode_used to scaling records ──────────────────

pub struct AddScalingModeUsedToScalingRecord;

impl MigrationName for AddScalingModeUsedToScalingRecord {
    fn name(&self) -> &str {
        "m20260504_000005_add_scaling_mode_used_to_scaling_record"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddScalingModeUsedToScalingRecord {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxResourceScalingRecords::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ScalingModeUsed)
                            .string_len(16)
                            .not_null()
                            .default("absolute"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxResourceScalingRecords::Table)
                    .drop_column(ProxmoxResourceScalingRecords::ScalingModeUsed)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration: create proxmox_resource_scaling_records ─────────────────────

pub struct CreateProxmoxResourceScalingRecord;

impl MigrationName for CreateProxmoxResourceScalingRecord {
    fn name(&self) -> &str {
        "m20260503_000002_proxmox_resource_scaling_record"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxResourceScalingRecord {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxResourceScalingRecords::Table)
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::UpdateHistoryId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::HostId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::MappingId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::VmType)
                            .string_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::OriginalCores)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::OriginalMemoryMb)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ScaledCores)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ScaledMemoryMb)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ScaleStatus)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::RestoreStatus)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::ErrorMessage)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxResourceScalingRecords::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxResourceScalingRecords::Table)
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
        Box::new(AddProxmoxProtectionTimeoutColumns),
        Box::new(CreateProxmoxProtectionAudit),
        Box::new(AddProxmoxHmUniqueHostIdIndex),
        Box::new(ProxmoxHmVmidUniquePerConfig),
        Box::new(AddProxmoxResourceScalingPolicyColumns),
        Box::new(CreateProxmoxResourceScalingRecord),
        Box::new(CreateProxmoxScalingDefaults),
        Box::new(CreateProxmoxScalingItemOverrides),
        Box::new(MigrateProxmoxScalingFromProtectionTables),
        Box::new(DropProxmoxScalingColumnsFromProtectionTables),
        Box::new(AddScalingModeUsedToScalingRecord),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    use sea_orm_migration::{MigrationTrait, SchemaManager};

    async fn column_names(db: &sea_orm::DatabaseConnection, table: &str) -> Vec<String> {
        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA table_info({table})"),
            ))
            .await
            .unwrap();
        rows.into_iter()
            .map(|row| String::try_get(&row, "", "name").unwrap())
            .collect()
    }

    async fn column_decl_types(
        db: &sea_orm::DatabaseConnection,
        table: &str,
    ) -> std::collections::HashMap<String, String> {
        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA table_info({table})"),
            ))
            .await
            .unwrap();
        rows.into_iter()
            .map(|row| {
                (
                    String::try_get(&row, "", "name").unwrap(),
                    String::try_get(&row, "", "type").unwrap(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn forward_timeout_migration_is_noop_after_fresh_create() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);

        CreateProxmoxProtectionPolicyTables
            .up(&manager)
            .await
            .unwrap();
        AddProxmoxProtectionTimeoutColumns
            .up(&manager)
            .await
            .expect("forward migration must be safe on fresh DBs");

        let defaults = column_names(&db, "proxmox_protection_defaults").await;
        let overrides = column_names(&db, "proxmox_protection_item_overrides").await;

        assert!(defaults.contains(&"snapshot_timeout_seconds".to_string()));
        assert!(defaults.contains(&"backup_timeout_seconds".to_string()));
        assert!(overrides.contains(&"snapshot_timeout_seconds".to_string()));
        assert!(overrides.contains(&"backup_timeout_seconds".to_string()));
    }

    #[tokio::test]
    async fn migration_a_adds_update_cores_and_memory_mb_columns() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);

        // Run all existing migrations so the table exists
        CreateProxmoxProtectionPolicyTables
            .up(&manager)
            .await
            .unwrap();

        AddProxmoxResourceScalingPolicyColumns
            .up(&manager)
            .await
            .unwrap();

        let defaults_cols = column_names(&db, "proxmox_protection_defaults").await;
        assert!(defaults_cols.contains(&"update_cores".to_string()));
        assert!(defaults_cols.contains(&"update_memory_mb".to_string()));

        let overrides_cols = column_names(&db, "proxmox_protection_item_overrides").await;
        assert!(overrides_cols.contains(&"update_cores".to_string()));
        assert!(overrides_cols.contains(&"update_memory_mb".to_string()));
    }

    #[tokio::test]
    async fn migration_b_creates_scaling_records_table() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);

        CreateProxmoxResourceScalingRecord
            .up(&manager)
            .await
            .unwrap();

        let cols = column_names(&db, "proxmox_resource_scaling_records").await;
        for expected in &[
            "update_history_id",
            "tenant_id",
            "host_id",
            "software_item_id",
            "plugin_config_id",
            "mapping_id",
            "vm_type",
            "original_cores",
            "original_memory_mb",
            "scaled_cores",
            "scaled_memory_mb",
            "scale_status",
            "restore_status",
            "error_message",
            "created_at",
            "updated_at",
        ] {
            assert!(
                cols.contains(&ToString::to_string(expected)),
                "missing column: {expected}"
            );
        }
    }

    #[tokio::test]
    async fn scaling_tables_declare_uuid_typed_id_columns() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        CreateProxmoxScalingDefaults.up(&manager).await.unwrap();
        CreateProxmoxScalingItemOverrides
            .up(&manager)
            .await
            .unwrap();

        // sea_query renders ColumnType::Uuid as `uuid_text` on SQLite; the raw
        // legacy declaration was `TEXT`. Postgres gets a real `uuid` column.
        // The INTENT of this assertion is "no longer raw TEXT". If the rendered
        // token differs from `uuid_text`, confirm the actual token in vendored
        // sea-query-1.0.1/src/backend/sqlite/table.rs (ColumnType::Uuid arm)
        // and pin the assertion to that token — do not weaken it to a
        // not-equals-TEXT check.
        let defaults = column_decl_types(&db, "proxmox_scaling_defaults").await;
        for col in ["id", "tenant_id", "plugin_config_id"] {
            assert!(
                defaults[col].eq_ignore_ascii_case("uuid_text"),
                "proxmox_scaling_defaults.{col} declared {} — want uuid_text",
                defaults[col]
            );
        }
        let overrides = column_decl_types(&db, "proxmox_scaling_item_overrides").await;
        for col in ["id", "tenant_id", "software_item_id", "plugin_config_id"] {
            assert!(
                overrides[col].eq_ignore_ascii_case("uuid_text"),
                "proxmox_scaling_item_overrides.{col} declared {} — want uuid_text",
                overrides[col]
            );
        }
    }

    #[tokio::test]
    async fn scaling_store_roundtrips_on_builder_schema() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        CreateProxmoxScalingDefaults.up(&manager).await.unwrap();

        let tenant_id = uuid::Uuid::now_v7();
        let plugin_config_id = uuid::Uuid::now_v7();
        let policy = crate::scaling_store::ScalingPolicy {
            mode: crate::scaling_mode::ScalingMode::Absolute,
            absolute_cores: Some(4),
            ..Default::default()
        };

        crate::scaling_store::upsert_scaling_global_default(
            &db,
            tenant_id,
            plugin_config_id,
            &policy,
        )
        .await
        .expect("runtime upsert must work against the builder schema");
        let loaded =
            crate::scaling_store::load_scaling_global_default(&db, tenant_id, plugin_config_id)
                .await
                .expect("entity read-back must decode");
        assert_eq!(loaded.absolute_cores, Some(4));
        assert_eq!(loaded.mode, crate::scaling_mode::ScalingMode::Absolute);
    }

    #[tokio::test]
    async fn migration_new_a_creates_proxmox_scaling_defaults_table() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);

        CreateProxmoxScalingDefaults.up(&manager).await.unwrap();

        let cols = column_names(&db, "proxmox_scaling_defaults").await;
        for expected in &[
            "id",
            "tenant_id",
            "plugin_config_id",
            "scaling_mode",
            "absolute_cores",
            "absolute_memory_mb",
            "delta_cores",
            "delta_memory_mb",
            "created_at",
            "updated_at",
        ] {
            assert!(
                cols.contains(&ToString::to_string(expected)),
                "missing column: {expected}"
            );
        }
    }

    #[tokio::test]
    async fn migration_new_a_check_constraint_rejects_zero_absolute_cores() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        CreateProxmoxScalingDefaults.up(&manager).await.unwrap();

        let tid = "00000000-0000-0000-0000-000000000001";
        let cid = "00000000-0000-0000-0000-000000000002";
        let id = "00000000-0000-0000-0000-000000000003";
        let result = db
            .execute_unprepared(&format!(
                "INSERT INTO proxmox_scaling_defaults \
                 (id, tenant_id, plugin_config_id, scaling_mode, absolute_cores, created_at, updated_at) \
                 VALUES ('{id}', '{tid}', '{cid}', 'absolute', 0, '2026-01-01', '2026-01-01')"
            ))
            .await;
        assert!(
            result.is_err(),
            "CHECK constraint should reject absolute_cores = 0"
        );
    }

    #[tokio::test]
    async fn migration_new_b_creates_proxmox_scaling_item_overrides_table() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);

        CreateProxmoxScalingItemOverrides
            .up(&manager)
            .await
            .unwrap();

        let cols = column_names(&db, "proxmox_scaling_item_overrides").await;
        for expected in &[
            "id",
            "tenant_id",
            "software_item_id",
            "plugin_config_id",
            "scaling_mode",
            "absolute_cores",
            "absolute_memory_mb",
            "delta_cores",
            "delta_memory_mb",
            "created_at",
            "updated_at",
        ] {
            assert!(
                cols.contains(&ToString::to_string(expected)),
                "missing column: {expected}"
            );
        }
    }

    /// Shared setup for Migration C / repair tests: in-memory DB, stub parent
    /// tables (FK targets of the protection tables), and all prerequisite
    /// migrations through A/B. Seeds no rows.
    async fn scaling_migration_test_db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE IF NOT EXISTS tenants (id TEXT PRIMARY KEY)")
            .await
            .unwrap();
        db.execute_unprepared("CREATE TABLE IF NOT EXISTS software_items (id TEXT PRIMARY KEY)")
            .await
            .unwrap();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS plugin_configs \
             (id TEXT PRIMARY KEY, tenant_id TEXT, name TEXT, plugin_type TEXT, \
              config TEXT, created_at TEXT, updated_at TEXT)",
        )
        .await
        .unwrap();
        let manager = SchemaManager::new(&db);
        CreateProxmoxProtectionPolicyTables
            .up(&manager)
            .await
            .unwrap();
        AddProxmoxProtectionTimeoutColumns
            .up(&manager)
            .await
            .unwrap();
        AddProxmoxResourceScalingPolicyColumns
            .up(&manager)
            .await
            .unwrap();
        CreateProxmoxScalingDefaults.up(&manager).await.unwrap();
        CreateProxmoxScalingItemOverrides
            .up(&manager)
            .await
            .unwrap();
        db
    }

    #[tokio::test]
    async fn migration_c_transfers_protection_rows_to_scaling_tables() {
        let db = scaling_migration_test_db().await;
        let manager = SchemaManager::new(&db);

        // Production-parity seeds: uuid cells bound as Value::Uuid (runtime
        // rows are blobs on SQLite); timestamps as OffsetDateTime values.
        // FK enforcement is ON, so parents get the same binds as children.
        let seeded_at = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let tenant_a = uuid::Uuid::now_v7();
        let tenant_b = uuid::Uuid::now_v7();
        let cfg_a = uuid::Uuid::now_v7();
        let cfg_b = uuid::Uuid::now_v7();

        for t in [tenant_a, tenant_b] {
            db.execute(
                &Query::insert()
                    .into_table(Alias::new("tenants"))
                    .columns([Alias::new("id")])
                    .values_panic([t.into()])
                    .to_owned(),
            )
            .await
            .unwrap();
        }
        for (c, t) in [(cfg_a, tenant_a), (cfg_b, tenant_b)] {
            db.execute(
                &Query::insert()
                    .into_table(Alias::new("plugin_configs"))
                    .columns([
                        Alias::new("id"),
                        Alias::new("tenant_id"),
                        Alias::new("name"),
                        Alias::new("plugin_type"),
                        Alias::new("config"),
                        Alias::new("created_at"),
                        Alias::new("updated_at"),
                    ])
                    .values_panic([
                        c.into(),
                        t.into(),
                        "cfg".into(),
                        "infrastructure_proxmox".into(),
                        "{}".into(),
                        seeded_at.into(),
                        seeded_at.into(),
                    ])
                    .to_owned(),
            )
            .await
            .unwrap();
        }
        // One scaling-bearing row, one null-only row (must NOT migrate).
        for (t, c, cores, mem) in [
            (tenant_a, cfg_a, Some(8i32), Some(4096i32)),
            (tenant_b, cfg_b, None, None),
        ] {
            db.execute(
                &Query::insert()
                    .into_table(Alias::new("proxmox_protection_defaults"))
                    .columns([
                        Alias::new("tenant_id"),
                        Alias::new("plugin_config_id"),
                        Alias::new("mode"),
                        Alias::new("update_cores"),
                        Alias::new("update_memory_mb"),
                        Alias::new("created_at"),
                        Alias::new("updated_at"),
                    ])
                    .values_panic([
                        t.into(),
                        c.into(),
                        "do_nothing".into(),
                        cores.into(),
                        mem.into(),
                        seeded_at.into(),
                        seeded_at.into(),
                    ])
                    .to_owned(),
            )
            .await
            .unwrap();
        }

        MigrateProxmoxScalingFromProtectionTables
            .up(&manager)
            .await
            .unwrap();

        // Assert through the entity — the production decode path.
        use sea_orm::EntityTrait as _;
        let migrated = crate::entity::proxmox_scaling_default::Entity::find()
            .all(&db)
            .await
            .expect("entity read must decode migrated rows");
        assert_eq!(migrated.len(), 1, "null-only row must not migrate");
        let row = &migrated[0];
        assert_eq!(row.id.get_version_num(), 7, "id must be a fresh v7 uuid");
        assert_eq!(row.tenant_id, tenant_a);
        assert_eq!(row.plugin_config_id, cfg_a);
        assert_eq!(row.scaling_mode, crate::scaling_mode::ScalingMode::Absolute);
        assert_eq!(row.absolute_cores, Some(8));
        assert_eq!(row.absolute_memory_mb, Some(4096));
        assert_eq!(row.delta_cores, None);
        assert_eq!(row.delta_memory_mb, None);
        assert_eq!(
            row.created_at, seeded_at,
            "created_at must round-trip exactly"
        );
        assert_eq!(
            row.updated_at, seeded_at,
            "updated_at must round-trip exactly"
        );

        // C.3 nulled the source columns (count non-null, no uuid comparison).
        let remaining: i64 = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS c FROM proxmox_protection_defaults \
                 WHERE update_cores IS NOT NULL OR update_memory_mb IS NOT NULL",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "c")
            .unwrap();
        assert_eq!(remaining, 0, "source scaling columns must be NULL'd by C.3");
    }

    #[tokio::test]
    async fn migration_c_transfers_item_override_rows_to_scaling_tables() {
        let db = scaling_migration_test_db().await;
        let manager = SchemaManager::new(&db);

        // Production-parity seeds, same binds as the defaults-table test
        // above. The plugin_configs tenant_id is the value the C.2 JOIN must
        // resolve — kept distinct from every other uuid in this test so a
        // JOIN bug (e.g. reading the wrong column) can't accidentally match.
        let seeded_at = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let tenant = uuid::Uuid::now_v7();
        let software_item = uuid::Uuid::now_v7();
        let plugin_config = uuid::Uuid::now_v7();

        db.execute(
            &Query::insert()
                .into_table(Alias::new("tenants"))
                .columns([Alias::new("id")])
                .values_panic([tenant.into()])
                .to_owned(),
        )
        .await
        .unwrap();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("software_items"))
                .columns([Alias::new("id")])
                .values_panic([software_item.into()])
                .to_owned(),
        )
        .await
        .unwrap();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("plugin_configs"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("name"),
                    Alias::new("plugin_type"),
                    Alias::new("config"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    plugin_config.into(),
                    tenant.into(),
                    "cfg".into(),
                    "infrastructure_proxmox".into(),
                    "{}".into(),
                    seeded_at.into(),
                    seeded_at.into(),
                ])
                .to_owned(),
        )
        .await
        .unwrap();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("proxmox_protection_item_overrides"))
                .columns([
                    Alias::new("software_item_id"),
                    Alias::new("plugin_config_id"),
                    Alias::new("mode"),
                    Alias::new("update_cores"),
                    Alias::new("update_memory_mb"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    software_item.into(),
                    plugin_config.into(),
                    "do_nothing".into(),
                    Some(4i32).into(),
                    Some(2048i32).into(),
                    seeded_at.into(),
                    seeded_at.into(),
                ])
                .to_owned(),
        )
        .await
        .unwrap();

        MigrateProxmoxScalingFromProtectionTables
            .up(&manager)
            .await
            .unwrap();

        use sea_orm::EntityTrait as _;
        let migrated = crate::entity::proxmox_scaling_item_override::Entity::find()
            .all(&db)
            .await
            .expect("entity read must decode migrated rows");
        assert_eq!(migrated.len(), 1, "one override row should be migrated");
        let row = &migrated[0];
        assert_eq!(row.id.get_version_num(), 7, "id must be a fresh v7 uuid");
        assert_eq!(row.tenant_id, tenant, "tenant_id must resolve via the JOIN");
        assert_eq!(row.software_item_id, software_item);
        assert_eq!(row.plugin_config_id, plugin_config);
        assert_eq!(row.scaling_mode, crate::scaling_mode::ScalingMode::Absolute);
        assert_eq!(row.absolute_cores, Some(4));
        assert_eq!(row.absolute_memory_mb, Some(2048));
        assert_eq!(row.delta_cores, None);
        assert_eq!(row.delta_memory_mb, None);
        assert_eq!(
            row.created_at, seeded_at,
            "created_at must round-trip exactly"
        );
        assert_eq!(
            row.updated_at, seeded_at,
            "updated_at must round-trip exactly"
        );
    }

    #[tokio::test]
    async fn migration_c_with_no_scaling_source_rows_succeeds_and_inserts_nothing() {
        // The empty-batch guard: with zero source rows the per-row loop never
        // builds an insert, so nothing emits an empty-VALUES statement (the
        // case that failed on Postgres at parse time with the old raw SQL).
        let db = scaling_migration_test_db().await;
        let manager = SchemaManager::new(&db);
        MigrateProxmoxScalingFromProtectionTables
            .up(&manager)
            .await
            .expect("migration must succeed with zero source rows");

        use sea_orm::EntityTrait as _;
        assert!(
            crate::entity::proxmox_scaling_default::Entity::find()
                .all(&db)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            crate::entity::proxmox_scaling_item_override::Entity::find()
                .all(&db)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn scaling_migrations_use_no_dialect_specific_sql() {
        // Tripwire against reintroducing backend-specific SQL. The banner
        // comments bracket the A/B and C migration blocks; first-find hits the
        // real banners (the test module sits after them in the file — the
        // assertion below pins that ordering so a future file reshuffle can't
        // silently retarget the slices).
        let source = include_str!("controller_migration.rs");
        let a_start = source
            .find("// ── Migration A:")
            .expect("Migration A banner");
        let c_start = source
            .find("// ── Migration C:")
            .expect("Migration C banner");
        let d_start = source
            .find("// ── Migration D:")
            .expect("Migration D banner");
        let tests_start = source.find("#[cfg(test)]").expect("test module marker");
        assert!(
            d_start < tests_start,
            "migration banners must precede the test module for the slices to be valid"
        );
        let ab_block = source
            .get(a_start..c_start)
            .expect("banner offsets are on ASCII comment boundaries");
        let c_block = source
            .get(c_start..d_start)
            .expect("banner offsets are on ASCII comment boundaries");
        assert!(
            !ab_block.contains("execute_unprepared"),
            "Migrations A/B must use sea_query builders, not raw SQL"
        );
        assert!(
            !c_block.contains("randomblob"),
            "Migration C must not use SQLite-only randomblob()"
        );
        assert!(
            !c_block.contains("hex("),
            "Migration C must not use SQLite-only hex()"
        );
    }

    #[tokio::test]
    async fn migration_d_drops_scaling_columns_from_protection_tables() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);

        // Create plugin_configs stub before protection tables (they have FK to plugin_configs)
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS plugin_configs \
             (id TEXT PRIMARY KEY, tenant_id TEXT, name TEXT, plugin_type TEXT, \
              config TEXT, created_at TEXT, updated_at TEXT)",
        )
        .await
        .unwrap();

        CreateProxmoxProtectionPolicyTables
            .up(&manager)
            .await
            .unwrap();
        AddProxmoxProtectionTimeoutColumns
            .up(&manager)
            .await
            .unwrap();
        AddProxmoxResourceScalingPolicyColumns
            .up(&manager)
            .await
            .unwrap();
        CreateProxmoxScalingDefaults.up(&manager).await.unwrap();
        CreateProxmoxScalingItemOverrides
            .up(&manager)
            .await
            .unwrap();
        MigrateProxmoxScalingFromProtectionTables
            .up(&manager)
            .await
            .unwrap();
        DropProxmoxScalingColumnsFromProtectionTables
            .up(&manager)
            .await
            .unwrap();

        let defaults = column_names(&db, "proxmox_protection_defaults").await;
        assert!(
            !defaults.contains(&"update_cores".to_string()),
            "update_cores must be dropped"
        );
        assert!(
            !defaults.contains(&"update_memory_mb".to_string()),
            "update_memory_mb must be dropped"
        );

        let overrides = column_names(&db, "proxmox_protection_item_overrides").await;
        assert!(
            !overrides.contains(&"update_cores".to_string()),
            "update_cores must be dropped"
        );
        assert!(
            !overrides.contains(&"update_memory_mb".to_string()),
            "update_memory_mb must be dropped"
        );
    }

    #[tokio::test]
    async fn migration_e_adds_scaling_mode_used_to_scaling_records() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);

        CreateProxmoxResourceScalingRecord
            .up(&manager)
            .await
            .unwrap();
        AddScalingModeUsedToScalingRecord
            .up(&manager)
            .await
            .unwrap();

        let cols = column_names(&db, "proxmox_resource_scaling_records").await;
        assert!(
            cols.contains(&"scaling_mode_used".to_string()),
            "scaling_mode_used must exist"
        );

        db.execute_unprepared(
            "INSERT INTO proxmox_resource_scaling_records \
             (update_history_id, tenant_id, host_id, software_item_id, plugin_config_id, \
              mapping_id, vm_type, original_cores, original_memory_mb, scaled_cores, \
              scaled_memory_mb, scale_status, restore_status, created_at, updated_at) \
             VALUES ('h1', 't1', 'h2', 's1', 'p1', 'm1', 'qemu', 4, 4096, 8, 8192, \
                     'scaled', 'pending', '2026-01-01', '2026-01-01')",
        )
        .await
        .unwrap();
        let mode: String = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT scaling_mode_used FROM proxmox_resource_scaling_records WHERE update_history_id = 'h1'",
            ))
            .await.unwrap().unwrap()
            .try_get("", "scaling_mode_used").unwrap();
        assert_eq!(mode, "absolute");
    }

    #[tokio::test]
    async fn forward_timeout_migration_upgrades_existing_schema() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);

        // Simulate old schema without timeout columns
        manager
            .create_table(
                sea_orm_migration::prelude::Table::create()
                    .table(ProxmoxProtectionDefaults::Table)
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionDefaults::TenantId,
                        )
                        .uuid()
                        .not_null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionDefaults::PluginConfigId,
                        )
                        .uuid()
                        .not_null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(ProxmoxProtectionDefaults::Mode)
                            .text()
                            .not_null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionDefaults::BackupTargetKey,
                        )
                        .text()
                        .null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionDefaults::CreatedAt,
                        )
                        .timestamp()
                        .not_null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionDefaults::UpdatedAt,
                        )
                        .timestamp()
                        .not_null(),
                    )
                    .to_owned(),
            )
            .await
            .unwrap();

        manager
            .create_table(
                sea_orm_migration::prelude::Table::create()
                    .table(ProxmoxProtectionItemOverrides::Table)
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionItemOverrides::SoftwareItemId,
                        )
                        .uuid()
                        .not_null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionItemOverrides::PluginConfigId,
                        )
                        .uuid()
                        .not_null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionItemOverrides::Mode,
                        )
                        .text()
                        .not_null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionItemOverrides::BackupTargetKey,
                        )
                        .text()
                        .null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionItemOverrides::CreatedAt,
                        )
                        .timestamp()
                        .not_null(),
                    )
                    .col(
                        sea_orm_migration::prelude::ColumnDef::new(
                            ProxmoxProtectionItemOverrides::UpdatedAt,
                        )
                        .timestamp()
                        .not_null(),
                    )
                    .to_owned(),
            )
            .await
            .unwrap();

        AddProxmoxProtectionTimeoutColumns
            .up(&manager)
            .await
            .expect("forward migration should upgrade existing schema");

        let defaults = column_names(&db, "proxmox_protection_defaults").await;
        let overrides = column_names(&db, "proxmox_protection_item_overrides").await;

        assert!(defaults.contains(&"snapshot_timeout_seconds".to_string()));
        assert!(defaults.contains(&"backup_timeout_seconds".to_string()));
        assert!(overrides.contains(&"snapshot_timeout_seconds".to_string()));
        assert!(overrides.contains(&"backup_timeout_seconds".to_string()));
    }
}
