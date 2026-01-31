use std::fmt;

use rootcause::Report;
use sea_orm::DatabaseConnection;

use uptrakit_web_api::settings_store::{load_setting, upsert_setting};

/// Error type used for reconciliation failures.
#[derive(Debug, thiserror::Error)]
#[error("settings reconciliation failed")]
pub struct ReconcileError;

/// Reconcile a single DB-managed setting with an optional CLI value.
///
/// The five cases:
/// 1. DB has value + CLI provided + differs + `force` → use CLI, update DB
/// 2. DB has value + CLI provided + differs + no force → use DB, log warning
/// 3. DB has value + (CLI absent OR same) → use DB
/// 4. No DB value + CLI provided → use CLI, save to DB
/// 5. No DB value + CLI absent → use default, save to DB
pub async fn reconcile_setting<T>(
    db: &DatabaseConnection,
    key: &str,
    cli_value: Option<T>,
    default_value: T,
    force: bool,
    to_json: fn(&T) -> serde_json::Value,
    from_json: fn(&serde_json::Value) -> Option<T>,
) -> Result<T, Report<ReconcileError>>
where
    T: PartialEq + Clone + fmt::Display,
{
    let db_value = load_setting(db, key)
        .await
        .map_err(|e| {
            tracing::error!(key, error = ?e, "failed to load setting from DB");
            rootcause::report!(ReconcileError)
        })?
        .and_then(|v| from_json(&v));

    match (db_value, cli_value) {
        // Case 1 & 2: DB has a value and CLI differs
        (Some(db_val), Some(cli_val)) if db_val != cli_val => {
            if force {
                // Case 1: force override — use CLI, update DB
                tracing::info!(key, cli = %cli_val, db = %db_val, "force-overriding DB setting with CLI value");
                upsert_setting(db, key, to_json(&cli_val))
                    .await
                    .map_err(|e| {
                        tracing::error!(key, error = ?e, "failed to upsert setting");
                        rootcause::report!(ReconcileError)
                    })?;
                Ok(cli_val)
            } else {
                // Case 2: no force — use DB, warn
                tracing::warn!(
                    key,
                    cli = %cli_val,
                    db = %db_val,
                    "CLI value differs from DB; using DB value (pass --force-settings-override to overwrite)"
                );
                Ok(db_val)
            }
        }
        // Case 3: DB has value, CLI either absent or same
        (Some(db_val), _) => {
            tracing::debug!(key, value = %db_val, "using DB value");
            Ok(db_val)
        }
        // Case 4: No DB value, CLI provided
        (None, Some(cli_val)) => {
            tracing::info!(key, value = %cli_val, "seeding DB setting from CLI");
            upsert_setting(db, key, to_json(&cli_val))
                .await
                .map_err(|e| {
                    tracing::error!(key, error = ?e, "failed to upsert setting");
                    rootcause::report!(ReconcileError)
                })?;
            Ok(cli_val)
        }
        // Case 5: No DB value, no CLI
        (None, None) => {
            tracing::info!(key, value = %default_value, "seeding DB setting from default");
            upsert_setting(db, key, to_json(&default_value))
                .await
                .map_err(|e| {
                    tracing::error!(key, error = ?e, "failed to upsert setting");
                    rootcause::report!(ReconcileError)
                })?;
            Ok(default_value)
        }
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};

    use super::*;
    use crate::migration;

    async fn setup_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let conn = Database::connect(opt).await.expect("test db");
        migration::run_migrations(&conn).await.expect("migrate");
        conn
    }

    fn string_to_json(v: &String) -> serde_json::Value {
        serde_json::json!(v)
    }

    fn string_from_json(v: &serde_json::Value) -> Option<String> {
        v.as_str().map(String::from)
    }

    #[tokio::test]
    async fn no_db_no_cli_uses_default() {
        let db = setup_db().await;
        let result = reconcile_setting(
            &db,
            "test.no_db_no_cli",
            None,
            "default_val".to_string(),
            false,
            string_to_json,
            string_from_json,
        )
        .await
        .unwrap();
        assert_eq!(result, "default_val");

        // Verify it was saved to DB
        let saved = load_setting(&db, "test.no_db_no_cli").await.unwrap();
        assert_eq!(saved.unwrap().as_str(), Some("default_val"));
    }

    #[tokio::test]
    async fn no_db_cli_provided_uses_cli() {
        let db = setup_db().await;
        let result = reconcile_setting(
            &db,
            "test.no_db_cli",
            Some("cli_val".to_string()),
            "default_val".to_string(),
            false,
            string_to_json,
            string_from_json,
        )
        .await
        .unwrap();
        assert_eq!(result, "cli_val");

        let saved = load_setting(&db, "test.no_db_cli").await.unwrap();
        assert_eq!(saved.unwrap().as_str(), Some("cli_val"));
    }

    #[tokio::test]
    async fn db_exists_no_cli_uses_db() {
        let db = setup_db().await;
        upsert_setting(&db, "test.db_exists", serde_json::json!("db_val"))
            .await
            .unwrap();

        let result = reconcile_setting(
            &db,
            "test.db_exists",
            None,
            "default_val".to_string(),
            false,
            string_to_json,
            string_from_json,
        )
        .await
        .unwrap();
        assert_eq!(result, "db_val");
    }

    #[tokio::test]
    async fn db_exists_cli_differs_no_force_uses_db() {
        let db = setup_db().await;
        upsert_setting(&db, "test.no_force", serde_json::json!("db_val"))
            .await
            .unwrap();

        let result = reconcile_setting(
            &db,
            "test.no_force",
            Some("cli_val".to_string()),
            "default_val".to_string(),
            false,
            string_to_json,
            string_from_json,
        )
        .await
        .unwrap();
        assert_eq!(result, "db_val");
    }

    #[tokio::test]
    async fn db_exists_cli_differs_force_uses_cli() {
        let db = setup_db().await;
        upsert_setting(&db, "test.force", serde_json::json!("db_val"))
            .await
            .unwrap();

        let result = reconcile_setting(
            &db,
            "test.force",
            Some("cli_val".to_string()),
            "default_val".to_string(),
            true,
            string_to_json,
            string_from_json,
        )
        .await
        .unwrap();
        assert_eq!(result, "cli_val");

        // Verify DB was updated
        let saved = load_setting(&db, "test.force").await.unwrap();
        assert_eq!(saved.unwrap().as_str(), Some("cli_val"));
    }
}
