use sea_orm::{ConnectionTrait as _, DatabaseBackend, Statement, TransactionTrait as _};
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Convert `ssh_hosts.id` from TEXT to UUID and `ssh_hosts.pve_plugin_config_id`
/// from TEXT to UUID.
///
/// ## Background
///
/// The `ssh_hosts` table was originally created with `id TEXT PRIMARY KEY`.
/// The controller's shared entities use SeaORM's `.uuid()` column type, which
/// stores UUIDs as 16-byte BLOBs in SQLite. This migration aligns the SSH
/// agent's local database with the same convention.
///
/// ## Strategy
///
/// SQLite has no `ALTER COLUMN` statement, so we recreate the table:
///
/// 1. Create `ssh_hosts_new` with the correct column types.
/// 2. Read all rows from `ssh_hosts`, parse TEXT UUIDs into 16-byte BLOBs in
///    Rust, and insert into `ssh_hosts_new` using parameterised queries (so
///    sea-query binds the `Uuid` as a BLOB, not a TEXT literal).
/// 3. Drop `ssh_hosts` and rename `ssh_hosts_new` → `ssh_hosts`.
///
/// ## Why not INSERT…SELECT?
///
/// SeaORM/sqlx reads `.uuid()` columns via `sqlite3_column_blob()`.  If we
/// simply `INSERT INTO ssh_hosts_new SELECT * FROM ssh_hosts`, the existing
/// 36-character TEXT values would be copied verbatim, and SeaORM would fail
/// with `ParseByteLength { len: 36 }` when reading them back.  The TEXT→BLOB
/// conversion must happen in Rust via `Uuid::parse_str()` →
/// `Value::Bytes(uuid.as_bytes().to_vec())`.
///
/// ## Scope
///
/// SQLite-only (the agent-ssh database is always SQLite).
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // This migration is SQLite-only.
        if db.get_database_backend() != DatabaseBackend::Sqlite {
            return Ok(());
        }

        // 1. Create the replacement table with UUID column types.
        manager
            .create_table(
                Table::create()
                    .table(SshHostsNew::Table)
                    .col(
                        ColumnDef::new(SshHostsNew::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SshHostsNew::Name)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(SshHostsNew::Hostname).string().not_null())
                    .col(
                        ColumnDef::new(SshHostsNew::Port)
                            .integer()
                            .not_null()
                            .default(22),
                    )
                    .col(ColumnDef::new(SshHostsNew::Username).string().not_null())
                    .col(ColumnDef::new(SshHostsNew::PrivateKey).string().not_null())
                    .col(ColumnDef::new(SshHostsNew::KeyType).string().not_null())
                    .col(
                        ColumnDef::new(SshHostsNew::HostKeyFingerprint)
                            .string()
                            .null(),
                    )
                    .col(ColumnDef::new(SshHostsNew::MachineId).string().null())
                    .col(
                        ColumnDef::new(SshHostsNew::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SshHostsNew::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(SshHostsNew::SudoAvailable).boolean().null())
                    .col(ColumnDef::new(SshHostsNew::IsRoot).boolean().null())
                    .col(
                        ColumnDef::new(SshHostsNew::SudoPolicy)
                            .string()
                            .not_null()
                            .default("auto"),
                    )
                    .col(ColumnDef::new(SshHostsNew::PvePluginConfigId).uuid().null())
                    .col(ColumnDef::new(SshHostsNew::PveNodeName).string().null())
                    .to_owned(),
            )
            .await?;

        // 2. Read all existing rows and re-insert with proper UUID BLOB values.
        //
        // `typeof()` is a SQLite-specific function with no sea_query equivalent;
        // using query_all_raw with a Statement is the approved exception for this
        // pattern.
        let rows = db
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT id, name, hostname, port, username, private_key, key_type, \
                 host_key_fingerprint, machine_id, created_at, updated_at, \
                 sudo_available, is_root, sudo_policy, \
                 pve_plugin_config_id, pve_node_name \
                 FROM ssh_hosts",
            ))
            .await?;

        if !rows.is_empty() {
            let txn = db.begin().await?;

            for row in &rows {
                use sea_orm::TryGetable as _;

                // Column 0: id (TEXT → UUID BLOB)
                let id_str: String = String::try_get_by_index(row, 0)
                    .map_err(|e| DbErr::Custom(format!("failed to read id: {e:?}")))?;
                let id_uuid = Uuid::parse_str(&id_str).map_err(|e| {
                    DbErr::Custom(format!(
                        "invalid UUID text '{id_str}' for ssh_hosts.id: {e}"
                    ))
                })?;

                // Column 14: pve_plugin_config_id (nullable TEXT → nullable UUID BLOB)
                let pve_config_str: Option<String> = Option::<String>::try_get_by_index(row, 14)
                    .map_err(|e| {
                        DbErr::Custom(format!("failed to read pve_plugin_config_id: {e:?}"))
                    })?;
                let pve_config_blob: Value = match pve_config_str {
                    Some(ref s) if !s.is_empty() => {
                        let uuid = Uuid::parse_str(s).map_err(|e| {
                            DbErr::Custom(format!(
                                "invalid UUID text '{s}' for pve_plugin_config_id: {e}"
                            ))
                        })?;
                        Value::Bytes(Some(uuid.as_bytes().to_vec()))
                    }
                    _ => Value::Bytes(None),
                };

                // Read remaining columns as raw values.
                let name: String = String::try_get_by_index(row, 1)
                    .map_err(|e| DbErr::Custom(format!("failed to read name: {e:?}")))?;
                let hostname: String = String::try_get_by_index(row, 2)
                    .map_err(|e| DbErr::Custom(format!("failed to read hostname: {e:?}")))?;
                let port: i32 = i32::try_get_by_index(row, 3)
                    .map_err(|e| DbErr::Custom(format!("failed to read port: {e:?}")))?;
                let username: String = String::try_get_by_index(row, 4)
                    .map_err(|e| DbErr::Custom(format!("failed to read username: {e:?}")))?;
                let private_key: String = String::try_get_by_index(row, 5)
                    .map_err(|e| DbErr::Custom(format!("failed to read private_key: {e:?}")))?;
                let key_type: String = String::try_get_by_index(row, 6)
                    .map_err(|e| DbErr::Custom(format!("failed to read key_type: {e:?}")))?;
                let host_key_fingerprint: Option<String> =
                    Option::<String>::try_get_by_index(row, 7).map_err(|e| {
                        DbErr::Custom(format!("failed to read host_key_fingerprint: {e:?}"))
                    })?;
                let machine_id: Option<String> = Option::<String>::try_get_by_index(row, 8)
                    .map_err(|e| DbErr::Custom(format!("failed to read machine_id: {e:?}")))?;
                let created_at: String = String::try_get_by_index(row, 9)
                    .map_err(|e| DbErr::Custom(format!("failed to read created_at: {e:?}")))?;
                let updated_at: String = String::try_get_by_index(row, 10)
                    .map_err(|e| DbErr::Custom(format!("failed to read updated_at: {e:?}")))?;
                let sudo_available: Option<bool> = Option::<bool>::try_get_by_index(row, 11)
                    .map_err(|e| DbErr::Custom(format!("failed to read sudo_available: {e:?}")))?;
                let is_root: Option<bool> = Option::<bool>::try_get_by_index(row, 12)
                    .map_err(|e| DbErr::Custom(format!("failed to read is_root: {e:?}")))?;
                let sudo_policy: String = String::try_get_by_index(row, 13)
                    .map_err(|e| DbErr::Custom(format!("failed to read sudo_policy: {e:?}")))?;
                let pve_node_name: Option<String> = Option::<String>::try_get_by_index(row, 15)
                    .map_err(|e| DbErr::Custom(format!("failed to read pve_node_name: {e:?}")))?;

                // Insert with properly-bound UUID BLOB values.
                txn.execute(
                    &Query::insert()
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
                            SshHostsNew::PvePluginConfigId,
                            SshHostsNew::PveNodeName,
                        ])
                        .values_panic([
                            id_uuid.into(),
                            name.into(),
                            hostname.into(),
                            port.into(),
                            username.into(),
                            private_key.into(),
                            key_type.into(),
                            host_key_fingerprint.into(),
                            machine_id.into(),
                            created_at.into(),
                            updated_at.into(),
                            sudo_available.into(),
                            is_root.into(),
                            sudo_policy.into(),
                            Expr::val(pve_config_blob),
                            pve_node_name.into(),
                        ])
                        .to_owned(),
                )
                .await?;
            }

            txn.commit().await?;
        }

        // 3. Drop old table and rename.
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

    /// No-op: re-introducing TEXT storage would lose the BLOB representation.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

#[derive(DeriveIden)]
enum SshHosts {
    Table,
}

#[derive(DeriveIden)]
#[allow(clippy::enum_variant_names)]
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
    PvePluginConfigId,
    PveNodeName,
}
