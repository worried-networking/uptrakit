use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // sudo_available: NULL = unknown, TRUE = passwordless sudo works.
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .add_column(ColumnDef::new(SshHosts::SudoAvailable).boolean().null())
                    .to_owned(),
            )
            .await?;

        // is_root: NULL = unknown, TRUE = agent user is UID 0.
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .add_column(ColumnDef::new(SshHosts::IsRoot).boolean().null())
                    .to_owned(),
            )
            .await?;

        // sudo_policy: 'auto' | 'force_with' | 'force_without'.
        // DEFAULT 'auto' so existing rows are valid immediately.
        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .add_column(
                        ColumnDef::new(SshHosts::SudoPolicy)
                            .text()
                            .not_null()
                            .default("auto"),
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
                    .drop_column(SshHosts::SudoPolicy)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .drop_column(SshHosts::IsRoot)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(SshHosts::Table)
                    .drop_column(SshHosts::SudoAvailable)
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
enum SshHosts {
    Table,
    SudoAvailable,
    IsRoot,
    SudoPolicy,
}
