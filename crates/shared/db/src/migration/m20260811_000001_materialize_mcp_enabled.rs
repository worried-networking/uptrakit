use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use sea_orm_migration::prelude::*;
use time::OffsetDateTime;
use uptrakit_db_tx::begin_immediate;

/// Materialize the pre-2026-08 `oauth.mcp_enabled` auto-enable rule.
///
/// Until this migration, a missing `oauth.mcp_enabled` row combined with a
/// configured `oauth.canonical_host` implicitly enabled the MCP OAuth
/// authorization server (`resolve_mcp_enabled` auto-enable arm). The rule is
/// being inverted to explicit opt-in so that setting a canonical host for
/// OIDC CIMD support cannot silently boot a public authorization server.
/// Deployments that were auto-enabled get an explicit `true` row here so the
/// inversion preserves their resolved state. One deliberate exception: a
/// stored empty-string host (`oauth.canonical_host = ""`) auto-enabled under
/// the old rule and then failed boot with `CanonicalHostMissing`; it gets no
/// row and boots disabled instead.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let txn = begin_immediate(manager.get_connection()).await?;

        let mcp_row = crate::entity::global_setting::Entity::find_by_id("oauth.mcp_enabled")
            .one(&txn)
            .await?;
        if mcp_row.is_none() {
            let host_set =
                crate::entity::global_setting::Entity::find_by_id("oauth.canonical_host")
                    .one(&txn)
                    .await?
                    .is_some_and(|row| row.value.as_str().is_some_and(|s| !s.is_empty()));
            if host_set {
                crate::entity::global_setting::ActiveModel {
                    key: Set("oauth.mcp_enabled".to_string()),
                    value: Set(serde_json::Value::Bool(true)),
                    updated_at: Set(OffsetDateTime::now_utc()),
                }
                .insert(&txn)
                .await?;
            }
        }

        txn.commit().await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Irreversible by design: deleting the row would re-enter the
        // auto-enable ambiguity this migration exists to remove.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait};
    use sea_orm_migration::prelude::*;

    use crate::migration::Migrator;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    fn migration_index() -> u32 {
        Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20260811_000001_materialize_mcp_enabled")
            .expect("materialize_mcp_enabled migration must be registered") as u32
    }

    /// Run migrations up to (but excluding) this one, seed via raw SQL
    /// statements (one `INSERT` per slice entry), then run the remaining
    /// migrations (this one included) through `Migrator`.
    async fn seed_and_run_up(seed_sql: &[&str]) -> DatabaseConnection {
        let db = test_db().await;
        Migrator::up(&db, Some(migration_index()))
            .await
            .expect("migrations before materialize_mcp_enabled must apply");

        for sql in seed_sql {
            db.execute_unprepared(sql)
                .await
                .expect("seed global_settings row");
        }

        Migrator::up(&db, None)
            .await
            .expect("remaining migrations, including materialize_mcp_enabled, must apply");

        db
    }

    async fn mcp_enabled_row(
        db: &DatabaseConnection,
    ) -> Option<crate::entity::global_setting::Model> {
        crate::entity::global_setting::Entity::find_by_id("oauth.mcp_enabled")
            .one(db)
            .await
            .expect("query oauth.mcp_enabled")
    }

    #[tokio::test]
    async fn materializes_true_when_host_set_and_row_absent() {
        let db = seed_and_run_up(&["INSERT INTO global_settings (key, value, updated_at) \
             VALUES ('oauth.canonical_host', '\"auth.example.com\"', '2026-08-11T00:00:00Z')"])
        .await;

        let row = mcp_enabled_row(&db)
            .await
            .expect("oauth.mcp_enabled row must be materialized");
        assert_eq!(row.value, serde_json::Value::Bool(true));
    }

    #[tokio::test]
    async fn no_row_written_when_host_absent() {
        let db = seed_and_run_up(&[]).await;

        assert!(
            mcp_enabled_row(&db).await.is_none(),
            "oauth.mcp_enabled must stay absent when no canonical host was ever configured"
        );
    }

    #[tokio::test]
    async fn explicit_row_untouched() {
        let db = seed_and_run_up(&[
            "INSERT INTO global_settings (key, value, updated_at) \
             VALUES ('oauth.canonical_host', '\"auth.example.com\"', '2026-08-11T00:00:00Z')",
            "INSERT INTO global_settings (key, value, updated_at) \
             VALUES ('oauth.mcp_enabled', 'false', '2026-08-11T00:00:00Z')",
        ])
        .await;

        let row = mcp_enabled_row(&db)
            .await
            .expect("existing oauth.mcp_enabled row must remain");
        assert_eq!(
            row.value,
            serde_json::Value::Bool(false),
            "an explicit existing row must not be overwritten"
        );
    }

    #[tokio::test]
    async fn null_host_value_is_not_set() {
        let db = seed_and_run_up(&["INSERT INTO global_settings (key, value, updated_at) \
             VALUES ('oauth.canonical_host', 'null', '2026-08-11T00:00:00Z')"])
        .await;

        assert!(
            mcp_enabled_row(&db).await.is_none(),
            "a JSON null canonical_host must not materialize oauth.mcp_enabled"
        );
    }

    #[tokio::test]
    async fn empty_host_string_is_not_set() {
        let db = seed_and_run_up(&["INSERT INTO global_settings (key, value, updated_at) \
             VALUES ('oauth.canonical_host', '\"\"', '2026-08-11T00:00:00Z')"])
        .await;

        assert!(
            mcp_enabled_row(&db).await.is_none(),
            "an empty-string canonical_host booted disabled under the old rule too \
             (CanonicalHostMissing) and must not materialize oauth.mcp_enabled"
        );
    }
}
