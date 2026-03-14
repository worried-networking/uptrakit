use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // pve_node_name: short Proxmox VE node name (e.g. "optiplex2").
        // Used to match discovered guests to their PVE host.
        // NULL for non-PVE hosts or hosts bootstrapped before this migration
        // (backfilled via `host sync`).
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .add_column(ColumnDef::new(SshHosts::PveNodeName).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .drop_column(SshHosts::PveNodeName)
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
enum SshHosts {
    Table,
    PveNodeName,
}
