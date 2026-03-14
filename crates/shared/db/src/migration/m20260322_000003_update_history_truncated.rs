use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;

/// Add `output_truncated` (BOOL NOT NULL DEFAULT FALSE) to `update_history`,
/// and widen `output` to LONGTEXT on MySQL deployments.
///
/// # `output_truncated`
///
/// Records whether any update output was dropped because the output-size cap
/// was exceeded. Set atomically on first truncation (conditional UPDATE that
/// only fires when `output_truncated = false`). The API exposes this flag so
/// the history detail view can show an amber warning banner even for completed
/// updates where the live truncation notice was already displayed in the
/// terminal stream.
///
/// # MySQL `output` column widening
///
/// The output-size cap is raised from 1 MB to 50 MB. MySQL `TEXT` columns are
/// limited to 64 KB, which would immediately truncate every update — so the
/// column must be widened to `LONGTEXT` (4 GB limit) on MySQL deployments.
/// PostgreSQL and SQLite `TEXT` columns already support arbitrary lengths; no
/// change is needed for those backends.
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

        // Widen output column to LONGTEXT on MySQL only.
        //
        // MySQL TEXT is capped at 64 KB; LONGTEXT (4 GB) is required for the
        // 50 MB output cap. PostgreSQL and SQLite TEXT already support
        // arbitrary lengths — no change needed there.
        //
        // `MODIFY COLUMN … NOT NULL` (MySQL syntax) cannot be expressed with
        // the cross-backend `Table::alter` builder because LONGTEXT has no
        // first-class sea-query ColumnType variant. `execute_unprepared` with
        // MySQL-native DDL is the approved exception for backend-specific
        // statements (see `m20260309_000003_host_tags.rs` for precedent).
        if manager.get_database_backend() == DbBackend::MySql {
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE `update_history` MODIFY COLUMN `output` LONGTEXT NOT NULL",
                )
                .await?;
        }

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
