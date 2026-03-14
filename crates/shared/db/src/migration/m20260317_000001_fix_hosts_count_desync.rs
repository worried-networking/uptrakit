use sea_orm_migration::prelude::*;

/// Add a composite index on `host_software_items(software_item_id, host_id)`.
///
/// All queries that count or read version data for a software item now join
/// `host_software_items` with `hosts` and filter `hosts.deactivated_at IS NULL`
/// so that deactivated hosts are excluded from host counts and version
/// calculations.  The existing single-column index on `(software_item_id)`
/// requires a separate heap fetch to retrieve `host_id` for the join.  The new
/// composite index `(software_item_id, host_id)` is a covering index for these
/// queries: the planner can satisfy both the filter and the join key from a
/// single index scan without touching the table heap, improving performance for
/// the bulk-count and bulk-version-load queries on the Software list page.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_host_software_items_software_item_host")
                    .table(HostSoftwareItems::Table)
                    .col(HostSoftwareItems::SoftwareItemId)
                    .col(HostSoftwareItems::HostId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_host_software_items_software_item_host")
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
}
