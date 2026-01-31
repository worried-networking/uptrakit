use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AgentCertificates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AgentCertificates::CaFingerprint)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AgentCertificates::SerialNumber)
                            .string()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(AgentCertificates::CaFingerprint)
                            .col(AgentCertificates::SerialNumber),
                    )
                    .col(ColumnDef::new(AgentCertificates::AgentId).uuid().not_null())
                    .col(timestamp(AgentCertificates::NotBefore))
                    .col(timestamp(AgentCertificates::NotAfter))
                    .col(timestamp_null(AgentCertificates::RevokedAt))
                    .col(string_null(AgentCertificates::RevocationReason))
                    .col(timestamp(AgentCertificates::CreatedAt))
                    .col(timestamp_null(AgentCertificates::LastSeenAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_agent_certificates_agent_id")
                            .from(AgentCertificates::Table, AgentCertificates::AgentId)
                            .to(Agents::Table, Agents::Id),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_agent_certificates_agent_id_revoked_at")
                    .table(AgentCertificates::Table)
                    .col(AgentCertificates::AgentId)
                    .col(AgentCertificates::RevokedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AgentCertificates::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum AgentCertificates {
    Table,
    CaFingerprint,
    AgentId,
    SerialNumber,
    NotBefore,
    NotAfter,
    RevokedAt,
    RevocationReason,
    CreatedAt,
    LastSeenAt,
}

#[derive(DeriveIden)]
enum Agents {
    Table,
    Id,
}
