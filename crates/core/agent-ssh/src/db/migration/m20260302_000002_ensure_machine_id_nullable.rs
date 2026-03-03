use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

/// Guarantee that `ssh_hosts.machine_id` is a nullable column.
///
/// An earlier incarnation of [`m20260302_000001_convert_ssh_host_timestamps`]
/// may have shipped `machine_id TEXT NOT NULL DEFAULT ''`.  Any database
/// migrated by that version will fail when `add_host` inserts `NULL` for the
/// column (introduced by the `machine_id: Option<String>` fix in the entity).
///
/// This migration unconditionally recreates the table with the correct schema,
/// ensuring `machine_id` has no `NOT NULL` constraint on every installation.
#[derive(DeriveMigrationName)]
pub struct Migration;

/// Recreate `ssh_hosts` with the correct schema, copying all data.
///
/// Used by both `up()` and `down()` since the schema is identical in both
/// directions (nullable `machine_id`).
async fn recreate_ssh_hosts(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(SshHostsNew::Table)
                .col(
                    ColumnDef::new(SshHostsNew::Id)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(string_uniq(SshHostsNew::Name))
                .col(string(SshHostsNew::Hostname))
                .col(
                    ColumnDef::new(SshHostsNew::Port)
                        .integer()
                        .not_null()
                        .default(22),
                )
                .col(string(SshHostsNew::Username))
                .col(string(SshHostsNew::PrivateKey))
                .col(string(SshHostsNew::KeyType))
                .col(string_null(SshHostsNew::HostKeyFingerprint))
                .col(string_null(SshHostsNew::MachineId))
                .col(timestamp(SshHostsNew::CreatedAt))
                .col(timestamp(SshHostsNew::UpdatedAt))
                .col(ColumnDef::new(SshHostsNew::SudoAvailable).integer())
                .col(ColumnDef::new(SshHostsNew::IsRoot).integer())
                .col(
                    ColumnDef::new(SshHostsNew::SudoPolicy)
                        .string()
                        .not_null()
                        .default("auto"),
                )
                .to_owned(),
        )
        .await?;

    let insert = Query::insert()
        .into_table(SshHostsNew::Table)
        .columns([
            SshHostsNew::Id,
            SshHostsNew::Name,
            SshHostsNew::Hostname,
            SshHostsNew::Port,
            SshHostsNew::Username,
            SshHostsNew::PrivateKey,
            SshHostsNew::KeyType,
            SshHostsNew::HostKeyFingerprint,
            SshHostsNew::MachineId,
            SshHostsNew::CreatedAt,
            SshHostsNew::UpdatedAt,
            SshHostsNew::SudoAvailable,
            SshHostsNew::IsRoot,
            SshHostsNew::SudoPolicy,
        ])
        .select_from(
            Query::select()
                .column(SshHosts::Id)
                .column(SshHosts::Name)
                .column(SshHosts::Hostname)
                .column(SshHosts::Port)
                .column(SshHosts::Username)
                .column(SshHosts::PrivateKey)
                .column(SshHosts::KeyType)
                .column(SshHosts::HostKeyFingerprint)
                .column(SshHosts::MachineId)
                .column(SshHosts::CreatedAt)
                .column(SshHosts::UpdatedAt)
                .column(SshHosts::SudoAvailable)
                .column(SshHosts::IsRoot)
                .column(SshHosts::SudoPolicy)
                .from(SshHosts::Table)
                .to_owned(),
        )
        .map_err(|e| DbErr::Migration(e.to_string()))?
        .to_owned();
    manager.exec_stmt(insert).await?;

    manager
        .drop_table(Table::drop().table(SshHosts::Table).to_owned())
        .await?;

    manager
        .rename_table(
            Table::rename()
                .table(SshHostsNew::Table, SshHosts::Table)
                .to_owned(),
        )
        .await?;

    Ok(())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        recreate_ssh_hosts(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The only change made by `up` is ensuring machine_id is nullable;
        // rolling back to a NOT NULL column would re-introduce the bug.
        // Recreate with the same correct schema so down is safe and idempotent.
        recreate_ssh_hosts(manager).await
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
}

#[derive(DeriveIden)]
enum SshHostsNew {
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
}
