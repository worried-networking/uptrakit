use sea_orm_migration::prelude::*;

use super::helpers::{self, timestamp_null};

/// Encrypt `plugin_configs.config` at rest and add `credential_updated_at`.
///
/// `up()`:
/// - PostgreSQL: widen `config` from `json` to `text` so the entity's
///   [`uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig`] newtype
///   (declared `Text` via `sea_query::ValueType::column_type()`) can decode
///   it by column OID. Existing plaintext JSON values survive the cast
///   unchanged; the app-level `reencrypt_to_v3` upgrade path (controller
///   boot) converts them to ciphertext afterward.
/// - SQLite: no schema change (see inline comment below).
/// - Both backends: add the nullable `credential_updated_at` timestamp,
///   stamped by the credential-rotation write path (not this migration —
///   every `ActiveModel` literal sets it to `None` for now).
///
/// `down()` refuses when any row's `config` already holds ciphertext
/// (`ENC:` prefix) — a migration has no access to the DEK ring, so
/// reverting an encrypted column to `json` would either fail the column
/// cast or silently truncate ciphertext into an invalid JSON document.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if helpers::is_sqlite(manager) {
            // SQLite: deliberate no-op — Json columns already store TEXT (type affinity), so no table recreation is needed (exception to the column-type-change→recreation default; see ADR). The declared column type remains 'json' while the entity declares Text — harmless via affinity; a future schema-comparison gate must not be surprised by it.
        } else {
            // Raw SQL exception (spec 2026-08-09 §4; comment protocol per docs/development/database-migrations.md): Postgres has no assignment cast from json to text, so the type change needs an explicit USING cast.
            #[expect(
                clippy::disallowed_methods,
                reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
            )]
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE plugin_configs ALTER COLUMN config TYPE text USING config::text",
                )
                .await?;
        }

        manager
            .alter_table(
                Table::alter()
                    .table(PluginConfigs::Table)
                    .add_column(timestamp_null(PluginConfigs::CredentialUpdatedAt))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let encrypted_count: i64 = manager
            .get_connection()
            .query_one(
                &Query::select()
                    .expr_as(Expr::col(PluginConfigs::Config).count(), Alias::new("cnt"))
                    .from(PluginConfigs::Table)
                    .and_where(Expr::col(PluginConfigs::Config).like("ENC:%"))
                    .to_owned(),
            )
            .await?
            .ok_or_else(|| DbErr::Custom("count query returned no row".to_string()))?
            .try_get("", "cnt")?;

        if encrypted_count > 0 {
            return Err(DbErr::Custom(
                "cannot revert: encrypted rows present in plugin_configs; decrypt is impossible in a migration (no DEK ring)".to_string(),
            ));
        }

        manager
            .alter_table(
                Table::alter()
                    .table(PluginConfigs::Table)
                    .drop_column(PluginConfigs::CredentialUpdatedAt)
                    .to_owned(),
            )
            .await?;

        if !helpers::is_sqlite(manager) {
            // Raw SQL exception (spec 2026-08-09 §4; comment protocol per docs/development/database-migrations.md): Postgres has no assignment cast from json to text, so the type change needs an explicit USING cast.
            #[expect(
                clippy::disallowed_methods,
                reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
            )]
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE plugin_configs ALTER COLUMN config TYPE json USING config::json",
                )
                .await?;
        }
        // SQLite: deliberate no-op — see the matching comment in `up()`.

        Ok(())
    }
}

#[derive(DeriveIden)]
enum PluginConfigs {
    Table,
    Config,
    CredentialUpdatedAt,
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
    use sea_orm_migration::prelude::*;
    use uuid::Uuid;

    use super::Migration;
    use crate::migration::Migrator;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    /// Insert a tenant with the nil UUID so FK constraints on
    /// `plugin_configs.tenant_id` are satisfied.
    async fn seed_tenant(db: &DatabaseConnection) {
        db.execute(
            &Query::insert()
                .into_table(Alias::new("tenants"))
                .columns([
                    Alias::new("id"),
                    Alias::new("name"),
                    Alias::new("slug"),
                    Alias::new("is_default"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    Uuid::nil().into(),
                    "Test Tenant".into(),
                    "test-tenant".into(),
                    true.into(),
                    "2026-01-01T00:00:00Z".into(),
                    "2026-01-01T00:00:00Z".into(),
                ])
                .to_owned(),
        )
        .await
        .expect("seed nil tenant for FK satisfaction");
    }

    async fn seed_plugin_config(db: &DatabaseConnection, name: &str, config: &str) {
        db.execute(
            &Query::insert()
                .into_table(Alias::new("plugin_configs"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("name"),
                    Alias::new("plugin_type"),
                    Alias::new("config"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    Uuid::now_v7().into(),
                    Uuid::nil().into(),
                    name.into(),
                    "releases_docker".into(),
                    config.into(),
                    "2026-01-01T00:00:00Z".into(),
                    "2026-01-01T00:00:00Z".into(),
                ])
                .to_owned(),
        )
        .await
        .expect("seed plugin_configs row");
    }

    async fn table_info(db: &DatabaseConnection, table: &str) -> Vec<String> {
        // `PRAGMA table_info(...)` is SQLite-specific with no sea_query
        // equivalent (see docs/development/database-migrations.md's
        // execute_unprepared/raw-statement exception table); query_all_raw
        // with a raw Statement is the approved exception for this pattern.
        #[expect(
            clippy::disallowed_methods,
            reason = "builder limitation: PRAGMA table_info() has no sea_query equivalent"
        )]
        db.query_all_raw(sea_orm::Statement::from_string(
            db.get_database_backend(),
            format!("PRAGMA table_info({table})"),
        ))
        .await
        .expect("pragma table_info")
        .into_iter()
        .map(|row| row.try_get::<String>("", "name").expect("column name"))
        .collect()
    }

    #[tokio::test]
    async fn up_adds_credential_updated_at() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up");

        let columns = table_info(&db, "plugin_configs").await;
        assert!(
            columns.iter().any(|c| c == "credential_updated_at"),
            "plugin_configs must gain credential_updated_at; got columns: {columns:?}"
        );
    }

    #[tokio::test]
    async fn up_then_down_on_plaintext_succeeds() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up");

        seed_tenant(&db).await;
        seed_plugin_config(&db, "cfg-plaintext", r#"{"foo":"bar"}"#).await;

        let schema_manager = SchemaManager::new(&db);
        Migration
            .down(&schema_manager)
            .await
            .expect("down must succeed when no encrypted rows are present");

        let columns = table_info(&db, "plugin_configs").await;
        assert!(
            !columns.iter().any(|c| c == "credential_updated_at"),
            "down must drop credential_updated_at; got columns: {columns:?}"
        );

        let row = db
            .query_one(
                &Query::select()
                    .column(Alias::new("config"))
                    .from(Alias::new("plugin_configs"))
                    .and_where(Expr::col(Alias::new("name")).eq("cfg-plaintext"))
                    .to_owned(),
            )
            .await
            .expect("query seeded row after down")
            .expect("seeded row must still exist after down");
        let config: String = row.try_get("", "config").expect("config column");
        assert_eq!(
            config, r#"{"foo":"bar"}"#,
            "down must preserve the seeded row's config data"
        );
    }

    #[tokio::test]
    async fn down_refuses_with_enc_rows() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up");

        seed_tenant(&db).await;
        seed_plugin_config(&db, "cfg-encrypted", "ENC:v3:deadbeef").await;

        let schema_manager = SchemaManager::new(&db);
        let err = Migration
            .down(&schema_manager)
            .await
            .expect_err("down must refuse when encrypted rows are present");
        let msg = err.to_string();
        assert!(
            msg.contains("cannot revert") && msg.contains("encrypted rows present"),
            "error must name the specific refusal reason, not just mention the table; got: {msg}"
        );
    }
}
