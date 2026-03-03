use sea_orm_migration::prelude::*;

/// Add a composite index on `update_history(host_id, software_item_id, status)`.
///
/// `validate_update_preconditions` queries this table on every precondition
/// check with the predicate:
///
/// ```sql
/// WHERE host_id = $1 AND software_item_id = $2 AND status IN ('Pending', 'InProgress')
/// ```
///
/// Without this index the query is a full table scan. As `update_history`
/// grows (one row per triggered update) the scan cost increases linearly
/// with every batch precondition check.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_host_item_status")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::HostId)
                    .col(UpdateHistory::SoftwareItemId)
                    .col(UpdateHistory::Status)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_update_history_host_item_status")
                    .table(UpdateHistory::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    HostId,
    SoftwareItemId,
    Status,
}
