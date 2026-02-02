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
                    .table(Agents::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Agents::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Agents::TenantId).uuid().not_null())
                    .col(string(Agents::Hostname))
                    .col(string(Agents::FriendlyName))
                    .col(string_null(Agents::IpAddress))
                    .col(
                        ColumnDef::new(Agents::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(string_uniq(Agents::EnrollmentSecretHash))
                    .col(timestamp_null(Agents::LastSeenAt))
                    .col(timestamp(Agents::CreatedAt))
                    .col(timestamp(Agents::UpdatedAt))
                    .col(timestamp_null(Agents::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_agents_tenant")
                            .from(Agents::Table, Agents::TenantId)
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
                    .name("idx_agents_tenant_id")
                    .table(Agents::Table)
                    .col(Agents::TenantId)
                    .to_owned(),
            )
            .await?;

        // Index on enrollment_secret_hash for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_agents_enrollment_secret_hash")
                    .table(Agents::Table)
                    .col(Agents::EnrollmentSecretHash)
                    .to_owned(),
            )
            .await?;

        // Index on status for filtered queries
        manager
            .create_index(
                Index::create()
                    .name("idx_agents_status")
                    .table(Agents::Table)
                    .col(Agents::Status)
                    .to_owned(),
            )
            .await?;

        // Index on deactivated_at for soft-delete filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_agents_deactivated_at")
                    .table(Agents::Table)
                    .col(Agents::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Agents::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Agents {
    Table,
    Id,
    TenantId,
    Hostname,
    FriendlyName,
    IpAddress,
    Status,
    EnrollmentSecretHash,
    LastSeenAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}
