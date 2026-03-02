use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

/// Create `system_services` and `system_service_certificates` tables.
///
/// System services (MQTT bridge, external scheduler) are global infrastructure
/// that serves all tenants. Unlike tenant-scoped `services`, they carry no
/// `tenant_id` or `enrollment_token_id`.
///
/// `system_service_certificates` mirrors `service_certificates` with a FK to
/// `system_services` instead of `services`.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // --- system_services ---
        manager
            .create_table(
                Table::create()
                    .table(SystemServices::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SystemServices::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SystemServices::Capabilities)
                            .text()
                            .not_null()
                            .default("[]"),
                    )
                    .col(string(SystemServices::Hostname))
                    .col(string(SystemServices::FriendlyName))
                    .col(string_null(SystemServices::IpAddress))
                    .col(
                        ColumnDef::new(SystemServices::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(string_uniq(SystemServices::EnrollmentSecretHash))
                    .col(string_null(SystemServices::ClientVersion))
                    .col(timestamp_null(SystemServices::LastSeenAt))
                    .col(timestamp(SystemServices::CreatedAt))
                    .col(timestamp(SystemServices::UpdatedAt))
                    .col(timestamp_null(SystemServices::DeactivatedAt))
                    .col(
                        ColumnDef::new(SystemServices::PingIntervalSeconds)
                            .integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(SystemServices::CertLifetimeHours)
                            .integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_system_services_status")
                    .table(SystemServices::Table)
                    .col(SystemServices::Status)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_system_services_deactivated_at")
                    .table(SystemServices::Table)
                    .col(SystemServices::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // --- system_service_certificates ---
        manager
            .create_table(
                Table::create()
                    .table(SystemServiceCertificates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SystemServiceCertificates::CaFingerprint)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SystemServiceCertificates::SerialNumber)
                            .string()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(SystemServiceCertificates::CaFingerprint)
                            .col(SystemServiceCertificates::SerialNumber),
                    )
                    .col(
                        ColumnDef::new(SystemServiceCertificates::SystemServiceId)
                            .uuid()
                            .not_null(),
                    )
                    .col(timestamp(SystemServiceCertificates::NotBefore))
                    .col(timestamp(SystemServiceCertificates::NotAfter))
                    .col(timestamp_null(SystemServiceCertificates::RevokedAt))
                    .col(string_null(SystemServiceCertificates::RevocationReason))
                    .col(timestamp(SystemServiceCertificates::CreatedAt))
                    .col(timestamp_null(SystemServiceCertificates::LastSeenAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_system_svc_certs_system_service_id")
                            .from(
                                SystemServiceCertificates::Table,
                                SystemServiceCertificates::SystemServiceId,
                            )
                            .to(SystemServices::Table, SystemServices::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_system_svc_certs_ca_fingerprint")
                            .from(
                                SystemServiceCertificates::Table,
                                SystemServiceCertificates::CaFingerprint,
                            )
                            .to(CaCertificates::Table, CaCertificates::Fingerprint)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_system_svc_certs_svc_revoked")
                    .table(SystemServiceCertificates::Table)
                    .col(SystemServiceCertificates::SystemServiceId)
                    .col(SystemServiceCertificates::RevokedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(SystemServiceCertificates::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(Table::drop().table(SystemServices::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum SystemServices {
    Table,
    Id,
    Capabilities,
    Hostname,
    FriendlyName,
    IpAddress,
    Status,
    EnrollmentSecretHash,
    ClientVersion,
    LastSeenAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
    PingIntervalSeconds,
    CertLifetimeHours,
}

#[derive(DeriveIden)]
enum SystemServiceCertificates {
    Table,
    CaFingerprint,
    SerialNumber,
    SystemServiceId,
    NotBefore,
    NotAfter,
    RevokedAt,
    RevocationReason,
    CreatedAt,
    LastSeenAt,
}

#[derive(DeriveIden)]
enum CaCertificates {
    Table,
    Fingerprint,
}
