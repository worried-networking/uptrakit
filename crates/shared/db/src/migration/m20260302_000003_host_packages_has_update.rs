use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

/// Add `has_update` stored generated column and covering indexes to `host_packages`.
///
/// SQLite does not support `ALTER TABLE … ADD COLUMN … GENERATED ALWAYS AS`.
/// This migration uses the standard SQLite 12-step table-recreation approach:
/// disable FK enforcement, create the replacement table with the generated
/// column already present, copy all data, drop the original, rename, rebuild
/// indexes, and re-enable FK enforcement.
///
/// On PostgreSQL (≥ 12) and MySQL (≥ 5.7) the same table-recreation strategy
/// is used for uniformity. The `PRAGMA foreign_keys` guard is gated behind a
/// database-backend check and is never sent to those engines.
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
pub struct Migration;

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

/// Suspend or resume FK enforcement on SQLite.
///
/// On PostgreSQL and MySQL the `PRAGMA` statement is not recognised; this
/// function is a no-op for those backends.
async fn set_foreign_keys(manager: &SchemaManager<'_>, enabled: bool) -> Result<(), DbErr> {
    if manager.get_database_backend() == DbBackend::Sqlite {
        let pragma = if enabled {
            "PRAGMA foreign_keys = ON"
        } else {
            "PRAGMA foreign_keys = OFF"
        };
        manager.get_connection().execute_unprepared(pragma).await?;
    }
    Ok(())
}

/// Build and execute `INSERT INTO <target> (<cols>) SELECT <cols> FROM <source>`.
async fn copy_table(
    manager: &SchemaManager<'_>,
    source: impl IntoTableRef,
    target: impl IntoTableRef,
) -> Result<(), DbErr> {
    let select = Query::select()
        .columns(DATA_COLS)
        .from(source)
        .to_owned();

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
        .col(
            ColumnDef::new(Col::Id)
                .uuid()
                .not_null()
                .primary_key(),
        )
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
                    .and(
                        Expr::col(Col::InstalledVersion)
                            .ne(Expr::col(Col::LatestVersion)),
                    ),
                true, // STORED
            ),
        );
    }

    t.to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Suspend FK enforcement on SQLite while the table swap is in progress.
        set_foreign_keys(manager, false).await?;

        // Recover from a previous partial run.  Three states are possible:
        //
        //   A. host_packages exists, host_packages_new does not
        //      → normal path; fall through to the create/copy/drop block below.
        //
        //   B. Both host_packages and host_packages_new exist
        //      → a previous run created the temp table but did not complete.
        //        The original data is still intact.  Discard the partial temp
        //        table and restart from scratch (treated as State A).
        //
        //   C. host_packages_new exists but host_packages does not
        //      → a previous run copied the data and dropped the original, but
        //        the rename never happened.  Skip the create/copy/drop steps
        //        and proceed directly to the rename.
        let new_exists = manager.has_table("host_packages_new").await?;
        let orig_exists = manager.has_table("host_packages").await?;

        if new_exists && orig_exists {
            // State B: discard the incomplete temp table; fall through to State A.
            manager
                .drop_table(Table::drop().table(HostPackagesNew::Table).to_owned())
                .await?;
        }

        if orig_exists {
            // State A (or recovered from B): create replacement, copy, drop original.

            // Step 1: create the replacement table (identical schema + has_update).
            manager
                .create_table(build_host_packages_table(
                    HostPackagesNew::Table,
                    true,
                ))
                .await?;

            // Step 2: copy all non-generated rows; has_update is computed by the engine.
            copy_table(manager, HostPackages::Table, HostPackagesNew::Table).await?;

            // Step 3: drop the original table (its indexes are dropped implicitly).
            manager
                .drop_table(Table::drop().table(HostPackages::Table).to_owned())
                .await?;
        }
        // else (State C): host_packages_new already holds the full dataset.

        // Step 4: rename the replacement table to the canonical name.
        manager
            .rename_table(
                Table::rename()
                    .table(HostPackagesNew::Table, HostPackages::Table)
                    .to_owned(),
            )
            .await?;

        // Step 5: recreate the three original indexes.
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

        // Step 6: create the three new covering indexes on has_update.
        //
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

        // Step 7: re-enable FK enforcement on SQLite.
        set_foreign_keys(manager, true).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Recreate the table without the has_update column using the same
        // 12-step approach as `up`.
        set_foreign_keys(manager, false).await?;

        // Same three-state recovery logic as `up`, mirrored for `down`.
        let bak_exists = manager.has_table("host_packages_bak").await?;
        let orig_exists = manager.has_table("host_packages").await?;

        if bak_exists && orig_exists {
            // Incomplete previous down run: discard the partial backup table.
            manager
                .drop_table(Table::drop().table(HostPackagesBak::Table).to_owned())
                .await?;
        }

        if orig_exists {
            // Create the pre-migration schema (no has_update column) under a
            // temporary name.
            manager
                .create_table(build_host_packages_table(
                    HostPackagesBak::Table,
                    false,
                ))
                .await?;

            // Copy all data rows (DATA_COLS, which already excludes has_update).
            copy_table(manager, HostPackages::Table, HostPackagesBak::Table).await?;

            // Drop the current table (drops the three new indexes implicitly).
            manager
                .drop_table(Table::drop().table(HostPackages::Table).to_owned())
                .await?;
        }
        // else: host_packages_bak already holds the data; fall through to rename.

        // Rename the backup to the canonical name.
        manager
            .rename_table(
                Table::rename()
                    .table(HostPackagesBak::Table, HostPackages::Table)
                    .to_owned(),
            )
            .await?;

        // Restore the original indexes.
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

        set_foreign_keys(manager, true).await?;

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
