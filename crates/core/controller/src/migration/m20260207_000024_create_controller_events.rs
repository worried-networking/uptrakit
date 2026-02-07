use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ControllerEvents::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ControllerEvents::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ControllerEvents::SourceControllerId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ControllerEvents::TargetServiceId).uuid())
                    .col(ColumnDef::new(ControllerEvents::TargetServiceType).text())
                    .col(
                        ColumnDef::new(ControllerEvents::MessageJson)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ControllerEvents::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Index for efficient polling: exclude own events + cursor-based scan
        manager
            .create_index(
                Index::create()
                    .name("idx_controller_events_source_id")
                    .table(ControllerEvents::Table)
                    .col(ControllerEvents::SourceControllerId)
                    .col(ControllerEvents::Id)
                    .to_owned(),
            )
            .await?;

        // Index for efficient cleanup of old events
        manager
            .create_index(
                Index::create()
                    .name("idx_controller_events_created_at")
                    .table(ControllerEvents::Table)
                    .col(ControllerEvents::CreatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ControllerEvents::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum ControllerEvents {
    Table,
    Id,
    SourceControllerId,
    TargetServiceId,
    TargetServiceType,
    MessageJson,
    CreatedAt,
}
