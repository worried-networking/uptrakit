use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m20260129_000001_initial::Tenants;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Services::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Services::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Services::TenantId).uuid().not_null())
                    .col(
                        ColumnDef::new(Services::ServiceType)
                            .string()
                            .not_null()
                            .default("agent"),
                    )
                    .col(string(Services::Hostname))
                    .col(string(Services::FriendlyName))
                    .col(string_null(Services::IpAddress))
                    .col(
                        ColumnDef::new(Services::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(string_uniq(Services::EnrollmentSecretHash))
                    .col(string_null(Services::ClientVersion))
                    .col(timestamp_null(Services::LastSeenAt))
                    .col(timestamp(Services::CreatedAt))
                    .col(timestamp(Services::UpdatedAt))
                    .col(timestamp_null(Services::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_services_tenant")
                            .from(Services::Table, Services::TenantId)
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
                    .name("idx_services_tenant_id")
                    .table(Services::Table)
                    .col(Services::TenantId)
                    .to_owned(),
            )
            .await?;

        // Index on service_type for type-filtered queries
        manager
            .create_index(
                Index::create()
                    .name("idx_services_service_type")
                    .table(Services::Table)
                    .col(Services::ServiceType)
                    .to_owned(),
            )
            .await?;

        // Composite index on (tenant_id, service_type)
        manager
            .create_index(
                Index::create()
                    .name("idx_services_tenant_id_service_type")
                    .table(Services::Table)
                    .col(Services::TenantId)
                    .col(Services::ServiceType)
                    .to_owned(),
            )
            .await?;

        // Index on enrollment_secret_hash for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_services_enrollment_secret_hash")
                    .table(Services::Table)
                    .col(Services::EnrollmentSecretHash)
                    .to_owned(),
            )
            .await?;

        // Index on status for filtered queries
        manager
            .create_index(
                Index::create()
                    .name("idx_services_status")
                    .table(Services::Table)
                    .col(Services::Status)
                    .to_owned(),
            )
            .await?;

        // Index on deactivated_at for soft-delete filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_services_deactivated_at")
                    .table(Services::Table)
                    .col(Services::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Services::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum Services {
    Table,
    Id,
    TenantId,
    ServiceType,
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
}
