use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SettingsVersion::Table)
                    .add_column(
                        ColumnDef::new(SettingsVersion::RevocationVersion)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SettingsVersion::Table)
                    .drop_column(SettingsVersion::RevocationVersion)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum SettingsVersion {
    Table,
    RevocationVersion,
}
