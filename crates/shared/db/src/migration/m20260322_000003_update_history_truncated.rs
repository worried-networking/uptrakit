use sea_orm_migration::prelude::*;

/// Add `output_truncated` (BOOL NOT NULL DEFAULT FALSE) to `update_history`.
///
/// Records whether any update output was dropped because the output-size cap
/// was exceeded. Set atomically on first truncation (conditional UPDATE that
/// only fires when `output_truncated = false`). The API exposes this flag so
/// the history detail view can show an amber warning banner even for completed
/// updates where the live truncation notice was already displayed in the
/// terminal stream.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add output_truncated column (all backends).
        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .add_column(
                        ColumnDef::new(UpdateHistory::OutputTruncated)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .drop_column(UpdateHistory::OutputTruncated)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    OutputTruncated,
}
