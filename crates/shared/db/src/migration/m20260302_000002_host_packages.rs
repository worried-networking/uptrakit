use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

/// Create `host_packages`, `host_package_ignores`, and
/// `host_package_update_history` tables for per-host package tracking.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. host_packages
        manager
            .create_table(
                Table::create()
                    .table(HostPackages::Table)
                    .col(
                        ColumnDef::new(HostPackages::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(HostPackages::TenantId).uuid().not_null())
                    .col(ColumnDef::new(HostPackages::HostId).uuid().not_null())
                    .col(
                        ColumnDef::new(HostPackages::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string(HostPackages::PackageIdentifier))
                    .col(string(HostPackages::Name))
                    .col(string_null(HostPackages::InstalledVersion))
                    .col(timestamp_null(HostPackages::InstalledVersionDetectedAt))
                    .col(string_null(HostPackages::LatestVersion))
                    .col(timestamp_null(HostPackages::LatestVersionFetchedAt))
                    .col(
                        ColumnDef::new(HostPackages::LatestReleaseMetadata)
                            .json_binary()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HostPackages::UpdateCategory)
                            .text()
                            .not_null()
                            .default("unknown"),
                    )
                    .col(
                        ColumnDef::new(HostPackages::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(timestamp_null(HostPackages::LastCheckedAt))
                    .col(timestamp_null(HostPackages::LastUpdatedAt))
                    .col(timestamp(HostPackages::CreatedAt))
                    .col(timestamp(HostPackages::UpdatedAt))
                    .col(timestamp_null(HostPackages::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_packages_tenant_id")
                            .from(HostPackages::Table, HostPackages::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_packages_host_id")
                            .from(HostPackages::Table, HostPackages::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_packages_plugin_config_id")
                            .from(HostPackages::Table, HostPackages::PluginConfigId)
                            .to(PluginConfigs::Table, PluginConfigs::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Unique constraint: (host_id, plugin_config_id, package_identifier) WHERE deactivated_at IS NULL
        // SQLite doesn't support partial unique indexes via SeaORM, so we create a
        // regular unique index on the triple. The application layer enforces the
        // deactivated_at IS NULL condition.
        manager
            .create_index(
                Index::create()
                    .name("idx_hp_host_plugin_pkg")
                    .table(HostPackages::Table)
                    .col(HostPackages::HostId)
                    .col(HostPackages::PluginConfigId)
                    .col(HostPackages::PackageIdentifier)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hp_tenant_host")
                    .table(HostPackages::Table)
                    .col(HostPackages::TenantId)
                    .col(HostPackages::HostId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hp_host_enabled")
                    .table(HostPackages::Table)
                    .col(HostPackages::HostId)
                    .col(HostPackages::Enabled)
                    .to_owned(),
            )
            .await?;

        // 2. host_package_ignores
        manager
            .create_table(
                Table::create()
                    .table(HostPackageIgnores::Table)
                    .col(
                        ColumnDef::new(HostPackageIgnores::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(HostPackageIgnores::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(HostPackageIgnores::HostId).uuid().not_null())
                    .col(
                        ColumnDef::new(HostPackageIgnores::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string(HostPackageIgnores::PackageIdentifier))
                    .col(timestamp(HostPackageIgnores::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hpi_tenant_id")
                            .from(HostPackageIgnores::Table, HostPackageIgnores::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hpi_host_id")
                            .from(HostPackageIgnores::Table, HostPackageIgnores::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hpi_plugin_config_id")
                            .from(
                                HostPackageIgnores::Table,
                                HostPackageIgnores::PluginConfigId,
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
                    .name("idx_hpi_host_plugin_pkg")
                    .table(HostPackageIgnores::Table)
                    .col(HostPackageIgnores::HostId)
                    .col(HostPackageIgnores::PluginConfigId)
                    .col(HostPackageIgnores::PackageIdentifier)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // 3. host_package_update_history
        manager
            .create_table(
                Table::create()
                    .table(HpUpdateHistory::Table)
                    .col(
                        ColumnDef::new(HpUpdateHistory::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(HpUpdateHistory::TenantId).uuid().not_null())
                    .col(ColumnDef::new(HpUpdateHistory::HostId).uuid().not_null())
                    .col(
                        ColumnDef::new(HpUpdateHistory::HostPackageId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string_null(HpUpdateHistory::FromVersion))
                    .col(string_null(HpUpdateHistory::ToVersion))
                    .col(
                        ColumnDef::new(HpUpdateHistory::Status)
                            .text()
                            .not_null()
                            .default("pending"),
                    )
                    .col(ColumnDef::new(HpUpdateHistory::Output).text().null())
                    .col(
                        ColumnDef::new(HpUpdateHistory::OutputBytes)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(string(HpUpdateHistory::ActorType))
                    .col(string(HpUpdateHistory::ActorId))
                    .col(
                        ColumnDef::new(HpUpdateHistory::UpdateCategory)
                            .text()
                            .not_null()
                            .default("unknown"),
                    )
                    .col(timestamp_null(HpUpdateHistory::StartedAt))
                    .col(timestamp_null(HpUpdateHistory::CompletedAt))
                    .col(timestamp(HpUpdateHistory::CreatedAt))
                    .col(ColumnDef::new(HpUpdateHistory::BatchId).uuid().null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hpuh_tenant_id")
                            .from(HpUpdateHistory::Table, HpUpdateHistory::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hpuh_host_id")
                            .from(HpUpdateHistory::Table, HpUpdateHistory::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hpuh_host_package_id")
                            .from(HpUpdateHistory::Table, HpUpdateHistory::HostPackageId)
                            .to(HostPackages::Table, HostPackages::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hpuh_batch_id")
                            .from(HpUpdateHistory::Table, HpUpdateHistory::BatchId)
                            .to(UpdateBatches::Table, UpdateBatches::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hpuh_host_package_status")
                    .table(HpUpdateHistory::Table)
                    .col(HpUpdateHistory::HostPackageId)
                    .col(HpUpdateHistory::Status)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hpuh_batch_id")
                    .table(HpUpdateHistory::Table)
                    .col(HpUpdateHistory::BatchId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hpuh_tenant_host")
                    .table(HpUpdateHistory::Table)
                    .col(HpUpdateHistory::TenantId)
                    .col(HpUpdateHistory::HostId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(HpUpdateHistory::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(HostPackageIgnores::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(HostPackages::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum HostPackages {
    Table,
    Id,
    TenantId,
    HostId,
    PluginConfigId,
    PackageIdentifier,
    Name,
    InstalledVersion,
    InstalledVersionDetectedAt,
    LatestVersion,
    LatestVersionFetchedAt,
    LatestReleaseMetadata,
    UpdateCategory,
    Enabled,
    LastCheckedAt,
    LastUpdatedAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum HostPackageIgnores {
    Table,
    Id,
    TenantId,
    HostId,
    PluginConfigId,
    PackageIdentifier,
    CreatedAt,
}

#[derive(DeriveIden)]
enum HpUpdateHistory {
    #[sea_orm(iden = "host_package_update_history")]
    Table,
    Id,
    TenantId,
    HostId,
    HostPackageId,
    FromVersion,
    ToVersion,
    Status,
    Output,
    OutputBytes,
    ActorType,
    ActorId,
    UpdateCategory,
    StartedAt,
    CompletedAt,
    CreatedAt,
    BatchId,
}

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum PluginConfigs {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum UpdateBatches {
    Table,
    Id,
}
