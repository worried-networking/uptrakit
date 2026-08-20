use sea_orm_migration::prelude::*;

use super::helpers;

/// Encrypt `instance_plugin_setting.config` at rest.
///
/// `up()`:
/// - PostgreSQL: widen `config` from `json` to `text` so the entity's
///   [`uptrakit_shared_db::encrypted_columns::EncryptedInstancePluginConfig`]
///   newtype (declared `Text` via `sea_query::ValueType::column_type()`) can
///   decode it by column OID. Existing plaintext JSON values survive the
///   cast unchanged; the app-level `reencrypt_to_v3` upgrade path
///   (controller boot) converts them to ciphertext afterward.
/// - SQLite: no schema change (see inline comment below).
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
                    "ALTER TABLE instance_plugin_setting ALTER COLUMN config TYPE text USING config::text",
                )
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let encrypted_count: i64 = manager
            .get_connection()
            .query_one(
                &Query::select()
                    .expr_as(
                        Expr::col(InstancePluginSetting::Config).count(),
                        Alias::new("cnt"),
                    )
                    .from(InstancePluginSetting::Table)
                    .and_where(Expr::col(InstancePluginSetting::Config).like("ENC:%"))
                    .to_owned(),
            )
            .await?
            .ok_or_else(|| DbErr::Custom("count query returned no row".to_string()))?
            .try_get("", "cnt")?;

        if encrypted_count > 0 {
            return Err(DbErr::Custom(
                "cannot revert: encrypted rows present in instance_plugin_setting; decrypt is impossible in a migration (no DEK ring)".to_string(),
            ));
        }

        if !helpers::is_sqlite(manager) {
            // Raw SQL exception (spec 2026-08-09 §4; comment protocol per docs/development/database-migrations.md): Postgres has no assignment cast from text to json, so the type change needs an explicit USING cast.
            #[expect(
                clippy::disallowed_methods,
                reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
            )]
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE instance_plugin_setting ALTER COLUMN config TYPE json USING config::json",
                )
                .await?;
        }
        // SQLite: deliberate no-op — see the matching comment in `up()`.

        Ok(())
    }
}

#[derive(DeriveIden)]
enum InstancePluginSetting {
    Table,
    Config,
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
    use sea_orm_migration::prelude::*;

    use super::Migration;
    use crate::migration::Migrator;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    /// `instance_plugin_setting` has no `tenant_id` FK, unlike `plugin_configs`
    /// and `plugin_type_settings` — no tenant seed helper is needed here.
    async fn seed_instance_plugin_setting(
        db: &DatabaseConnection,
        plugin_type_id: &str,
        config: &str,
    ) {
        db.execute(
            &Query::insert()
                .into_table(Alias::new("instance_plugin_setting"))
                .columns([
                    Alias::new("plugin_type_id"),
                    Alias::new("enabled"),
                    Alias::new("config"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    plugin_type_id.into(),
                    true.into(),
                    config.into(),
                    "2026-01-01T00:00:00Z".into(),
                ])
                .to_owned(),
        )
        .await
        .expect("seed instance_plugin_setting row");
    }

    #[tokio::test]
    async fn up_then_down_on_plaintext_succeeds() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up");

        seed_instance_plugin_setting(&db, "package-manager.apt", r#"{"foo":"bar"}"#).await;

        let schema_manager = SchemaManager::new(&db);
        Migration
            .down(&schema_manager)
            .await
            .expect("down must succeed when no encrypted rows are present");

        let row = db
            .query_one(
                &Query::select()
                    .column(Alias::new("config"))
                    .from(Alias::new("instance_plugin_setting"))
                    .and_where(Expr::col(Alias::new("plugin_type_id")).eq("package-manager.apt"))
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

        seed_instance_plugin_setting(&db, "package-manager.apt", "ENC:v3:deadbeef").await;

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
