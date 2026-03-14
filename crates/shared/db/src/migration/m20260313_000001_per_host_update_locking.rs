use sea_orm_migration::prelude::*;

/// Add a partial unique index on `update_history(host_id)` scoped to active
/// rows (`status IN ('pending', 'in_progress')`).
///
/// This enforces the invariant at the database level: **at most one active
/// (Pending or InProgress) `update_history` row may exist per host at any
/// time**.  Rows with other statuses (`queued`, `completed`, `failed`) are
/// excluded from the constraint.
///
/// The `queued` status is intentionally excluded so that batch items waiting
/// for a preceding item on the same host to complete can coexist in the table
/// without violating the constraint.
///
/// The partial unique index provides race safety for multi-controller
/// deployments: a concurrent INSERT from a second controller process will
/// produce a unique-constraint violation (which surfaces as `UpdateAlreadyActive`)
/// rather than silently inserting a duplicate active row.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("uix_update_history_host_active")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::HostId)
                    .unique()
                    .and_where(Expr::col(UpdateHistory::Status).is_in(["pending", "in_progress"]))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("uix_update_history_host_active")
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
    Status,
}
