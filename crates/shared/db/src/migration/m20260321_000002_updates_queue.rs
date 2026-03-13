use sea_orm_migration::prelude::*;

/// Add a partial index on `update_history(host_id, id)` scoped to queued rows
/// (`status = 'queued'`) to support efficient FIFO dispatch queries.
///
/// The index speeds up the per-host FIFO query in `dispatch_next_queued_for_host`:
/// `SELECT ... WHERE host_id = ? AND status = 'queued' ORDER BY id LIMIT 1`.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_host_queued")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::HostId)
                    .col(UpdateHistory::Id)
                    .and_where(Expr::col(UpdateHistory::Status).eq("queued"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_update_history_host_queued")
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
    Id,
    Status,
}
