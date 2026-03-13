use sea_orm_migration::prelude::*;

/// Drop the `is_pve_node` column from `ssh_hosts`.
///
/// This column was always `false` — PVE node detection state is tracked in
/// the Proxmox plugin's own `proxmox_host_state` table.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .drop_column(SshHosts::IsPveNode)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
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
            .await
    }
}

#[derive(Iden)]
enum SshHosts {
    Table,
    IsPveNode,
}
