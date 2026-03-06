use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // is_pve_node: whether this host is a Proxmox VE node (default false).
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .add_column(
                        ColumnDef::new(SshHosts::IsPveNode)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;

        // pve_plugin_config_id: controller-side plugin config ID for this PVE node.
        // NULL for non-PVE hosts.
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .add_column(ColumnDef::new(SshHosts::PvePluginConfigId).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .drop_column(SshHosts::PvePluginConfigId)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .drop_column(SshHosts::IsPveNode)
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
enum SshHosts {
    Table,
    IsPveNode,
    PvePluginConfigId,
}
