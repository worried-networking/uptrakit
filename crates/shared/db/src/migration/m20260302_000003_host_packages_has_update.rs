use sea_orm_migration::prelude::*;

/// Add `has_update` stored generated column and covering indexes to `host_packages`.
///
/// The column is computed by the database engine as:
///
/// ```sql
/// installed_version IS NOT NULL
///     AND latest_version IS NOT NULL
///     AND installed_version <> latest_version
/// ```
///
/// Identical `GENERATED ALWAYS AS ... STORED` syntax is supported on SQLite (≥ 3.31.0),
/// PostgreSQL (≥ 12), and MySQL (≥ 5.7). The database maintains the value automatically;
/// it must never be set explicitly in application code.
///
/// Three indexes are added to make the common filter combinations efficient:
///
/// | Index | Covers |
/// |-------|--------|
/// | `idx_hp_has_update` | `has_update`-only filter |
/// | `idx_hp_has_update_category` | combined `has_update` + `update_category` filter |
/// | `idx_hp_host_category` | `update_category`-only filter |
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Stored generated column. Syntax is identical across SQLite (≥ 3.31.0),
        // PostgreSQL (≥ 12), and MySQL (≥ 5.7); BOOLEAN is an alias for TINYINT(1)
        // in MySQL and an integer affinity in SQLite.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE host_packages \
                 ADD COLUMN has_update BOOLEAN GENERATED ALWAYS AS \
                 (installed_version IS NOT NULL \
                  AND latest_version IS NOT NULL \
                  AND installed_version <> latest_version) STORED",
            )
            .await?;

        // (host_id, has_update) — has_update-only filter, and the update-summary
        // aggregation queries (WHERE host_id = ? AND enabled = true AND has_update = true).
        manager
            .create_index(
                Index::create()
                    .name("idx_hp_has_update")
                    .table(HostPackages::Table)
                    .col(HostPackages::HostId)
                    .col(Alias::new("has_update"))
                    .to_owned(),
            )
            .await?;

        // (host_id, has_update, update_category) — combined has_update + category filter.
        manager
            .create_index(
                Index::create()
                    .name("idx_hp_has_update_category")
                    .table(HostPackages::Table)
                    .col(HostPackages::HostId)
                    .col(Alias::new("has_update"))
                    .col(HostPackages::UpdateCategory)
                    .to_owned(),
            )
            .await?;

        // (host_id, update_category) — category-only filter without has_update.
        manager
            .create_index(
                Index::create()
                    .name("idx_hp_host_category")
                    .table(HostPackages::Table)
                    .col(HostPackages::HostId)
                    .col(HostPackages::UpdateCategory)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_hp_host_category")
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
                    .name("idx_hp_has_update")
                    .table(HostPackages::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(HostPackages::Table)
                    .drop_column(Alias::new("has_update"))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum HostPackages {
    Table,
    HostId,
    UpdateCategory,
}
