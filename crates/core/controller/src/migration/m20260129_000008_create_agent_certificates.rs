use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m20260129_000007_create_agents::Services;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ServiceCertificates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ServiceCertificates::CaFingerprint)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ServiceCertificates::SerialNumber)
                            .string()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(ServiceCertificates::CaFingerprint)
                            .col(ServiceCertificates::SerialNumber),
                    )
                    .col(
                        ColumnDef::new(ServiceCertificates::ServiceId)
                            .uuid()
                            .not_null(),
                    )
                    .col(timestamp(ServiceCertificates::NotBefore))
                    .col(timestamp(ServiceCertificates::NotAfter))
                    .col(timestamp_null(ServiceCertificates::RevokedAt))
                    .col(string_null(ServiceCertificates::RevocationReason))
                    .col(timestamp(ServiceCertificates::CreatedAt))
                    .col(timestamp_null(ServiceCertificates::LastSeenAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_service_certificates_service_id")
                            .from(ServiceCertificates::Table, ServiceCertificates::ServiceId)
                            .to(Services::Table, Services::Id),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_service_certificates_service_id_revoked_at")
                    .table(ServiceCertificates::Table)
                    .col(ServiceCertificates::ServiceId)
                    .col(ServiceCertificates::RevokedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ServiceCertificates::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ServiceCertificates {
    Table,
    CaFingerprint,
    ServiceId,
    SerialNumber,
    NotBefore,
    NotAfter,
    RevokedAt,
    RevocationReason,
    CreatedAt,
    LastSeenAt,
}
