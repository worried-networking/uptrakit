use sea_orm_migration::prelude::*;

/// Add `update_category` column to `host_software_items` and `update_history`.
///
/// The column stores the classification of an available update (security,
/// bugfix, feature, unknown). Defaults to `"unknown"` for existing rows and
/// newly created rows where the plugin cannot classify.
///
/// Also adds a composite index on `(host_id, update_category)` in
/// `host_software_items` for filtered batch-update queries.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // host_software_items.update_category
        manager
            .alter_table(
                Table::alter()
                    .table(HostSoftwareItems::Table)
                    .add_column(
                        ColumnDef::new(HostSoftwareItems::UpdateCategory)
                            .text()
                            .not_null()
                            .default("unknown"),
                    )
                    .to_owned(),
            )
            .await?;

        // update_history.update_category
        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .add_column(
                        ColumnDef::new(UpdateHistory::UpdateCategory)
                            .text()
                            .not_null()
                            .default("unknown"),
                    )
                    .to_owned(),
            )
            .await?;

        // Composite index for filtered batch queries:
        // "find all outdated items of category X on host Y"
        manager
            .create_index(
                Index::create()
                    .name("idx_hsi_host_category")
                    .table(HostSoftwareItems::Table)
                    .col(HostSoftwareItems::HostId)
                    .col(HostSoftwareItems::UpdateCategory)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_hsi_host_category")
                    .table(HostSoftwareItems::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .drop_column(UpdateHistory::UpdateCategory)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(HostSoftwareItems::Table)
                    .drop_column(HostSoftwareItems::UpdateCategory)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum HostSoftwareItems {
    Table,
    HostId,
    UpdateCategory,
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    UpdateCategory,
}
