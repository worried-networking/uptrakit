use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add machine_id column with a DEFAULT '' so existing rows are valid.
        // The empty string is overwritten on next connection to each host.
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .add_column(
                        ColumnDef::new(SshHosts::MachineId)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .drop_column(SshHosts::MachineId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
enum SshHosts {
    Table,
    MachineId,
}
