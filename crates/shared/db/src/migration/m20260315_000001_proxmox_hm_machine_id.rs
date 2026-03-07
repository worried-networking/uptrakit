use sea_orm_migration::prelude::*;

/// Add `machine_id` column to `proxmox_host_mappings`.
///
/// Populated best-effort during QEMU discovery via the guest agent
/// file-read endpoint (`/etc/machine-id`). LXC containers will have
/// `NULL` until the host reports its machine_id after bootstrap.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostMappings::Table)
                    .add_column(ColumnDef::new(ProxmoxHostMappings::MachineId).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostMappings::Table)
                    .drop_column(ProxmoxHostMappings::MachineId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ProxmoxHostMappings {
    Table,
    MachineId,
}
