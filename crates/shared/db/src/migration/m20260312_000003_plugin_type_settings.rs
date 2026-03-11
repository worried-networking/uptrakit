//! Migration: plugin type settings + host_software_item_plugins schema updates.
//!
//! ## Step A — Create `plugin_type_settings` table
//! Tenant-level per-type preferences (replaces auto-created package manager configs).
//!
//! ## Step B — Add `plugin_type` column to `host_software_item_plugins`
//! Denormalized from the joined `plugin_configs` row, so the column can be read
//! without joining when `plugin_config_id` is NULL.
//!
//! ## Step C — Make `plugin_config_id` nullable on `host_software_item_plugins`
//! Package manager assignments no longer require a `plugin_config` row.
//!
//! ## Step D — Rename `config_override` → `config` on `host_software_item_plugins`
//! Reflects that this column is a first-class config field, not just an override.
//!
//! ## Step E — Migrate auto-created package manager configs to `plugin_type_settings`
//! Moves discovery preferences to the new table and NULLs out the FK for package
//! manager assignments.

use sea_orm_migration::prelude::*;

use super::helpers::{self, CrashRecoveryState};

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Package manager plugin types whose auto-created configs should be migrated.
const PACKAGE_MANAGER_TYPES: &[&str] = &[
    "package_manager_apt",
    "package_manager_homebrew",
    "package_manager_pacman",
    "package_manager_apk",
    "package_manager_pkg",
    "package_manager_snap",
    "package_manager_dnf",
    "package_manager_mas",
    "package_manager_cargo",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ── Step A: Create plugin_type_settings ──────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(PluginTypeSettings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PluginTypeSettings::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PluginTypeSettings::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PluginTypeSettings::PluginType)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PluginTypeSettings::Config)
                            .json()
                            .not_null()
                            .default("{}"),
                    )
                    .col(
                        ColumnDef::new(PluginTypeSettings::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PluginTypeSettings::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_plugin_type_settings_tenant")
                            .from(PluginTypeSettings::Table, PluginTypeSettings::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_plugin_type_settings_tenant_type")
                    .table(PluginTypeSettings::Table)
                    .col(PluginTypeSettings::TenantId)
                    .col(PluginTypeSettings::PluginType)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_plugin_type_settings_tenant")
                    .table(PluginTypeSettings::Table)
                    .col(PluginTypeSettings::TenantId)
                    .to_owned(),
            )
            .await?;

        // ── Steps B+C+D: Alter host_software_item_plugins ───────────────
        // SQLite requires table recreation for column renames and nullability
        // changes. For Postgres/MySQL, ALTER TABLE suffices.
        if helpers::is_sqlite(manager) {
            self.recreate_hsip_sqlite(manager).await?;
        } else {
            self.alter_hsip_postgres(manager).await?;
        }

        // ── Step E: Migrate package manager configs ──────────────────────
        self.migrate_package_manager_configs(manager).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Reverse Step E is not practical (data already moved).
        // Reverse Steps B+C+D: restore original schema.
        if helpers::is_sqlite(manager) {
            self.reverse_recreate_hsip_sqlite(manager).await?;
        } else {
            self.reverse_alter_hsip_postgres(manager).await?;
        }

        // Reverse Step A.
        manager
            .drop_table(Table::drop().table(PluginTypeSettings::Table).to_owned())
            .await?;

        Ok(())
    }
}

impl Migration {
    // ── SQLite table recreation (Steps B+C+D) ────────────────────────────

    async fn recreate_hsip_sqlite(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        helpers::set_foreign_keys(manager, false).await?;

        let state = helpers::check_crash_recovery(
            manager,
            "host_software_item_plugins",
            "host_software_item_plugins_new",
        )
        .await?;

        if state == CrashRecoveryState::Normal {
            // Create the new table with updated schema.
            manager
                .create_table(
                    Table::create()
                        .table(HsipNew::Table)
                        .col(ColumnDef::new(HsipNew::Id).uuid().not_null().primary_key())
                        .col(ColumnDef::new(HsipNew::HostId).uuid().not_null())
                        .col(ColumnDef::new(HsipNew::SoftwareItemId).uuid().not_null())
                        .col(
                            ColumnDef::new(HsipNew::HostSoftwareItemId)
                                .uuid()
                                .not_null(),
                        )
                        .col(ColumnDef::new(HsipNew::PluginConfigId).uuid()) // nullable
                        .col(
                            ColumnDef::new(HsipNew::PluginType)
                                .string()
                                .not_null()
                                .default(""),
                        )
                        .col(ColumnDef::new(HsipNew::Role).string().not_null())
                        .col(
                            ColumnDef::new(HsipNew::Ordinal)
                                .integer()
                                .not_null()
                                .default(0),
                        )
                        .col(
                            ColumnDef::new(HsipNew::PackageIdentifier)
                                .string()
                                .not_null(),
                        )
                        .col(ColumnDef::new(HsipNew::Config).json()) // renamed from config_override
                        .col(
                            ColumnDef::new(HsipNew::ExecutionSite)
                                .string()
                                .not_null()
                                .default("auto"),
                        )
                        .col(ColumnDef::new(HsipNew::CreatedAt).timestamp().not_null())
                        .col(ColumnDef::new(HsipNew::UpdatedAt).timestamp().not_null())
                        .foreign_key(
                            ForeignKey::create()
                                .name("fk_hsip_plugin_config")
                                .from(HsipNew::Table, HsipNew::PluginConfigId)
                                .to(PluginConfigs::Table, PluginConfigs::Id)
                                .on_delete(ForeignKeyAction::Restrict),
                        )
                        .to_owned(),
                )
                .await?;

            // Copy data from old table, populating plugin_type from joined plugin_configs.
            // `config_override` is renamed to `config` in the copy.
            // `execute_unprepared` is the approved pattern for complex INSERT...SELECT
            // with JOINs that sea_query cannot express.
            manager
                .get_connection()
                .execute_unprepared(
                    "INSERT INTO host_software_item_plugins_new \
                     (id, host_id, software_item_id, host_software_item_id, \
                      plugin_config_id, plugin_type, role, ordinal, \
                      package_identifier, config, execution_site, created_at, updated_at) \
                     SELECT \
                       hsip.id, hsip.host_id, hsip.software_item_id, hsip.host_software_item_id, \
                       hsip.plugin_config_id, \
                       COALESCE(pc.plugin_type, ''), \
                       hsip.role, hsip.ordinal, \
                       hsip.package_identifier, hsip.config_override, \
                       hsip.execution_site, hsip.created_at, hsip.updated_at \
                     FROM host_software_item_plugins hsip \
                     LEFT JOIN plugin_configs pc ON pc.id = hsip.plugin_config_id",
                )
                .await?;

            helpers::drop_original(manager, "host_software_item_plugins").await?;
        }

        helpers::rename_temp(
            manager,
            "host_software_item_plugins_new",
            "host_software_item_plugins",
        )
        .await?;

        // Recreate indexes.
        self.create_hsip_indexes(manager).await?;

        helpers::set_foreign_keys(manager, true).await?;
        Ok(())
    }

    async fn create_hsip_indexes(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("uq_hsip_host_item_role_ordinal")
                    .table(Hsip::Table)
                    .col(Hsip::HostId)
                    .col(Hsip::SoftwareItemId)
                    .col(Hsip::Role)
                    .col(Hsip::Ordinal)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hsip_plugin_config_id")
                    .table(Hsip::Table)
                    .col(Hsip::PluginConfigId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hsip_host_item")
                    .table(Hsip::Table)
                    .col(Hsip::HostId)
                    .col(Hsip::SoftwareItemId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hsip_role_exec")
                    .table(Hsip::Table)
                    .col(Hsip::Role)
                    .col(Hsip::ExecutionSite)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    // ── Postgres/MySQL ALTER TABLE (Steps B+C+D) ─────────────────────────

    async fn alter_hsip_postgres(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        // Step B: Add plugin_type column.
        manager
            .alter_table(
                Table::alter()
                    .table(Hsip::Table)
                    .add_column(
                        ColumnDef::new(Hsip::PluginType)
                            .string()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await?;

        // Populate plugin_type from joined plugin_configs.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE host_software_item_plugins \
                 SET plugin_type = COALESCE(( \
                     SELECT pc.plugin_type \
                     FROM plugin_configs pc \
                     WHERE pc.id = host_software_item_plugins.plugin_config_id \
                 ), '')",
            )
            .await?;

        // Step C: Make plugin_config_id nullable.
        manager
            .alter_table(
                Table::alter()
                    .table(Hsip::Table)
                    .modify_column(ColumnDef::new(Hsip::PluginConfigId).uuid().null())
                    .to_owned(),
            )
            .await?;

        // Step D: Rename config_override → config.
        manager
            .alter_table(
                Table::alter()
                    .table(Hsip::Table)
                    .rename_column(Hsip::ConfigOverride, Hsip::Config)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    // ── Step E: Migrate package manager configs ──────────────────────────

    async fn migrate_package_manager_configs(
        &self,
        manager: &SchemaManager<'_>,
    ) -> Result<(), DbErr> {
        // For each package manager type: migrate the auto-created plugin_config
        // rows to plugin_type_settings and NULL out the FK on assignments.
        //
        // We use raw SQL because this involves a correlated UPDATE + INSERT
        // with conditional logic that sea_query cannot express.
        for pm_type in PACKAGE_MANAGER_TYPES {
            // Insert into plugin_type_settings if not already present.
            // Uses the first active plugin_config per (tenant_id, plugin_type).
            let insert_sql = format!(
                "INSERT INTO plugin_type_settings (id, tenant_id, plugin_type, config, created_at, updated_at) \
                 SELECT \
                   pc.id, pc.tenant_id, pc.plugin_type, pc.config, pc.created_at, pc.updated_at \
                 FROM plugin_configs pc \
                 WHERE pc.plugin_type = '{pm_type}' \
                   AND pc.deactivated_at IS NULL \
                   AND NOT EXISTS ( \
                     SELECT 1 FROM plugin_type_settings pts \
                     WHERE pts.tenant_id = pc.tenant_id \
                       AND pts.plugin_type = pc.plugin_type \
                   ) \
                 GROUP BY pc.tenant_id"
            );
            manager
                .get_connection()
                .execute_unprepared(&insert_sql)
                .await?;

            // NULL out plugin_config_id on assignments for this type.
            let update_sql = format!(
                "UPDATE host_software_item_plugins \
                 SET plugin_config_id = NULL \
                 WHERE plugin_type = '{pm_type}'"
            );
            manager
                .get_connection()
                .execute_unprepared(&update_sql)
                .await?;

            // Soft-delete the auto-created plugin_config rows.
            let deactivate_sql = format!(
                "UPDATE plugin_configs \
                 SET deactivated_at = CURRENT_TIMESTAMP \
                 WHERE plugin_type = '{pm_type}' \
                   AND deactivated_at IS NULL"
            );
            manager
                .get_connection()
                .execute_unprepared(&deactivate_sql)
                .await?;
        }

        Ok(())
    }

    // ── Reverse operations ───────────────────────────────────────────────

    async fn reverse_recreate_hsip_sqlite(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        helpers::set_foreign_keys(manager, false).await?;

        let state = helpers::check_crash_recovery(
            manager,
            "host_software_item_plugins",
            "host_software_item_plugins_old",
        )
        .await?;

        if state == CrashRecoveryState::Normal {
            // Create old-schema table.
            manager
                .create_table(
                    Table::create()
                        .table(HsipOld::Table)
                        .col(ColumnDef::new(HsipOld::Id).uuid().not_null().primary_key())
                        .col(ColumnDef::new(HsipOld::HostId).uuid().not_null())
                        .col(ColumnDef::new(HsipOld::SoftwareItemId).uuid().not_null())
                        .col(
                            ColumnDef::new(HsipOld::HostSoftwareItemId)
                                .uuid()
                                .not_null(),
                        )
                        .col(ColumnDef::new(HsipOld::PluginConfigId).uuid().not_null())
                        .col(ColumnDef::new(HsipOld::Role).string().not_null())
                        .col(
                            ColumnDef::new(HsipOld::Ordinal)
                                .integer()
                                .not_null()
                                .default(0),
                        )
                        .col(
                            ColumnDef::new(HsipOld::PackageIdentifier)
                                .string()
                                .not_null(),
                        )
                        .col(ColumnDef::new(HsipOld::ConfigOverride).json())
                        .col(
                            ColumnDef::new(HsipOld::ExecutionSite)
                                .string()
                                .not_null()
                                .default("auto"),
                        )
                        .col(ColumnDef::new(HsipOld::CreatedAt).timestamp().not_null())
                        .col(ColumnDef::new(HsipOld::UpdatedAt).timestamp().not_null())
                        .to_owned(),
                )
                .await?;

            // Copy data back (only rows with non-NULL plugin_config_id).
            manager
                .get_connection()
                .execute_unprepared(
                    "INSERT INTO host_software_item_plugins_old \
                     (id, host_id, software_item_id, host_software_item_id, \
                      plugin_config_id, role, ordinal, package_identifier, \
                      config_override, execution_site, created_at, updated_at) \
                     SELECT id, host_id, software_item_id, host_software_item_id, \
                       plugin_config_id, role, ordinal, package_identifier, \
                       config, execution_site, created_at, updated_at \
                     FROM host_software_item_plugins \
                     WHERE plugin_config_id IS NOT NULL",
                )
                .await?;

            helpers::drop_original(manager, "host_software_item_plugins").await?;
        }

        helpers::rename_temp(
            manager,
            "host_software_item_plugins_old",
            "host_software_item_plugins",
        )
        .await?;

        // Recreate original indexes.
        self.create_hsip_indexes(manager).await?;

        helpers::set_foreign_keys(manager, true).await?;
        Ok(())
    }

    async fn reverse_alter_hsip_postgres(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        // Reverse D: Rename config → config_override.
        manager
            .alter_table(
                Table::alter()
                    .table(Hsip::Table)
                    .rename_column(Hsip::Config, Hsip::ConfigOverride)
                    .to_owned(),
            )
            .await?;

        // Reverse C: Make plugin_config_id NOT NULL again (delete rows with NULL first).
        manager
            .get_connection()
            .execute_unprepared(
                "DELETE FROM host_software_item_plugins WHERE plugin_config_id IS NULL",
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Hsip::Table)
                    .modify_column(ColumnDef::new(Hsip::PluginConfigId).uuid().not_null())
                    .to_owned(),
            )
            .await?;

        // Reverse B: Drop plugin_type column.
        manager
            .alter_table(
                Table::alter()
                    .table(Hsip::Table)
                    .drop_column(Hsip::PluginType)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

// ── Iden enums ───────────────────────────────────────────────────────────────

#[derive(DeriveIden)]
enum PluginTypeSettings {
    Table,
    Id,
    TenantId,
    PluginType,
    Config,
    CreatedAt,
    UpdatedAt,
}

/// Current host_software_item_plugins columns (used for ALTER TABLE operations).
#[derive(DeriveIden)]
enum Hsip {
    #[sea_orm(iden = "host_software_item_plugins")]
    Table,
    HostId,
    SoftwareItemId,
    PluginConfigId,
    PluginType,
    Role,
    Ordinal,
    Config,
    ConfigOverride,
    ExecutionSite,
}

/// Temp table for SQLite forward recreation.
#[derive(DeriveIden)]
enum HsipNew {
    #[sea_orm(iden = "host_software_item_plugins_new")]
    Table,
    Id,
    HostId,
    SoftwareItemId,
    HostSoftwareItemId,
    PluginConfigId,
    PluginType,
    Role,
    Ordinal,
    PackageIdentifier,
    Config,
    ExecutionSite,
    CreatedAt,
    UpdatedAt,
}

/// Temp table for SQLite reverse recreation.
#[derive(DeriveIden)]
enum HsipOld {
    #[sea_orm(iden = "host_software_item_plugins_old")]
    Table,
    Id,
    HostId,
    SoftwareItemId,
    HostSoftwareItemId,
    PluginConfigId,
    Role,
    Ordinal,
    PackageIdentifier,
    ConfigOverride,
    ExecutionSite,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Tenants {
    #[sea_orm(iden = "tenants")]
    Table,
    Id,
}

#[derive(DeriveIden)]
enum PluginConfigs {
    #[sea_orm(iden = "plugin_configs")]
    Table,
    Id,
}
