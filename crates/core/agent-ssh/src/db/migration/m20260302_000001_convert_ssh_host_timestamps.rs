use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

/// Convert `ssh_hosts.created_at` and `ssh_hosts.updated_at` from INTEGER (Unix
/// epoch seconds) to TEXT (RFC 3339 / ISO-8601), matching the timestamp column
/// type used by every other entity in the workspace.
///
/// SQLite has no native DATETIME type and no `ALTER COLUMN` statement, so we
/// recreate the table with the correct schema, copy the data with an in-place
/// format conversion, drop the old table, and rename the new one.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create the replacement table with TEXT timestamp columns.
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

        // Copy data, converting INTEGER Unix epoch seconds to RFC 3339 text.
        // `strftime` is a SQLite-specific function with no sea_query equivalent;
        // execute_unprepared is the approved exception for this pattern.
        manager
            .get_connection()
            .execute_unprepared(
                "INSERT INTO ssh_hosts_new
                 SELECT
                   id, name, hostname, port, username, private_key, key_type,
                   host_key_fingerprint, machine_id,
                   strftime('%Y-%m-%dT%H:%M:%S+00:00', created_at, 'unixepoch'),
                   strftime('%Y-%m-%dT%H:%M:%S+00:00', updated_at, 'unixepoch'),
                   sudo_available, is_root, sudo_policy
                 FROM ssh_hosts",
            )
            .await?;

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

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Recreate the table with INTEGER timestamp columns.
        manager
            .create_table(
                Table::create()
                    .table(SshHostsOld::Table)
                    .col(
                        ColumnDef::new(SshHostsOld::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(string_uniq(SshHostsOld::Name))
                    .col(string(SshHostsOld::Hostname))
                    .col(
                        ColumnDef::new(SshHostsOld::Port)
                            .integer()
                            .not_null()
                            .default(22),
                    )
                    .col(string(SshHostsOld::Username))
                    .col(string(SshHostsOld::PrivateKey))
                    .col(string(SshHostsOld::KeyType))
                    .col(string_null(SshHostsOld::HostKeyFingerprint))
                    .col(string_null(SshHostsOld::MachineId))
                    .col(ColumnDef::new(SshHostsOld::CreatedAt).integer().not_null())
                    .col(ColumnDef::new(SshHostsOld::UpdatedAt).integer().not_null())
                    .col(ColumnDef::new(SshHostsOld::SudoAvailable).integer())
                    .col(ColumnDef::new(SshHostsOld::IsRoot).integer())
                    .col(
                        ColumnDef::new(SshHostsOld::SudoPolicy)
                            .string()
                            .not_null()
                            .default("auto"),
                    )
                    .to_owned(),
            )
            .await?;

        // Convert RFC 3339 text back to INTEGER Unix epoch seconds.
        // `strftime` is a SQLite-specific function with no sea_query equivalent;
        // execute_unprepared is the approved exception for this pattern.
        manager
            .get_connection()
            .execute_unprepared(
                "INSERT INTO ssh_hosts_old
                 SELECT
                   id, name, hostname, port, username, private_key, key_type,
                   host_key_fingerprint, machine_id,
                   CAST(strftime('%s', created_at) AS INTEGER),
                   CAST(strftime('%s', updated_at) AS INTEGER),
                   sudo_available, is_root, sudo_policy
                 FROM ssh_hosts",
            )
            .await?;

        manager
            .drop_table(Table::drop().table(SshHosts::Table).to_owned())
            .await?;

        manager
            .rename_table(
                Table::rename()
                    .table(SshHostsOld::Table, SshHosts::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum SshHosts {
    Table,
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

#[derive(DeriveIden)]
enum SshHostsOld {
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
