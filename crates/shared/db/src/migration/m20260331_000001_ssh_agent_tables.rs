use sea_orm_migration::prelude::*;

/// Create tables used by the SSH agent when running in embedded mode.
///
/// In embedded mode the SSH agent shares the controller's database instead of
/// maintaining its own SQLite file. These three tables mirror the agent-local
/// schema:
///
/// - `ssh_hosts` -- managed SSH host inventory
/// - `proxmox_host_state` -- Proxmox node discovery state
/// - `proxmox_pending_matches` -- pending Proxmox VM-to-host matches
///
/// All tables use `if_not_exists()` for idempotency.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ── ssh_hosts ──────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(SshHosts::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(SshHosts::Id).uuid().not_null().primary_key())
                    .col(
                        ColumnDef::new(SshHosts::Name)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
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
                    .col(ColumnDef::new(SshHosts::HostKeyFingerprint).text().null())
                    .col(ColumnDef::new(SshHosts::MachineId).text().null())
                    .col(
                        ColumnDef::new(SshHosts::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SshHosts::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(SshHosts::SudoAvailable).boolean().null())
                    .col(ColumnDef::new(SshHosts::IsRoot).boolean().null())
                    .col(
                        ColumnDef::new(SshHosts::SudoPolicy)
                            .text()
                            .not_null()
                            .default("auto"),
                    )
                    .col(ColumnDef::new(SshHosts::PvePluginConfigId).uuid().null())
                    .col(ColumnDef::new(SshHosts::PveNodeName).text().null())
                    .to_owned(),
            )
            .await?;

        // ── proxmox_host_state ─────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxHostState::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ProxmoxHostState::HostId)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostState::IsPveNode)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(ProxmoxHostState::PveNodeName).text().null())
                    .col(
                        ColumnDef::new(ProxmoxHostState::PvePluginConfigId)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostState::CreatedAt)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostState::UpdatedAt)
                            .text()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ── proxmox_pending_matches ────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxPendingMatches::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ProxmoxPendingMatches::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxPendingMatches::HostId)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxPendingMatches::MappingId)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxPendingMatches::CreatedAt)
                            .text()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxPendingMatches::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(ProxmoxHostState::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(Table::drop().table(SshHosts::Table).if_exists().to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum SshHosts {
    Table,
    Id,
    Name,
    Hostname,
    Port,
    Username,
    PrivateKey,
    KeyType,
    HostKeyFingerprint,
    MachineId,
    CreatedAt,
    UpdatedAt,
    SudoAvailable,
    IsRoot,
    SudoPolicy,
    PvePluginConfigId,
    PveNodeName,
}

#[derive(DeriveIden)]
enum ProxmoxHostState {
    Table,
    HostId,
    IsPveNode,
    PveNodeName,
    PvePluginConfigId,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ProxmoxPendingMatches {
    Table,
    Id,
    HostId,
    MappingId,
    CreatedAt,
}
