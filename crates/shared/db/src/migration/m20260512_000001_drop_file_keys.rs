use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use sea_orm_migration::prelude::*;
use uptrakit_db_tx::begin_immediate;

/// Drop `global_settings` rows whose keys have moved to TOML config.
///
/// After the graceful-reload migration (spec §6.3, §20), the settings listed in
/// `FILE_ONLY_KEYS` are read from the TOML config file only. Any row that was
/// written to `global_settings` in a prior release is now stale. This migration
/// purges those rows so that the DB is not a source of confusion.
///
/// Per-tenant `audit_log.*` overrides stored in the `settings` table are
/// unaffected — only rows in `global_settings` with the exact keys below are
/// removed.
///
/// The migration is irreversible: operators rolling back must re-populate via
/// the prior TOML/env mechanism (out of scope).
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// Settings keys that have moved from `global_settings` DB rows to TOML config.
const FILE_ONLY_KEYS: &[&str] = &[
    "network.https_addr",
    "network.pki_addr",
    "network.trusted_proxies",
    "network.real_ip_header",
    "network.sans",
    "network.forwarded_client_cert_info_header",
    "network.forwarded_client_cert_pem_header",
    "nats.url",
    "zeroconf.enabled",
    "zeroconf.url",
    "zeroconf.pki_addr",
    "audit_log.filter",
    "audit_log.retention_days",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Read-then-write inside one transaction. Per the snapshot rule
        // "BEGIN IMMEDIATE for read-then-write", we use SqliteTransactionMode::Immediate
        // so that a concurrent writer between SELECT and DELETE does not produce
        // SQLITE_BUSY_SNAPSHOT (code 5). No-op on Postgres.
        let txn = begin_immediate(manager.get_connection()).await?;

        let rows = crate::entity::global_setting::Entity::find()
            .filter(
                crate::entity::global_setting::Column::Key.is_in(FILE_ONLY_KEYS.iter().copied()),
            )
            .all(&txn)
            .await?;

        for row in &rows {
            tracing::warn!(
                key = %row.key,
                "dropping global_settings row; key moved to TOML (spec §6.3, §20)"
            );
        }

        crate::entity::global_setting::Entity::delete_many()
            .filter(
                crate::entity::global_setting::Column::Key.is_in(FILE_ONLY_KEYS.iter().copied()),
            )
            .exec(&txn)
            .await?;

        txn.commit().await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Irreversible — File-only keys do not round-trip back to DB after the upgrade.
        // Operators rolling back must re-populate via the prior TOML/env mechanism.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
    use sea_orm_migration::prelude::*;

    use super::{FILE_ONLY_KEYS, Migration};
    use crate::migration::Migrator;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    fn migration_index() -> u32 {
        Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20260512_000001_drop_file_keys")
            .expect("drop_file_keys migration must be registered") as u32
    }

    /// Seed a row in `global_settings`, run `up()`, assert only non-file-only keys survive.
    #[tokio::test]
    async fn up_removes_file_only_keys() {
        let db = test_db().await;
        Migrator::up(&db, Some(migration_index() + 1))
            .await
            .expect("migrations through drop_file_keys must apply");

        // Insert one file-only key and one that must survive.
        db.execute_unprepared(
            "INSERT INTO global_settings (key, value, updated_at) \
             VALUES ('network.https_addr', '\"https://example.com\"', CURRENT_TIMESTAMP)",
        )
        .await
        .expect("seed file-only key");

        db.execute_unprepared(
            "INSERT INTO global_settings (key, value, updated_at) \
             VALUES ('registration.mode', '\"open\"', CURRENT_TIMESTAMP)",
        )
        .await
        .expect("seed survivor key");

        // Re-run this migration's up() directly.
        let schema_manager = SchemaManager::new(&db);
        Migration
            .up(&schema_manager)
            .await
            .expect("up() must succeed");

        // file-only key must be gone
        let row = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS cnt FROM global_settings WHERE key = 'network.https_addr'"
                    .to_string(),
            ))
            .await
            .expect("count query should succeed")
            .expect("count row must exist");
        let cnt: i64 = row.try_get("", "cnt").expect("cnt column");
        assert_eq!(cnt, 0, "network.https_addr must be deleted by up()");

        // survivor key must remain
        let row = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS cnt FROM global_settings WHERE key = 'registration.mode'"
                    .to_string(),
            ))
            .await
            .expect("count query should succeed")
            .expect("count row must exist");
        let cnt: i64 = row.try_get("", "cnt").expect("cnt column");
        assert_eq!(cnt, 1, "registration.mode must survive up()");
    }

    /// All FILE_ONLY_KEYS are removed when rows exist for every one of them.
    #[tokio::test]
    async fn up_removes_all_file_only_keys() {
        let db = test_db().await;
        Migrator::up(&db, Some(migration_index() + 1))
            .await
            .expect("migrations through drop_file_keys must apply");

        for key in FILE_ONLY_KEYS {
            let sql = format!(
                "INSERT INTO global_settings (key, value, updated_at) \
                 VALUES ('{key}', '\"val\"', CURRENT_TIMESTAMP)"
            );
            db.execute_unprepared(&sql).await.expect("seed row");
        }

        let schema_manager = SchemaManager::new(&db);
        Migration
            .up(&schema_manager)
            .await
            .expect("up() must succeed");

        let row = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS cnt FROM global_settings".to_string(),
            ))
            .await
            .expect("count query should succeed")
            .expect("count row must exist");
        let cnt: i64 = row.try_get("", "cnt").expect("cnt column");
        assert_eq!(cnt, 0, "all file-only rows must be deleted");
    }

    /// `down()` is a no-op — it must not fail.
    #[tokio::test]
    async fn down_is_noop() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up all");
        let schema_manager = SchemaManager::new(&db);
        Migration
            .down(&schema_manager)
            .await
            .expect("down() is a no-op and must not fail");
    }
}
