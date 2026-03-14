use sea_orm_migration::prelude::*;

/// Add a composite covering index on `host_software_items(software_item_id,
/// host_id, installed_version, latest_version)` to support the `updatable`
/// EXISTS filter efficiently.
///
/// The `updatable` query parameter on `GET /api/v1/software-items` uses a
/// correlated EXISTS subquery that joins `host_software_items` on
/// `software_item_id` and `host_id`, then filters on both version columns.
/// Without this index the DB performs a seq-scan over all host assignments for
/// every software item in the outer query.
///
/// The column order is chosen so the index is also a drop-in superset of the
/// existing `idx_host_software_items_software_item_id` single-column index
/// (which remains in place — it may still be chosen for non-updatable queries
/// that only need the FK lookup).
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_hsi_software_item_id_host_id_versions")
                    .table(HostSoftwareItems::Table)
                    .col(HostSoftwareItems::SoftwareItemId)
                    .col(HostSoftwareItems::HostId)
                    .col(HostSoftwareItems::InstalledVersion)
                    .col(HostSoftwareItems::LatestVersion)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_hsi_software_item_id_host_id_versions")
                    .table(HostSoftwareItems::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum HostSoftwareItems {
    Table,
    SoftwareItemId,
    HostId,
    InstalledVersion,
    LatestVersion,
}
