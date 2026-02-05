use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m20260129_000001_initial::Tenants;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create mqtt_services table
        manager
            .create_table(
                Table::create()
                    .table(MqttServices::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MqttServices::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MqttServices::TenantId).uuid().not_null())
                    .col(string(MqttServices::Hostname))
                    .col(string(MqttServices::FriendlyName))
                    .col(
                        ColumnDef::new(MqttServices::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(string_uniq(MqttServices::EnrollmentSecretHash))
                    .col(timestamp_null(MqttServices::LastSeenAt))
                    .col(timestamp(MqttServices::CreatedAt))
                    .col(timestamp(MqttServices::UpdatedAt))
                    .col(timestamp_null(MqttServices::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mqtt_services_tenant")
                            .from(MqttServices::Table, MqttServices::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Index on tenant_id for tenant-scoped queries
        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_services_tenant_id")
                    .table(MqttServices::Table)
                    .col(MqttServices::TenantId)
                    .to_owned(),
            )
            .await?;

        // Index on status for filtered queries
        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_services_status")
                    .table(MqttServices::Table)
                    .col(MqttServices::Status)
                    .to_owned(),
            )
            .await?;

        // Create mqtt_service_certificates table
        manager
            .create_table(
                Table::create()
                    .table(MqttServiceCertificates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MqttServiceCertificates::CaFingerprint)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MqttServiceCertificates::SerialNumber)
                            .string()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(MqttServiceCertificates::CaFingerprint)
                            .col(MqttServiceCertificates::SerialNumber),
                    )
                    .col(
                        ColumnDef::new(MqttServiceCertificates::MqttServiceId)
                            .uuid()
                            .not_null(),
                    )
                    .col(timestamp(MqttServiceCertificates::NotBefore))
                    .col(timestamp(MqttServiceCertificates::NotAfter))
                    .col(timestamp_null(MqttServiceCertificates::RevokedAt))
                    .col(string_null(MqttServiceCertificates::RevocationReason))
                    .col(timestamp(MqttServiceCertificates::CreatedAt))
                    .col(timestamp_null(MqttServiceCertificates::LastSeenAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mqtt_service_certificates_mqtt_service_id")
                            .from(
                                MqttServiceCertificates::Table,
                                MqttServiceCertificates::MqttServiceId,
                            )
                            .to(MqttServices::Table, MqttServices::Id),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_service_certificates_mqtt_service_id_revoked_at")
                    .table(MqttServiceCertificates::Table)
                    .col(MqttServiceCertificates::MqttServiceId)
                    .col(MqttServiceCertificates::RevokedAt)
                    .to_owned(),
            )
            .await?;

        // Create mqtt_enrollment_tokens table
        manager
            .create_table(
                Table::create()
                    .table(MqttEnrollmentTokens::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MqttEnrollmentTokens::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MqttEnrollmentTokens::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string(MqttEnrollmentTokens::Name))
                    .col(string_uniq(MqttEnrollmentTokens::TokenHash))
                    .col(timestamp_null(MqttEnrollmentTokens::ExpiresAt))
                    .col(
                        ColumnDef::new(MqttEnrollmentTokens::UsesRemaining)
                            .integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MqttEnrollmentTokens::CreatedBy)
                            .uuid()
                            .not_null(),
                    )
                    .col(timestamp(MqttEnrollmentTokens::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mqtt_enrollment_tokens_tenant")
                            .from(MqttEnrollmentTokens::Table, MqttEnrollmentTokens::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mqtt_enrollment_tokens_created_by")
                            .from(MqttEnrollmentTokens::Table, MqttEnrollmentTokens::CreatedBy)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_enrollment_tokens_tenant_id")
                    .table(MqttEnrollmentTokens::Table)
                    .col(MqttEnrollmentTokens::TenantId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MqttEnrollmentTokens::Table).to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(MqttServiceCertificates::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(MqttServices::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
pub enum MqttServices {
    Table,
    Id,
    TenantId,
    Hostname,
    FriendlyName,
    Status,
    EnrollmentSecretHash,
    LastSeenAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum MqttServiceCertificates {
    Table,
    CaFingerprint,
    MqttServiceId,
    SerialNumber,
    NotBefore,
    NotAfter,
    RevokedAt,
    RevocationReason,
    CreatedAt,
    LastSeenAt,
}

#[derive(DeriveIden)]
enum MqttEnrollmentTokens {
    Table,
    Id,
    TenantId,
    Name,
    TokenHash,
    ExpiresAt,
    UsesRemaining,
    CreatedBy,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
