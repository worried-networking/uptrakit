use sea_orm_migration::prelude::*;

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
        let conn = manager.get_connection();

        // Create the replacement table with TEXT timestamp columns.
        conn.execute_unprepared(
            "CREATE TABLE ssh_hosts_new (
                id                   TEXT    NOT NULL PRIMARY KEY,
                name                 TEXT    NOT NULL UNIQUE,
                hostname             TEXT    NOT NULL,
                port                 INTEGER NOT NULL DEFAULT 22,
                username             TEXT    NOT NULL,
                private_key          TEXT    NOT NULL,
                key_type             TEXT    NOT NULL,
                host_key_fingerprint TEXT,
                machine_id           TEXT    NOT NULL DEFAULT '',
                created_at           TEXT    NOT NULL,
                updated_at           TEXT    NOT NULL,
                sudo_available       INTEGER,
                is_root              INTEGER,
                sudo_policy          TEXT    NOT NULL DEFAULT 'auto'
            )",
        )
        .await?;

        // Copy data, converting INTEGER Unix epoch seconds to RFC 3339 text.
        // `strftime` with 'unixepoch' is a no-op on NULL but the columns are
        // NOT NULL so there will always be a value to convert.
        conn.execute_unprepared(
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

        conn.execute_unprepared("DROP TABLE ssh_hosts").await?;

        conn.execute_unprepared("ALTER TABLE ssh_hosts_new RENAME TO ssh_hosts")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Recreate the table with INTEGER timestamp columns.
        conn.execute_unprepared(
            "CREATE TABLE ssh_hosts_old (
                id                   TEXT    NOT NULL PRIMARY KEY,
                name                 TEXT    NOT NULL UNIQUE,
                hostname             TEXT    NOT NULL,
                port                 INTEGER NOT NULL DEFAULT 22,
                username             TEXT    NOT NULL,
                private_key          TEXT    NOT NULL,
                key_type             TEXT    NOT NULL,
                host_key_fingerprint TEXT,
                machine_id           TEXT    NOT NULL DEFAULT '',
                created_at           INTEGER NOT NULL,
                updated_at           INTEGER NOT NULL,
                sudo_available       INTEGER,
                is_root              INTEGER,
                sudo_policy          TEXT    NOT NULL DEFAULT 'auto'
            )",
        )
        .await?;

        // Convert RFC 3339 text back to INTEGER Unix epoch seconds.
        conn.execute_unprepared(
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

        conn.execute_unprepared("DROP TABLE ssh_hosts").await?;

        conn.execute_unprepared("ALTER TABLE ssh_hosts_old RENAME TO ssh_hosts")
            .await?;

        Ok(())
    }
}
