pub mod entity;
pub mod migration;

use std::path::Path;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

/// Initialize (or open) the local SQLite database for SSH host credentials.
///
/// The database file is stored in `<state_dir>/agent-ssh.db`. Migrations are
/// applied automatically on every startup.
pub async fn init_db(state_dir: &Path) -> std::result::Result<DatabaseConnection, sea_orm::DbErr> {
    let db_path = state_dir.join("agent-ssh.db");
    let url = format!("sqlite:{}?mode=rwc", db_path.display());

    let opt = ConnectOptions::new(url);
    let db = Database::connect(opt).await?;

    migration::run_migrations(&db).await?;

    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn init_db_creates_and_migrates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = init_db(dir.path()).await.expect("init_db should succeed");

        // Verify the ssh_hosts table exists by running a simple query.
        use sea_orm::ConnectionTrait;
        let result = db
            .execute_unprepared("SELECT count(*) FROM ssh_hosts")
            .await;
        assert!(
            result.is_ok(),
            "ssh_hosts table should exist after migration"
        );
    }

    #[tokio::test]
    async fn init_db_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");

        // First call creates the DB and runs migrations.
        let _db1 = init_db(dir.path()).await.expect("first init_db");
        // Second call re-opens and re-runs migrations (no-op).
        let _db2 = init_db(dir.path()).await.expect("second init_db");
    }
}
