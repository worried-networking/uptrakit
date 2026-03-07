use sea_orm_migration::prelude::*;

/// Replace the plain unique index `uq_hosts_tenant_machine_id` on
/// `hosts(tenant_id, machine_id)` with a partial unique index that only
/// covers active (non-deactivated) rows.
///
/// Previously, a deactivated host with a given `machine_id` blocked the
/// creation of a new host record for the same machine. With the partial
/// index, deactivated rows are excluded and the agent can register a fresh
/// host record after the old one has been removed.
///
/// Note: MySQL does not support partial indexes; the WHERE clause is
/// silently ignored. The application layer enforces the condition.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the old plain unique index.
        manager
            .drop_index(
                Index::drop()
                    .name("uq_hosts_tenant_machine_id")
                    .table(Hosts::Table)
                    .to_owned(),
            )
            .await?;

        // Create a partial unique index scoped to active hosts only.
        manager
            .create_index(
                Index::create()
                    .name("uq_hosts_active_tenant_machine_id")
                    .table(Hosts::Table)
                    .col(Hosts::TenantId)
                    .col(Hosts::MachineId)
                    .unique()
                    .and_where(Expr::col(Hosts::DeactivatedAt).is_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("uq_hosts_active_tenant_machine_id")
                    .table(Hosts::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_hosts_tenant_machine_id")
                    .table(Hosts::Table)
                    .col(Hosts::TenantId)
                    .col(Hosts::MachineId)
                    .unique()
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    TenantId,
    MachineId,
    DeactivatedAt,
}
