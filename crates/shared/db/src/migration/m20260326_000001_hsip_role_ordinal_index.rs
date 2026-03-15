use sea_orm_migration::prelude::*;

/// Replace `idx_hsip_host_item(host_id, software_item_id)` with a wider
/// composite index that also covers `role` and `ordinal`.
///
/// The new index `idx_hsip_host_item_role_ordinal(host_id, software_item_id,
/// role, ordinal)` supports:
///
/// - All existing queries that filter on `(host_id, software_item_id)` — via
///   leftmost-prefix matching.
/// - The new hook-plugin queries that filter on `(host_id, software_item_id,
///   role)` and ORDER BY `ordinal` — fully covered by the index.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the old 2-column index — subsumed by the new wider index.
        manager
            .drop_index(
                Index::drop()
                    .name("idx_hsip_host_item")
                    .table(HostSoftwareItemPlugins::Table)
                    .to_owned(),
            )
            .await?;

        // Create the wider 4-column composite index.
        manager
            .create_index(
                Index::create()
                    .name("idx_hsip_host_item_role_ordinal")
                    .table(HostSoftwareItemPlugins::Table)
                    .col(HostSoftwareItemPlugins::HostId)
                    .col(HostSoftwareItemPlugins::SoftwareItemId)
                    .col(HostSoftwareItemPlugins::Role)
                    .col(HostSoftwareItemPlugins::Ordinal)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the wider index and restore the original 2-column index.
        manager
            .drop_index(
                Index::drop()
                    .name("idx_hsip_host_item_role_ordinal")
                    .table(HostSoftwareItemPlugins::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hsip_host_item")
                    .table(HostSoftwareItemPlugins::Table)
                    .col(HostSoftwareItemPlugins::HostId)
                    .col(HostSoftwareItemPlugins::SoftwareItemId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum HostSoftwareItemPlugins {
    Table,
    HostId,
    SoftwareItemId,
    Role,
    Ordinal,
}
