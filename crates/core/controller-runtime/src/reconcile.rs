use std::fmt;

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;

use uptrakit_web_api::settings_store::{RawSettings, upsert_global_setting_raw};

/// Error type used for reconciliation failures.
#[derive(Debug, thiserror::Error)]
#[error("settings reconciliation failed")]
pub(crate) struct ReconcileError;

pub(crate) type Result<T> = std::result::Result<T, Report<ReconcileError>>;

/// JSON conversion pair used by [`reconcile_setting`].
pub(crate) struct JsonConvert<T> {
    pub to_json: fn(&T) -> serde_json::Value,
    pub from_json: fn(&serde_json::Value) -> Option<T>,
}

/// Parameters for [`reconcile_setting`].
///
/// All reconciled settings are global (no `tenant_id`).
///
/// `key` is the raw DB key string (e.g. `"network.trusted_proxies"`).
/// The former `SettingKey` enum variants for file-only settings were removed in
/// the graceful-reload migration (spec §6.3, §20); callers now pass the raw string.
pub(crate) struct ReconcileParams<'a, T> {
    pub db: &'a DatabaseConnection,
    pub key: &'static str,
    pub raw: &'a RawSettings,
    pub cli_value: Option<T>,
    pub default_value: T,
    pub force: bool,
    pub convert: JsonConvert<T>,
}

/// Reconcile a single DB-managed global setting with an optional CLI value.
///
/// `raw` is the pre-fetched settings map from the bulk `load_all_global_settings()`
/// call. The function looks up `key` in the map instead of issuing a DB query.
/// The DB connection is still needed for upserts (cases 1, 4, 5).
///
/// The five cases:
/// 1. DB has value + CLI provided + differs + `force` → use CLI, update DB
/// 2. DB has value + CLI provided + differs + no force → use DB, log warning
/// 3. DB has value + (CLI absent OR same) → use DB
/// 4. No DB value + CLI provided → use CLI, save to DB
/// 5. No DB value + CLI absent → use default, save to DB
pub(crate) async fn reconcile_setting<T>(params: ReconcileParams<'_, T>) -> Result<T>
where
    T: PartialEq + Clone + fmt::Display,
{
    let ReconcileParams {
        db,
        key,
        raw,
        cli_value,
        default_value,
        force,
        convert,
    } = params;
    let db_value = raw.get(key).and_then(convert.from_json);

    match (db_value, cli_value) {
        // Case 1 & 2: DB has a value and CLI differs
        (Some(db_val), Some(cli_val)) if db_val != cli_val => {
            if force {
                // Case 1: force override — use CLI, update DB
                tracing::info!(key, cli = %cli_val, db = %db_val, "force-overriding DB setting with CLI value");
                upsert_global_setting_raw(db, key, (convert.to_json)(&cli_val))
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
            upsert_global_setting_raw(db, key, (convert.to_json)(&cli_val))
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
            upsert_global_setting_raw(db, key, (convert.to_json)(&default_value))
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
    use std::collections::HashMap;

    use sea_orm::{ConnectOptions, Database, DatabaseConnection};

    use super::*;
    use crate::migration;
    use uptrakit_web_api::settings_store::{load_all_global_settings, upsert_global_setting_raw};

    async fn setup_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let conn = Database::connect(opt).await.expect("test db");
        migration::run_migrations(&conn).await.expect("migrate");
        conn
    }

    fn string_convert() -> JsonConvert<String> {
        JsonConvert {
            to_json: |v| serde_json::json!(v),
            from_json: |v| v.as_str().map(String::from),
        }
    }

    #[tokio::test]
    async fn no_db_no_cli_uses_default() {
        let db = setup_db().await;
        let raw = HashMap::new();
        let result = reconcile_setting(ReconcileParams {
            db: &db,
            key: "network.trusted_proxies",
            raw: &raw,
            cli_value: None,
            default_value: "default_val".to_string(),
            force: false,
            convert: string_convert(),
        })
        .await
        .unwrap();
        assert_eq!(result, "default_val");

        // Verify it was saved to DB
        let all = load_all_global_settings(&db).await.unwrap();
        let saved = all.get("network.trusted_proxies");
        assert_eq!(saved.and_then(|v| v.as_str()), Some("default_val"));
    }

    #[tokio::test]
    async fn no_db_cli_provided_uses_cli() {
        let db = setup_db().await;
        let raw = HashMap::new();
        let result = reconcile_setting(ReconcileParams {
            db: &db,
            key: "network.real_ip_header",
            raw: &raw,
            cli_value: Some("cli_val".to_string()),
            default_value: "default_val".to_string(),
            force: false,
            convert: string_convert(),
        })
        .await
        .unwrap();
        assert_eq!(result, "cli_val");

        let all = load_all_global_settings(&db).await.unwrap();
        let saved = all.get("network.real_ip_header");
        assert_eq!(saved.and_then(|v| v.as_str()), Some("cli_val"));
    }

    #[tokio::test]
    async fn db_exists_no_cli_uses_db() {
        let db = setup_db().await;
        upsert_global_setting_raw(&db, "network.real_ip_header", serde_json::json!("db_val"))
            .await
            .unwrap();

        let raw = HashMap::from([(
            "network.real_ip_header".to_string(),
            serde_json::json!("db_val"),
        )]);
        let result = reconcile_setting(ReconcileParams {
            db: &db,
            key: "network.real_ip_header",
            raw: &raw,
            cli_value: None,
            default_value: "default_val".to_string(),
            force: false,
            convert: string_convert(),
        })
        .await
        .unwrap();
        assert_eq!(result, "db_val");
    }

    #[tokio::test]
    async fn db_exists_cli_differs_no_force_uses_db() {
        let db = setup_db().await;
        upsert_global_setting_raw(&db, "network.https_addr", serde_json::json!("db_val"))
            .await
            .unwrap();

        let raw = HashMap::from([(
            "network.https_addr".to_string(),
            serde_json::json!("db_val"),
        )]);
        let result = reconcile_setting(ReconcileParams {
            db: &db,
            key: "network.https_addr",
            raw: &raw,
            cli_value: Some("cli_val".to_string()),
            default_value: "default_val".to_string(),
            force: false,
            convert: string_convert(),
        })
        .await
        .unwrap();
        assert_eq!(result, "db_val");
    }

    #[tokio::test]
    async fn db_exists_cli_differs_force_uses_cli() {
        let db = setup_db().await;
        upsert_global_setting_raw(&db, "network.pki_addr", serde_json::json!("db_val"))
            .await
            .unwrap();

        let raw = HashMap::from([("network.pki_addr".to_string(), serde_json::json!("db_val"))]);
        let result = reconcile_setting(ReconcileParams {
            db: &db,
            key: "network.pki_addr",
            raw: &raw,
            cli_value: Some("cli_val".to_string()),
            default_value: "default_val".to_string(),
            force: true,
            convert: string_convert(),
        })
        .await
        .unwrap();
        assert_eq!(result, "cli_val");

        // Verify DB was updated
        let all = load_all_global_settings(&db).await.unwrap();
        let saved = all.get("network.pki_addr");
        assert_eq!(saved.and_then(|v| v.as_str()), Some("cli_val"));
    }
}
