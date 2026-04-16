use sea_orm_migration::prelude::*;

/// Add shared protection/recovery metadata fields to `update_history`.
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
                        ColumnDef::new(UpdateHistory::PreUpdateProtectionStatus)
                            .text()
                            .null(),
                    )
                    .add_column(
                        ColumnDef::new(UpdateHistory::PreUpdateProtectionSummary)
                            .text()
                            .null(),
                    )
                    .add_column(ColumnDef::new(UpdateHistory::RecoveryHint).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .drop_column(UpdateHistory::RecoveryHint)
                    .drop_column(UpdateHistory::PreUpdateProtectionSummary)
                    .drop_column(UpdateHistory::PreUpdateProtectionStatus)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    PreUpdateProtectionStatus,
    PreUpdateProtectionSummary,
    RecoveryHint,
}
