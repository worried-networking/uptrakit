use sea_orm_migration::prelude::*;

/// Add pagination-support indexes for the MQTT software-states host-page query.
///
/// | Index | Table | Columns | Purpose |
/// |---|---|---|---|
/// | `idx_hosts_tenant_id_id` | `hosts` | `(tenant_id, id)` | Ordered active-host pagination |
/// | `idx_hsi_host_id` | `host_software_items` | `(host_id)` | Per-host HSI lookup |
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_hosts_tenant_id_id")
                    .table(Hosts::Table)
                    .col(Hosts::TenantId)
                    .col(Hosts::Id)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hsi_host_id")
                    .table(HostSoftwareItems::Table)
                    .col(HostSoftwareItems::HostId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_hsi_host_id")
                    .table(HostSoftwareItems::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_hosts_tenant_id_id")
                    .table(Hosts::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    TenantId,
    Id,
}

#[derive(DeriveIden)]
enum HostSoftwareItems {
    Table,
    HostId,
}
