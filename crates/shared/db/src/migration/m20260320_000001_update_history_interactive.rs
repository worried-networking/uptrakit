use sea_orm_migration::prelude::*;

/// Add `interactive` (BOOL NOT NULL DEFAULT FALSE) to `update_history`.
///
/// Records whether an update was dispatched in interactive mode (PTY allocated,
/// stdin kept open). The flag is set at dispatch time and is immutable — it
/// describes how the update was started, not any runtime heuristic.
///
/// Used by the history-list UI to show an "Input Required" badge on every
/// in-progress interactive update, even when the user is not actively watching
/// its live output stream.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .add_column(
                        ColumnDef::new(UpdateHistory::Interactive)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .drop_column(UpdateHistory::Interactive)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    Interactive,
}
