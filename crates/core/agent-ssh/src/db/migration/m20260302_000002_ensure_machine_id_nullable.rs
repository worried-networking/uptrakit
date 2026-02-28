use sea_orm_migration::prelude::*;

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

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

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
                machine_id           TEXT,
                created_at           TEXT    NOT NULL,
                updated_at           TEXT    NOT NULL,
                sudo_available       INTEGER,
                is_root              INTEGER,
                sudo_policy          TEXT    NOT NULL DEFAULT 'auto'
            )",
        )
        .await?;

        conn.execute_unprepared(
            "INSERT INTO ssh_hosts_new
             SELECT id, name, hostname, port, username, private_key, key_type,
                    host_key_fingerprint, machine_id, created_at, updated_at,
                    sudo_available, is_root, sudo_policy
             FROM ssh_hosts",
        )
        .await?;

        conn.execute_unprepared("DROP TABLE ssh_hosts").await?;
        conn.execute_unprepared("ALTER TABLE ssh_hosts_new RENAME TO ssh_hosts").await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The only change made by `up` is ensuring machine_id is nullable;
        // rolling back to a NOT NULL column would re-introduce the bug.
        // Recreate with the same correct schema so down is safe and idempotent.
        let conn = manager.get_connection();

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
                machine_id           TEXT,
                created_at           TEXT    NOT NULL,
                updated_at           TEXT    NOT NULL,
                sudo_available       INTEGER,
                is_root              INTEGER,
                sudo_policy          TEXT    NOT NULL DEFAULT 'auto'
            )",
        )
        .await?;

        conn.execute_unprepared(
            "INSERT INTO ssh_hosts_new
             SELECT id, name, hostname, port, username, private_key, key_type,
                    host_key_fingerprint, machine_id, created_at, updated_at,
                    sudo_available, is_root, sudo_policy
             FROM ssh_hosts",
        )
        .await?;

        conn.execute_unprepared("DROP TABLE ssh_hosts").await?;
        conn.execute_unprepared("ALTER TABLE ssh_hosts_new RENAME TO ssh_hosts").await?;

        Ok(())
    }
}
