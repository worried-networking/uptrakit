use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SshHosts::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(SshHosts::Id).text().not_null().primary_key())
                    .col(ColumnDef::new(SshHosts::Name).text().not_null())
                    .col(ColumnDef::new(SshHosts::Hostname).text().not_null())
                    .col(
                        ColumnDef::new(SshHosts::Port)
                            .integer()
                            .not_null()
                            .default(22),
                    )
                    .col(ColumnDef::new(SshHosts::Username).text().not_null())
                    .col(ColumnDef::new(SshHosts::PrivateKey).text().not_null())
                    .col(ColumnDef::new(SshHosts::KeyType).text().not_null())
                    .col(ColumnDef::new(SshHosts::HostKeyFingerprint).text())
                    .col(ColumnDef::new(SshHosts::CreatedAt).integer().not_null())
                    .col(ColumnDef::new(SshHosts::UpdatedAt).integer().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SshHosts::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum SshHosts {
    Table,
    Id,
    Name,
    Hostname,
    Port,
    Username,
    PrivateKey,
    KeyType,
    HostKeyFingerprint,
    CreatedAt,
    UpdatedAt,
}
