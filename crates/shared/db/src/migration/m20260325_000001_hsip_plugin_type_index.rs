use sea_orm_migration::prelude::*;

/// Add a compound index on `host_software_item_plugins(software_item_id, plugin_type)`
/// to support the new "filter software list by plugin type" EXISTS subquery efficiently.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_hsip_software_item_id_plugin_type")
                    .table(HostSoftwareItemPlugins::Table)
                    .col(HostSoftwareItemPlugins::SoftwareItemId)
                    .col(HostSoftwareItemPlugins::PluginType)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_hsip_software_item_id_plugin_type")
                    .table(HostSoftwareItemPlugins::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum HostSoftwareItemPlugins {
    Table,
    SoftwareItemId,
    PluginType,
}
