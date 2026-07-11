use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("host_software_items"))
                    .add_column_if_not_exists(
                        ColumnDef::new(Alias::new("last_discovered_at"))
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("host_software_items"))
                    .add_column_if_not_exists(
                        ColumnDef::new(Alias::new("discovery_source")).text().null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("host_software_items"))
                    .add_column_if_not_exists(
                        ColumnDef::new(Alias::new("missing_since"))
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for col in ["missing_since", "discovery_source", "last_discovered_at"] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("host_software_items"))
                        .drop_column(Alias::new(col))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
