use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(UpdateHistory::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UpdateHistory::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UpdateHistory::HostId).uuid().not_null())
                    .col(
                        ColumnDef::new(UpdateHistory::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string_null(UpdateHistory::FromVersion))
                    .col(string(UpdateHistory::ToVersion))
                    .col(string(UpdateHistory::Status))
                    .col(ColumnDef::new(UpdateHistory::Output).text().not_null())
                    .col(string(UpdateHistory::InitiatedBy))
                    .col(timestamp(UpdateHistory::StartedAt))
                    .col(timestamp_null(UpdateHistory::CompletedAt))
                    .col(timestamp(UpdateHistory::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_history_host")
                            .from(UpdateHistory::Table, UpdateHistory::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_history_software_item")
                            .from(UpdateHistory::Table, UpdateHistory::SoftwareItemId)
                            .to(SoftwareItems::Table, SoftwareItems::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Index on host_id for FK lookups and host filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_host_id")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::HostId)
                    .to_owned(),
            )
            .await?;

        // Index on software_item_id for FK lookups and software item filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_software_item_id")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::SoftwareItemId)
                    .to_owned(),
            )
            .await?;

        // Index on status for status filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_status")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::Status)
                    .to_owned(),
            )
            .await?;

        // Composite index for common query pattern (host + software item)
        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_host_software_item")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::HostId)
                    .col(UpdateHistory::SoftwareItemId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UpdateHistory::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    Id,
    HostId,
    SoftwareItemId,
    FromVersion,
    ToVersion,
    Status,
    Output,
    InitiatedBy,
    StartedAt,
    CompletedAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum SoftwareItems {
    Table,
    Id,
}
