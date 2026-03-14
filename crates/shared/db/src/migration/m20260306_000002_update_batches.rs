use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

/// Create `update_batches` table and add `batch_id` to `update_history`.
///
/// Since there are no active deployments, the `update_history` and
/// `update_output_lines` tables are dropped and recreated to include the new
/// column with an inline FK constraint (avoids SQLite ALTER TABLE limitations).
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Create update_batches table
        manager
            .create_table(
                Table::create()
                    .table(UpdateBatches::Table)
                    .col(
                        ColumnDef::new(UpdateBatches::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UpdateBatches::TenantId).uuid().not_null())
                    .col(ColumnDef::new(UpdateBatches::BatchType).text().not_null())
                    .col(
                        ColumnDef::new(UpdateBatches::Status)
                            .text()
                            .not_null()
                            .default("in_progress"),
                    )
                    .col(
                        ColumnDef::new(UpdateBatches::TotalCount)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(UpdateBatches::ActorType).text().not_null())
                    .col(ColumnDef::new(UpdateBatches::ActorId).text().not_null())
                    .col(timestamp(UpdateBatches::CreatedAt))
                    .col(timestamp_null(UpdateBatches::CompletedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_batches_tenant_id")
                            .from(UpdateBatches::Table, UpdateBatches::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_ub_tenant_status")
                    .table(UpdateBatches::Table)
                    .col(UpdateBatches::TenantId)
                    .col(UpdateBatches::Status)
                    .to_owned(),
            )
            .await?;

        // 2. Drop update_output_lines (depends on update_history)
        manager
            .drop_table(Table::drop().table(UpdateOutputLines::Table).to_owned())
            .await?;

        // 3. Drop update_history
        manager
            .drop_table(Table::drop().table(UpdateHistory::Table).to_owned())
            .await?;

        // 4. Recreate update_history with batch_id and update_category
        manager
            .create_table(
                Table::create()
                    .table(UpdateHistory::Table)
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
                    .col(
                        ColumnDef::new(UpdateHistory::OutputBytes)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(UpdateHistory::ActorType)
                            .string()
                            .not_null()
                            .default("legacy"),
                    )
                    .col(
                        ColumnDef::new(UpdateHistory::ActorId)
                            .string()
                            .not_null()
                            .default(""),
                    )
                    .col(timestamp(UpdateHistory::StartedAt))
                    .col(timestamp_null(UpdateHistory::CompletedAt))
                    .col(timestamp(UpdateHistory::CreatedAt))
                    .col(
                        ColumnDef::new(UpdateHistory::UpdateCategory)
                            .text()
                            .not_null()
                            .default("unknown"),
                    )
                    .col(ColumnDef::new(UpdateHistory::BatchId).uuid().null())
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
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_history_batch_id")
                            .from(UpdateHistory::Table, UpdateHistory::BatchId)
                            .to(UpdateBatches::Table, UpdateBatches::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        // Recreate indexes on update_history.
        // Note: idx_update_history_created_at was added by m20260302_000001_add_missing_indexes
        // and must be recreated here since update_history is dropped and recreated.
        for (name, cols) in [
            ("idx_update_history_host_id", vec![UpdateHistory::HostId]),
            (
                "idx_update_history_software_item_id",
                vec![UpdateHistory::SoftwareItemId],
            ),
            ("idx_update_history_status", vec![UpdateHistory::Status]),
            (
                "idx_update_history_host_software_item",
                vec![UpdateHistory::HostId, UpdateHistory::SoftwareItemId],
            ),
            ("idx_uh_batch_id", vec![UpdateHistory::BatchId]),
            (
                "idx_update_history_created_at",
                vec![UpdateHistory::CreatedAt],
            ),
        ] {
            let mut idx = Index::create();
            idx.name(name).table(UpdateHistory::Table);
            for c in cols {
                idx.col(c);
            }
            manager.create_index(idx.to_owned()).await?;
        }

        // 5. Recreate update_output_lines
        manager
            .create_table(
                Table::create()
                    .table(UpdateOutputLines::Table)
                    .col(
                        ColumnDef::new(UpdateOutputLines::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(UpdateOutputLines::UpdateHistoryId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string(UpdateOutputLines::Stream))
                    .col(ColumnDef::new(UpdateOutputLines::Output).text().not_null())
                    .col(timestamp(UpdateOutputLines::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_output_lines_update_history")
                            .from(UpdateOutputLines::Table, UpdateOutputLines::UpdateHistoryId)
                            .to(UpdateHistory::Table, UpdateHistory::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_update_output_lines_update_history")
                    .table(UpdateOutputLines::Table)
                    .col(UpdateOutputLines::UpdateHistoryId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop update_output_lines and update_history (will be recreated
        // without batch_id by the initial migration's down path).
        manager
            .drop_table(Table::drop().table(UpdateOutputLines::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(UpdateHistory::Table).to_owned())
            .await?;

        // Drop update_batches
        manager
            .drop_index(
                Index::drop()
                    .name("idx_ub_tenant_status")
                    .table(UpdateBatches::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(Table::drop().table(UpdateBatches::Table).to_owned())
            .await?;

        // Recreate original update_history (without batch_id, without update_category)
        manager
            .create_table(
                Table::create()
                    .table(UpdateHistory::Table)
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
                    .col(
                        ColumnDef::new(UpdateHistory::OutputBytes)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(UpdateHistory::ActorType)
                            .string()
                            .not_null()
                            .default("legacy"),
                    )
                    .col(
                        ColumnDef::new(UpdateHistory::ActorId)
                            .string()
                            .not_null()
                            .default(""),
                    )
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

        // When rolling back this migration, m20260302_000001_add_missing_indexes is still
        // applied, so idx_update_history_created_at must be present on the recreated table.
        for (name, cols) in [
            ("idx_update_history_host_id", vec![UpdateHistory::HostId]),
            (
                "idx_update_history_software_item_id",
                vec![UpdateHistory::SoftwareItemId],
            ),
            ("idx_update_history_status", vec![UpdateHistory::Status]),
            (
                "idx_update_history_host_software_item",
                vec![UpdateHistory::HostId, UpdateHistory::SoftwareItemId],
            ),
            (
                "idx_update_history_created_at",
                vec![UpdateHistory::CreatedAt],
            ),
        ] {
            let mut idx = Index::create();
            idx.name(name).table(UpdateHistory::Table);
            for c in cols {
                idx.col(c);
            }
            manager.create_index(idx.to_owned()).await?;
        }

        // Recreate original update_output_lines
        manager
            .create_table(
                Table::create()
                    .table(UpdateOutputLines::Table)
                    .col(
                        ColumnDef::new(UpdateOutputLines::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(UpdateOutputLines::UpdateHistoryId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string(UpdateOutputLines::Stream))
                    .col(ColumnDef::new(UpdateOutputLines::Output).text().not_null())
                    .col(timestamp(UpdateOutputLines::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_output_lines_update_history")
                            .from(UpdateOutputLines::Table, UpdateOutputLines::UpdateHistoryId)
                            .to(UpdateHistory::Table, UpdateHistory::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_update_output_lines_update_history")
                    .table(UpdateOutputLines::Table)
                    .col(UpdateOutputLines::UpdateHistoryId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum UpdateBatches {
    Table,
    Id,
    TenantId,
    BatchType,
    Status,
    TotalCount,
    ActorType,
    ActorId,
    CreatedAt,
    CompletedAt,
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
    OutputBytes,
    ActorType,
    ActorId,
    StartedAt,
    CompletedAt,
    CreatedAt,
    UpdateCategory,
    BatchId,
}

#[derive(DeriveIden)]
enum UpdateOutputLines {
    Table,
    Id,
    UpdateHistoryId,
    Stream,
    Output,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
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
