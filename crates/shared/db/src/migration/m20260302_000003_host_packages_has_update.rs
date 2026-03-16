use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::helpers::{self, CrashRecoveryState, timestamp, timestamp_null};

/// Add `has_update` stored generated column and covering indexes to `host_packages`.
///
/// SQLite does not support `ALTER TABLE … ADD COLUMN … GENERATED ALWAYS AS`.
/// On SQLite this migration uses the standard 12-step table-recreation approach:
/// disable FK enforcement, create the replacement table with the generated
/// column already present, copy all data, drop the original, rename, rebuild
/// indexes, and re-enable FK enforcement.
///
/// On PostgreSQL (≥ 12) and MySQL (≥ 5.7) the migration uses
/// `ALTER TABLE ADD COLUMN` directly, which avoids the FK-cascade problems
/// that table recreation causes when other tables reference `host_packages`.
///
/// The column is computed by the database engine as:
///
/// ```text
/// installed_version IS NOT NULL
///     AND latest_version IS NOT NULL
///     AND installed_version <> latest_version
/// ```
///
/// Three indexes are added to make the common filter combinations efficient:
///
/// | Index | Covers |
/// |-------|--------|
/// | `idx_hp_has_update` | `(host_id, has_update)` |
/// | `idx_hp_has_update_category` | `(host_id, has_update, update_category)` |
/// | `idx_hp_host_category` | `(host_id, update_category)` |
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// Non-generated data columns of `host_packages`.
///
/// Shared between the INSERT column list and the SELECT column list during
/// table recreation. `has_update` is intentionally absent — the database engine
/// derives its value automatically from the `GENERATED ALWAYS AS` expression.
const DATA_COLS: [Col; 18] = [
    Col::Id,
    Col::TenantId,
    Col::HostId,
    Col::PluginConfigId,
    Col::PackageIdentifier,
    Col::Name,
    Col::InstalledVersion,
    Col::InstalledVersionDetectedAt,
    Col::LatestVersion,
    Col::LatestVersionFetchedAt,
    Col::LatestReleaseMetadata,
    Col::UpdateCategory,
    Col::Enabled,
    Col::LastCheckedAt,
    Col::LastUpdatedAt,
    Col::CreatedAt,
    Col::UpdatedAt,
    Col::DeactivatedAt,
];

/// Build and execute `INSERT INTO <target> (<cols>) SELECT <cols> FROM <source>`.
async fn copy_table(
    manager: &SchemaManager<'_>,
    source: impl IntoTableRef,
    target: impl IntoTableRef,
) -> Result<(), DbErr> {
    let select = Query::select().columns(DATA_COLS).from(source).to_owned();

    let mut insert = Query::insert()
        .into_table(target)
        .columns(DATA_COLS)
        .to_owned();

    insert
        .select_from(select)
        .map_err(|e| DbErr::Custom(e.to_string()))?;

    manager.execute(insert).await
}

/// Build the full `host_packages` schema, targeting `table_name`.
///
/// When `with_generated` is `true` the `has_update` STORED generated column is
/// included; when `false` it is omitted (used by the `down` path to restore the
/// pre-migration schema).
fn build_host_packages_table(
    table_name: impl IntoTableRef + Clone,
    with_generated: bool,
) -> TableCreateStatement {
    let mut t = Table::create();
    t.table(table_name.clone())
        .col(ColumnDef::new(Col::Id).uuid().not_null().primary_key())
        .col(ColumnDef::new(Col::TenantId).uuid().not_null())
        .col(ColumnDef::new(Col::HostId).uuid().not_null())
        .col(ColumnDef::new(Col::PluginConfigId).uuid().not_null())
        .col(string(Col::PackageIdentifier))
        .col(string(Col::Name))
        .col(string_null(Col::InstalledVersion))
        .col(timestamp_null(Col::InstalledVersionDetectedAt))
        .col(string_null(Col::LatestVersion))
        .col(timestamp_null(Col::LatestVersionFetchedAt))
        .col(
            ColumnDef::new(Col::LatestReleaseMetadata)
                .json_binary()
                .null(),
        )
        .col(
            ColumnDef::new(Col::UpdateCategory)
                .text()
                .not_null()
                .default("unknown"),
        )
        .col(
            ColumnDef::new(Col::Enabled)
                .boolean()
                .not_null()
                .default(true),
        )
        .col(timestamp_null(Col::LastCheckedAt))
        .col(timestamp_null(Col::LastUpdatedAt))
        .col(timestamp(Col::CreatedAt))
        .col(timestamp(Col::UpdatedAt))
        .col(timestamp_null(Col::DeactivatedAt))
        .foreign_key(
            ForeignKey::create()
                .name("fk_host_packages_tenant_id")
                .from(table_name.clone(), Col::TenantId)
                .to(Tenants::Table, Tenants::Id)
                .on_delete(ForeignKeyAction::Restrict),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_host_packages_host_id")
                .from(table_name.clone(), Col::HostId)
                .to(Hosts::Table, Hosts::Id)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_host_packages_plugin_config_id")
                .from(table_name, Col::PluginConfigId)
                .to(PluginConfigs::Table, PluginConfigs::Id)
                .on_delete(ForeignKeyAction::Restrict),
        );

    if with_generated {
        t.col(
            ColumnDef::new(Col::HasUpdate).boolean().generated(
                Expr::col(Col::InstalledVersion)
                    .is_not_null()
                    .and(Expr::col(Col::LatestVersion).is_not_null())
                    .and(Expr::col(Col::InstalledVersion).ne(Expr::col(Col::LatestVersion))),
                true, // STORED
            ),
        );
    }

    t.to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if helpers::is_sqlite(manager) {
            self.up_sqlite(manager).await
        } else {
            self.up_alter(manager).await
        }
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if helpers::is_sqlite(manager) {
            self.down_sqlite(manager).await
        } else {
            self.down_alter(manager).await
        }
    }
}

impl Migration {
    /// SQLite path: table recreation (create new -> copy -> drop old -> rename).
    async fn up_sqlite(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        helpers::set_foreign_keys(manager, false).await?;

        let state =
            helpers::check_crash_recovery(manager, "host_packages", "host_packages_new").await?;

        if state == CrashRecoveryState::Normal {
            // Create the replacement table (identical schema + has_update).
            manager
                .create_table(build_host_packages_table(HostPackagesNew::Table, true))
                .await?;

            // Copy all non-generated rows; has_update is computed by the engine.
            copy_table(manager, HostPackages::Table, HostPackagesNew::Table).await?;

            // Drop the original table (its indexes are dropped implicitly).
            helpers::drop_original(manager, "host_packages").await?;
        }

        helpers::rename_temp(manager, "host_packages_new", "host_packages").await?;

        // Recreate the three original indexes (dropped implicitly with the old table).
        self.create_original_indexes(manager).await?;

        // Create the three new covering indexes on has_update.
        self.create_has_update_indexes(manager).await?;

        helpers::set_foreign_keys(manager, true).await?;

        Ok(())
    }

    /// PostgreSQL/MySQL path: ALTER TABLE ADD COLUMN + indexes.
    async fn up_alter(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(HostPackages::Table)
                    .add_column(
                        ColumnDef::new(Col::HasUpdate).boolean().generated(
                            Expr::col(Col::InstalledVersion)
                                .is_not_null()
                                .and(Expr::col(Col::LatestVersion).is_not_null())
                                .and(
                                    Expr::col(Col::InstalledVersion)
                                        .ne(Expr::col(Col::LatestVersion)),
                                ),
                            true, // STORED
                        ),
                    )
                    .to_owned(),
            )
            .await?;

        // Create the three new covering indexes on has_update.
        self.create_has_update_indexes(manager).await?;

        Ok(())
    }

    /// SQLite down path: table recreation (create without has_update -> copy -> drop -> rename).
    async fn down_sqlite(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        helpers::set_foreign_keys(manager, false).await?;

        let state =
            helpers::check_crash_recovery(manager, "host_packages", "host_packages_bak").await?;

        if state == CrashRecoveryState::Normal {
            // Create the pre-migration schema (no has_update column).
            manager
                .create_table(build_host_packages_table(HostPackagesBak::Table, false))
                .await?;

            // Copy all data rows (DATA_COLS, which already excludes has_update).
            copy_table(manager, HostPackages::Table, HostPackagesBak::Table).await?;

            // Drop the current table (drops the new indexes implicitly).
            helpers::drop_original(manager, "host_packages").await?;
        }

        helpers::rename_temp(manager, "host_packages_bak", "host_packages").await?;

        // Restore the original indexes.
        self.create_original_indexes(manager).await?;

        helpers::set_foreign_keys(manager, true).await?;

        Ok(())
    }

    /// PostgreSQL/MySQL down path: DROP COLUMN + indexes.
    async fn down_alter(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        // Drop the has_update indexes first.
        manager
            .drop_index(
                Index::drop()
                    .name("idx_hp_has_update")
                    .table(HostPackages::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_hp_has_update_category")
                    .table(HostPackages::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_hp_host_category")
                    .table(HostPackages::Table)
                    .to_owned(),
            )
            .await?;

        // Drop the generated column.
        manager
            .alter_table(
                Table::alter()
                    .table(HostPackages::Table)
                    .drop_column(Col::HasUpdate)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    /// The three original indexes that existed before this migration.
    ///
    /// On the SQLite path these are dropped implicitly when the old table is
    /// dropped and must be recreated after the rename.
    async fn create_original_indexes(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_hp_host_plugin_pkg")
                    .table(HostPackages::Table)
                    .col(Col::HostId)
                    .col(Col::PluginConfigId)
                    .col(Col::PackageIdentifier)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hp_tenant_host")
                    .table(HostPackages::Table)
                    .col(Col::TenantId)
                    .col(Col::HostId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hp_host_enabled")
                    .table(HostPackages::Table)
                    .col(Col::HostId)
                    .col(Col::Enabled)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    /// The three new covering indexes introduced by this migration.
    async fn create_has_update_indexes(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        // (host_id, has_update) — has_update-only filter; used by the
        // update-summary aggregation query (WHERE host_id = ? AND enabled = true
        // AND has_update = true).
        manager
            .create_index(
                Index::create()
                    .name("idx_hp_has_update")
                    .table(HostPackages::Table)
                    .col(Col::HostId)
                    .col(Col::HasUpdate)
                    .to_owned(),
            )
            .await?;

        // (host_id, has_update, update_category) — combined has_update + category filter.
        manager
            .create_index(
                Index::create()
                    .name("idx_hp_has_update_category")
                    .table(HostPackages::Table)
                    .col(Col::HostId)
                    .col(Col::HasUpdate)
                    .col(Col::UpdateCategory)
                    .to_owned(),
            )
            .await?;

        // (host_id, update_category) — category-only filter without has_update.
        manager
            .create_index(
                Index::create()
                    .name("idx_hp_host_category")
                    .table(HostPackages::Table)
                    .col(Col::HostId)
                    .col(Col::UpdateCategory)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Iden definitions
// ---------------------------------------------------------------------------

/// Canonical `host_packages` table name.
#[derive(DeriveIden)]
enum HostPackages {
    Table,
}

/// Temporary table used during `up` — renamed to `host_packages` at the end.
#[derive(Clone, DeriveIden)]
enum HostPackagesNew {
    Table,
}

/// Temporary table used during `down` — renamed to `host_packages` at the end.
#[derive(Clone, DeriveIden)]
enum HostPackagesBak {
    Table,
}

/// All columns of `host_packages`, shared across both the old and new schemas.
///
/// `#[derive(Copy, Clone)]` allows the same `DATA_COLS` constant array to be
/// passed to both `Query::insert().columns(…)` and `Query::select().columns(…)`
/// without cloning.
#[derive(Copy, Clone, DeriveIden)]
enum Col {
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
    /// STORED generated column: present only in the post-migration schema.
    HasUpdate,
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
