use sea_orm_migration::prelude::*;

use crate::migration::helpers::timestamp;

/// Create the `service_merge_redirect` table.
///
/// Maps a deactivated source Service UUID to the active target Service UUID
/// produced by `merge_service`. The bearer-secret WS auth path consults this
/// table when an Agent reconnects with a `?service_id=hint` that no longer
/// matches an active row, so the Agent can be re-keyed onto the merge target
/// without operator intervention.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ServiceMergeRedirect::Table)
                    .col(
                        ColumnDef::new(ServiceMergeRedirect::SourceId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ServiceMergeRedirect::TargetId)
                            .uuid()
                            .not_null(),
                    )
                    .col(timestamp(ServiceMergeRedirect::RedirectedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_service_merge_redirect_target")
                            .from(ServiceMergeRedirect::Table, ServiceMergeRedirect::TargetId)
                            .to(Services::Table, Services::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(ServiceMergeRedirect::Table)
                    .name("idx_service_merge_redirect_target")
                    .col(ServiceMergeRedirect::TargetId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .table(ServiceMergeRedirect::Table)
                    .name("idx_service_merge_redirect_target")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(ServiceMergeRedirect::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum ServiceMergeRedirect {
    Table,
    SourceId,
    TargetId,
    RedirectedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
}
