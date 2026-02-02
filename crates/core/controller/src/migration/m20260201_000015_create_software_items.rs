use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m20260129_000001_initial::Tenants;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create software_items table
        manager
            .create_table(
                Table::create()
                    .table(SoftwareItems::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SoftwareItems::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SoftwareItems::TenantId).uuid().not_null())
                    .col(string(SoftwareItems::Name))
                    .col(
                        ColumnDef::new(SoftwareItems::ProviderConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string(SoftwareItems::PackageIdentifier).default(""))
                    .col(json_null(SoftwareItems::ConfigOverride))
                    .col(boolean(SoftwareItems::Enabled).default(true))
                    .col(timestamp_null(SoftwareItems::LastCheckedAt))
                    .col(timestamp(SoftwareItems::CreatedAt))
                    .col(timestamp(SoftwareItems::UpdatedAt))
                    .col(timestamp_null(SoftwareItems::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_software_items_tenant")
                            .from(SoftwareItems::Table, SoftwareItems::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_software_items_provider_config")
                            .from(SoftwareItems::Table, SoftwareItems::ProviderConfigId)
                            .to(ProviderConfigs::Table, ProviderConfigs::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Unique constraint: (tenant_id, provider_config_id, package_identifier)
        manager
            .create_index(
                Index::create()
                    .name("uq_software_items_tenant_provider_config_package")
                    .table(SoftwareItems::Table)
                    .col(SoftwareItems::TenantId)
                    .col(SoftwareItems::ProviderConfigId)
                    .col(SoftwareItems::PackageIdentifier)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Index on tenant_id
        manager
            .create_index(
                Index::create()
                    .name("idx_software_items_tenant_id")
                    .table(SoftwareItems::Table)
                    .col(SoftwareItems::TenantId)
                    .to_owned(),
            )
            .await?;

        // Index on provider_config_id for FK lookups and filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_software_items_provider_config_id")
                    .table(SoftwareItems::Table)
                    .col(SoftwareItems::ProviderConfigId)
                    .to_owned(),
            )
            .await?;

        // Index on deactivated_at for soft-delete filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_software_items_deactivated_at")
                    .table(SoftwareItems::Table)
                    .col(SoftwareItems::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // Create host_software_items junction table
        manager
            .create_table(
                Table::create()
                    .table(HostSoftwareItems::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(HostSoftwareItems::HostId).uuid().not_null())
                    .col(
                        ColumnDef::new(HostSoftwareItems::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string_null(HostSoftwareItems::InstalledVersion))
                    .col(timestamp_null(
                        HostSoftwareItems::InstalledVersionDetectedAt,
                    ))
                    .col(timestamp_null(HostSoftwareItems::LastUpdatedAt))
                    .col(timestamp(HostSoftwareItems::LinkedAt))
                    .primary_key(
                        Index::create()
                            .col(HostSoftwareItems::HostId)
                            .col(HostSoftwareItems::SoftwareItemId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_software_items_host")
                            .from(HostSoftwareItems::Table, HostSoftwareItems::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_software_items_software_item")
                            .from(HostSoftwareItems::Table, HostSoftwareItems::SoftwareItemId)
                            .to(SoftwareItems::Table, SoftwareItems::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create available_versions table
        manager
            .create_table(
                Table::create()
                    .table(AvailableVersions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AvailableVersions::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AvailableVersions::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string_null(AvailableVersions::Version))
                    .col(timestamp_null(AvailableVersions::ReleaseDate))
                    .col(
                        ColumnDef::new(AvailableVersions::ReleaseNotes)
                            .text()
                            .null(),
                    )
                    .col(json_null(AvailableVersions::Extra))
                    .col(timestamp(AvailableVersions::CreatedAt))
                    .col(timestamp(AvailableVersions::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_available_versions_software_item")
                            .from(AvailableVersions::Table, AvailableVersions::SoftwareItemId)
                            .to(SoftwareItems::Table, SoftwareItems::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .check(
                        Expr::col(AvailableVersions::Version)
                            .is_not_null()
                            .or(Expr::col(AvailableVersions::ReleaseDate).is_not_null()),
                    )
                    .to_owned(),
            )
            .await?;

        // Index on software_item_id for FK lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_available_versions_software_item_id")
                    .table(AvailableVersions::Table)
                    .col(AvailableVersions::SoftwareItemId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AvailableVersions::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(HostSoftwareItems::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(SoftwareItems::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum SoftwareItems {
    Table,
    Id,
    TenantId,
    Name,
    ProviderConfigId,
    PackageIdentifier,
    ConfigOverride,
    Enabled,
    LastCheckedAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum HostSoftwareItems {
    Table,
    HostId,
    SoftwareItemId,
    InstalledVersion,
    InstalledVersionDetectedAt,
    LastUpdatedAt,
    LinkedAt,
}

#[derive(DeriveIden)]
enum AvailableVersions {
    Table,
    Id,
    SoftwareItemId,
    Version,
    ReleaseDate,
    ReleaseNotes,
    Extra,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ProviderConfigs {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    Id,
}
