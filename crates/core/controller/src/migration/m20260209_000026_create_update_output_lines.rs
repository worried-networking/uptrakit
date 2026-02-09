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
                    .table(UpdateOutputLines::Table)
                    .if_not_exists()
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
        manager
            .drop_table(Table::drop().table(UpdateOutputLines::Table).to_owned())
            .await?;
        Ok(())
    }
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
enum UpdateHistory {
    Table,
    Id,
}
