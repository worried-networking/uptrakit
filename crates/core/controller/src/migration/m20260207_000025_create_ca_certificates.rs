use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(CaCertificates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CaCertificates::Fingerprint)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(CaCertificates::CertPem).text().not_null())
                    .col(ColumnDef::new(CaCertificates::KeyPem).text().not_null())
                    .col(
                        ColumnDef::new(CaCertificates::NotBefore)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CaCertificates::NotAfter)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CaCertificates::ActivatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(CaCertificates::DeactivatedAt).timestamp())
                    .col(
                        ColumnDef::new(CaCertificates::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_ca_certificates_not_after")
                    .table(CaCertificates::Table)
                    .col(CaCertificates::NotAfter)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_ca_certificates_deactivated_at")
                    .table(CaCertificates::Table)
                    .col(CaCertificates::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(CaCertificates::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum CaCertificates {
    Table,
    Fingerprint,
    CertPem,
    KeyPem,
    NotBefore,
    NotAfter,
    ActivatedAt,
    DeactivatedAt,
    CreatedAt,
}
