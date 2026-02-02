use std::fmt;

use rootcause::Report;
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use uptrakit_web_api::SettingKey;
use uptrakit_web_api::settings_store::{RawSettings, upsert_setting};

/// Error type used for reconciliation failures.
#[derive(Debug, thiserror::Error)]
#[error("settings reconciliation failed")]
pub struct ReconcileError;

/// JSON conversion pair used by [`reconcile_setting`].
pub struct JsonConvert<T> {
    pub to_json: fn(&T) -> serde_json::Value,
    pub from_json: fn(&serde_json::Value) -> Option<T>,
}

/// Reconcile a single DB-managed setting with an optional CLI value.
///
/// `raw` is the pre-fetched settings map from the bulk `load_all_settings()`
/// call. The function looks up `key` in the map instead of issuing a DB query.
/// The DB connection is still needed for upserts (cases 1, 4, 5).
///
/// The five cases:
/// 1. DB has value + CLI provided + differs + `force` → use CLI, update DB
/// 2. DB has value + CLI provided + differs + no force → use DB, log warning
/// 3. DB has value + (CLI absent OR same) → use DB
/// 4. No DB value + CLI provided → use CLI, save to DB
/// 5. No DB value + CLI absent → use default, save to DB
#[allow(clippy::too_many_arguments)]
pub async fn reconcile_setting<T>(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    key: SettingKey,
    raw: &RawSettings,
    cli_value: Option<T>,
    default_value: T,
    force: bool,
    convert: JsonConvert<T>,
) -> Result<T, Report<ReconcileError>>
where
    T: PartialEq + Clone + fmt::Display,
{
    let db_key = key.as_str();
    let db_value = raw.get(db_key).and_then(convert.from_json);

    match (db_value, cli_value) {
        // Case 1 & 2: DB has a value and CLI differs
        (Some(db_val), Some(cli_val)) if db_val != cli_val => {
            if force {
                // Case 1: force override — use CLI, update DB
                tracing::info!(key = db_key, cli = %cli_val, db = %db_val, "force-overriding DB setting with CLI value");
                upsert_setting(db, tenant_id, key, (convert.to_json)(&cli_val))
                    .await
                    .map_err(|e| {
                        tracing::error!(key = db_key, error = ?e, "failed to upsert setting");
                        rootcause::report!(ReconcileError)
                    })?;
                Ok(cli_val)
            } else {
                // Case 2: no force — use DB, warn
                tracing::warn!(
                    key = db_key,
                    cli = %cli_val,
                    db = %db_val,
                    "CLI value differs from DB; using DB value (pass --force-settings-override to overwrite)"
                );
                Ok(db_val)
            }
        }
        // Case 3: DB has value, CLI either absent or same
        (Some(db_val), _) => {
            tracing::debug!(key = db_key, value = %db_val, "using DB value");
            Ok(db_val)
        }
        // Case 4: No DB value, CLI provided
        (None, Some(cli_val)) => {
            tracing::info!(key = db_key, value = %cli_val, "seeding DB setting from CLI");
            upsert_setting(db, tenant_id, key, (convert.to_json)(&cli_val))
                .await
                .map_err(|e| {
                    tracing::error!(key = db_key, error = ?e, "failed to upsert setting");
                    rootcause::report!(ReconcileError)
                })?;
            Ok(cli_val)
        }
        // Case 5: No DB value, no CLI
        (None, None) => {
            tracing::info!(key = db_key, value = %default_value, "seeding DB setting from default");
            upsert_setting(db, tenant_id, key, (convert.to_json)(&default_value))
                .await
                .map_err(|e| {
                    tracing::error!(key = db_key, error = ?e, "failed to upsert setting");
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
    use uptrakit_web_api::SettingKey;
    use uptrakit_web_api::settings_store::{load_setting, upsert_setting};

    async fn setup_db() -> (DatabaseConnection, Uuid) {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let conn = Database::connect(opt).await.expect("test db");
        migration::run_migrations(&conn).await.expect("migrate");

        // Load the default tenant id from the seeded data
        use sea_orm::EntityTrait;
        use sea_orm::{ColumnTrait, QueryFilter};
        use uptrakit_shared_db::entity::prelude::Tenant;
        use uptrakit_shared_db::entity::tenant;

        let tenant = Tenant::find()
            .filter(tenant::Column::IsDefault.eq(true))
            .one(&conn)
            .await
            .expect("query default tenant")
            .expect("default tenant should exist");

        (conn, tenant.id)
    }

    fn string_convert() -> JsonConvert<String> {
        JsonConvert {
            to_json: |v| serde_json::json!(v),
            from_json: |v| v.as_str().map(String::from),
        }
    }

    #[tokio::test]
    async fn no_db_no_cli_uses_default() {
        let (db, tenant_id) = setup_db().await;
        let raw = HashMap::new();
        let result = reconcile_setting(
            &db,
            tenant_id,
            SettingKey::TrustedProxies,
            &raw,
            None,
            "default_val".to_string(),
            false,
            string_convert(),
        )
        .await
        .unwrap();
        assert_eq!(result, "default_val");

        // Verify it was saved to DB
        let saved = load_setting(&db, tenant_id, SettingKey::TrustedProxies)
            .await
            .unwrap();
        assert_eq!(saved.unwrap().as_str(), Some("default_val"));
    }

    #[tokio::test]
    async fn no_db_cli_provided_uses_cli() {
        let (db, tenant_id) = setup_db().await;
        let raw = HashMap::new();
        let result = reconcile_setting(
            &db,
            tenant_id,
            SettingKey::RealIpHeader,
            &raw,
            Some("cli_val".to_string()),
            "default_val".to_string(),
            false,
            string_convert(),
        )
        .await
        .unwrap();
        assert_eq!(result, "cli_val");

        let saved = load_setting(&db, tenant_id, SettingKey::RealIpHeader)
            .await
            .unwrap();
        assert_eq!(saved.unwrap().as_str(), Some("cli_val"));
    }

    #[tokio::test]
    async fn db_exists_no_cli_uses_db() {
        let (db, tenant_id) = setup_db().await;
        upsert_setting(
            &db,
            tenant_id,
            SettingKey::MqttHost,
            serde_json::json!("db_val"),
        )
        .await
        .unwrap();

        let raw = HashMap::from([(
            SettingKey::MqttHost.as_str().to_string(),
            serde_json::json!("db_val"),
        )]);
        let result = reconcile_setting(
            &db,
            tenant_id,
            SettingKey::MqttHost,
            &raw,
            None,
            "default_val".to_string(),
            false,
            string_convert(),
        )
        .await
        .unwrap();
        assert_eq!(result, "db_val");
    }

    #[tokio::test]
    async fn db_exists_cli_differs_no_force_uses_db() {
        let (db, tenant_id) = setup_db().await;
        upsert_setting(
            &db,
            tenant_id,
            SettingKey::MqttClientId,
            serde_json::json!("db_val"),
        )
        .await
        .unwrap();

        let raw = HashMap::from([(
            SettingKey::MqttClientId.as_str().to_string(),
            serde_json::json!("db_val"),
        )]);
        let result = reconcile_setting(
            &db,
            tenant_id,
            SettingKey::MqttClientId,
            &raw,
            Some("cli_val".to_string()),
            "default_val".to_string(),
            false,
            string_convert(),
        )
        .await
        .unwrap();
        assert_eq!(result, "db_val");
    }

    #[tokio::test]
    async fn db_exists_cli_differs_force_uses_cli() {
        let (db, tenant_id) = setup_db().await;
        upsert_setting(
            &db,
            tenant_id,
            SettingKey::MqttTopicPrefix,
            serde_json::json!("db_val"),
        )
        .await
        .unwrap();

        let raw = HashMap::from([(
            SettingKey::MqttTopicPrefix.as_str().to_string(),
            serde_json::json!("db_val"),
        )]);
        let result = reconcile_setting(
            &db,
            tenant_id,
            SettingKey::MqttTopicPrefix,
            &raw,
            Some("cli_val".to_string()),
            "default_val".to_string(),
            true,
            string_convert(),
        )
        .await
        .unwrap();
        assert_eq!(result, "cli_val");

        // Verify DB was updated
        let saved = load_setting(&db, tenant_id, SettingKey::MqttTopicPrefix)
            .await
            .unwrap();
        assert_eq!(saved.unwrap().as_str(), Some("cli_val"));
    }
}
