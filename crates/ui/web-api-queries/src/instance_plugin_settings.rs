//! Queries for `instance_plugin_setting` — Instance-Scoped Plugin enable
//! state and instance-wide configuration. See spec
//! `docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md`.

use std::collections::HashMap;

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Set, SqliteTransactionMode,
    TransactionOptions, TransactionTrait,
};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::instance_plugin_setting::{ActiveModel, Entity, Model};
use uptrakit_shared_macros::impl_report_conversion;

/// Error returned by instance plugin settings query helpers.
#[derive(Debug, Error)]
pub enum InstancePluginSettingsError {
    /// A database error occurred.
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<InstancePluginSettingsError>>;
impl_report_conversion!(sea_orm::DbErr => InstancePluginSettingsError::Db);

/// Snapshot of every row in `instance_plugin_setting`, loaded once at
/// controller boot and shared via `Arc<ArcSwap<...>>` in `AppState`.
#[derive(Default, Debug, Clone)]
pub struct InstancePluginSnapshot {
    rows: HashMap<String, InstancePluginRow>,
}

/// A single row from `instance_plugin_setting`, held in memory.
#[derive(Debug, Clone)]
pub struct InstancePluginRow {
    /// Whether this plugin is enabled instance-wide.
    pub enabled: bool,
    /// Instance-wide configuration JSON for this plugin.
    pub config: serde_json::Value,
    /// Timestamp of the last modification.
    pub updated_at: OffsetDateTime,
}

impl InstancePluginSnapshot {
    /// Construct an empty snapshot (no plugins enabled).
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns `true` if `plugin_type_id` is enabled; `false` if absent or disabled.
    #[must_use]
    pub fn enabled(&self, plugin_type_id: &str) -> bool {
        self.rows
            .get(plugin_type_id)
            .map(|r| r.enabled)
            .unwrap_or(false)
    }

    /// Returns the row for `plugin_type_id`, or `None` if absent.
    pub fn get(&self, plugin_type_id: &str) -> Option<&InstancePluginRow> {
        self.rows.get(plugin_type_id)
    }

    /// Iterate over all rows in the snapshot.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &InstancePluginRow)> {
        self.rows.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Returns a NEW snapshot with the given row inserted/replaced. The
    /// original is untouched (callers must `arc_swap.store(Arc::new(new))`
    /// to publish). Designed for the `Arc<ArcSwap<InstancePluginSnapshot>>`
    /// pattern in `AppState` where in-place mutation would silently mutate a
    /// discarded clone.
    #[must_use]
    pub fn with_upserted(&self, plugin_type_id: String, row: InstancePluginRow) -> Self {
        let mut new = self.clone();
        new.rows.insert(plugin_type_id, row);
        new
    }
}

/// Load every row from `instance_plugin_setting` in a single query.
///
/// Called once at controller boot; the result is stored in `AppState`.
#[tracing::instrument(skip_all)]
pub async fn load_at_boot(db: &DatabaseConnection) -> Result<InstancePluginSnapshot> {
    let rows = Entity::find().all(db).await.context_to()?;
    Ok(InstancePluginSnapshot {
        rows: rows
            .into_iter()
            .map(|m| {
                (
                    m.plugin_type_id,
                    InstancePluginRow {
                        enabled: m.enabled,
                        config: m.config,
                        updated_at: m.updated_at,
                    },
                )
            })
            .collect(),
    })
}

/// Toggle the `enabled` flag for a plugin. Uses `BEGIN IMMEDIATE` per project
/// SQLite rule (read-then-write in same transaction).
///
/// Returns `(previous_enabled, updated_model)` where `previous_enabled` is
/// `None` if no row existed before this call.
#[tracing::instrument(skip_all, fields(%plugin_type_id, %new_enabled))]
pub async fn set_enabled(
    db: &DatabaseConnection,
    plugin_type_id: &str,
    new_enabled: bool,
) -> Result<(Option<bool>, Model)> {
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .context_to()?;

    let existing = Entity::find_by_id(plugin_type_id.to_string())
        .one(&txn)
        .await
        .context_to()?;
    let previous_enabled = existing.as_ref().map(|m| m.enabled);

    let now = OffsetDateTime::now_utc();
    let model = match existing {
        Some(m) => {
            let mut active: ActiveModel = m.into();
            active.enabled = Set(new_enabled);
            active.updated_at = Set(now);
            active.update(&txn).await.context_to()?
        }
        None => {
            let active = ActiveModel {
                plugin_type_id: Set(plugin_type_id.to_string()),
                enabled: Set(new_enabled),
                config: Set(serde_json::json!({})),
                updated_at: Set(now),
            };
            active.insert(&txn).await.context_to()?
        }
    };

    txn.commit().await.context_to()?;

    Ok((previous_enabled, model))
}

/// Upsert the `config` for a plugin, preserving the existing `enabled` value.
/// Uses `BEGIN IMMEDIATE` per project SQLite rule (read-then-write).
///
/// If no row exists, one is inserted with `enabled = false`.
#[tracing::instrument(skip_all, fields(%plugin_type_id))]
pub async fn upsert_config(
    db: &DatabaseConnection,
    plugin_type_id: &str,
    new_config: serde_json::Value,
) -> Result<Model> {
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .context_to()?;

    let existing = Entity::find_by_id(plugin_type_id.to_string())
        .one(&txn)
        .await
        .context_to()?;

    let now = OffsetDateTime::now_utc();
    let model = match existing {
        Some(m) => {
            let mut active: ActiveModel = m.into();
            active.config = Set(new_config);
            active.updated_at = Set(now);
            active.update(&txn).await.context_to()?
        }
        None => {
            let active = ActiveModel {
                plugin_type_id: Set(plugin_type_id.to_string()),
                enabled: Set(false),
                config: Set(new_config),
                updated_at: Set(now),
            };
            active.insert(&txn).await.context_to()?
        }
    };

    txn.commit().await.context_to()?;

    Ok(model)
}

// ── Audit snapshot ─────────────────────────────────────────────────────────────

/// Audit snapshot for an `instance_plugin_setting` entity.
///
/// `config` is excluded — it may contain plugin-specific secrets.
pub struct InstancePluginSettingView {
    /// The plugin type identifier (primary key).
    pub plugin_type_id: String,
    /// Whether the plugin is currently enabled.
    pub enabled: bool,
}

impl uptrakit_audit_log::AuditView for InstancePluginSettingView {
    const TARGET_TYPE: &'static str = "instance_plugin";

    fn audit_target_id(&self) -> String {
        self.plugin_type_id.clone()
    }

    fn audit_target_display(&self) -> Option<String> {
        Some(self.plugin_type_id.clone())
    }

    fn audit_view(&self) -> serde_json::Value {
        serde_json::json!({
            "plugin_type_id": self.plugin_type_id,
            "enabled": self.enabled,
        })
    }
}

impl From<&Model> for InstancePluginSettingView {
    fn from(m: &Model) -> Self {
        Self {
            plugin_type_id: m.plugin_type_id.clone(),
            enabled: m.enabled,
        }
    }
}

// ── Transaction-aware helpers ──────────────────────────────────────────────────

async fn find_by_id_conn(db: &impl ConnectionTrait, plugin_type_id: &str) -> Result<Option<Model>> {
    Entity::find_by_id(plugin_type_id.to_string())
        .one(db)
        .await
        .context_to()
}

/// Toggle the `enabled` flag for a plugin inside a caller-managed transaction.
///
/// Returns `(None, after)` when no row existed prior to this call (INSERT), or
/// `(Some(before), after)` when a row existed (UPDATE). The caller is responsible
/// for opening a `BEGIN IMMEDIATE` transaction before calling this function.
#[tracing::instrument(skip_all, fields(%plugin_type_id, %new_enabled))]
pub async fn set_enabled_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    plugin_type_id: &str,
    new_enabled: bool,
) -> Result<(Option<Model>, Model)> {
    let existing = find_by_id_conn(tx, plugin_type_id).await?;
    let now = OffsetDateTime::now_utc();
    match existing {
        Some(before) => {
            let mut active: ActiveModel = before.clone().into();
            active.enabled = Set(new_enabled);
            active.updated_at = Set(now);
            let after = active.update(tx).await.context_to()?;
            Ok((Some(before), after))
        }
        None => {
            let active = ActiveModel {
                plugin_type_id: Set(plugin_type_id.to_string()),
                enabled: Set(new_enabled),
                config: Set(serde_json::json!({})),
                updated_at: Set(now),
            };
            let after = active.insert(tx).await.context_to()?;
            Ok((None, after))
        }
    }
}

/// Upsert the `config` for a plugin inside a caller-managed transaction,
/// preserving the existing `enabled` value.
///
/// Returns `(None, after)` when no row existed (INSERT), or
/// `(Some(before), after)` when a row existed (UPDATE). The caller is responsible
/// for opening a `BEGIN IMMEDIATE` transaction before calling this function.
///
/// If no row exists, one is inserted with `enabled = false`.
#[tracing::instrument(skip_all, fields(%plugin_type_id))]
pub async fn upsert_config_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    plugin_type_id: &str,
    new_config: serde_json::Value,
) -> Result<(Option<Model>, Model)> {
    let existing = find_by_id_conn(tx, plugin_type_id).await?;
    let now = OffsetDateTime::now_utc();
    match existing {
        Some(before) => {
            let mut active: ActiveModel = before.clone().into();
            active.config = Set(new_config);
            active.updated_at = Set(now);
            let after = active.update(tx).await.context_to()?;
            Ok((Some(before), after))
        }
        None => {
            let active = ActiveModel {
                plugin_type_id: Set(plugin_type_id.to_string()),
                enabled: Set(false),
                config: Set(new_config),
                updated_at: Set(now),
            };
            let after = active.insert(tx).await.context_to()?;
            Ok((None, after))
        }
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::Database;

    use super::*;

    async fn setup_db() -> DatabaseConnection {
        uptrakit_crypto::enable_plaintext_mode();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    #[tokio::test]
    async fn load_at_boot_returns_empty_when_no_rows() {
        let db = setup_db().await;
        let snapshot = load_at_boot(&db).await.unwrap();
        assert!(!snapshot.enabled("some.plugin"));
        assert!(snapshot.get("some.plugin").is_none());
        assert_eq!(snapshot.iter().count(), 0);
    }

    #[tokio::test]
    async fn set_enabled_inserts_new_row_previous_is_none() {
        let db = setup_db().await;
        let (prev, model) = set_enabled(&db, "test.plugin", true).await.unwrap();
        assert!(prev.is_none(), "no prior row should yield None");
        assert!(model.enabled);
        assert_eq!(model.plugin_type_id, "test.plugin");
    }

    #[tokio::test]
    async fn set_enabled_updates_existing_row_and_returns_previous() {
        let db = setup_db().await;

        // First call: insert with enabled = true
        let (prev1, _) = set_enabled(&db, "test.plugin", true).await.unwrap();
        assert!(prev1.is_none());

        // Second call: update to enabled = false
        let (prev2, model) = set_enabled(&db, "test.plugin", false).await.unwrap();
        assert_eq!(prev2, Some(true));
        assert!(!model.enabled);
    }

    #[tokio::test]
    async fn upsert_config_inserts_with_enabled_false_when_no_row() {
        let db = setup_db().await;
        let config = serde_json::json!({"key": "value"});
        let model = upsert_config(&db, "test.plugin", config.clone())
            .await
            .unwrap();
        assert!(!model.enabled, "new row should default to disabled");
        assert_eq!(model.config, config);
    }

    #[tokio::test]
    async fn upsert_config_preserves_enabled_when_row_exists() {
        let db = setup_db().await;

        // Enable the plugin first
        let (_, _) = set_enabled(&db, "test.plugin", true).await.unwrap();

        // Upsert config — enabled should remain true
        let new_config = serde_json::json!({"updated": true});
        let model = upsert_config(&db, "test.plugin", new_config.clone())
            .await
            .unwrap();
        assert!(
            model.enabled,
            "enabled must be preserved after config upsert"
        );
        assert_eq!(model.config, new_config);
    }

    #[tokio::test]
    async fn load_at_boot_reflects_set_enabled() {
        let db = setup_db().await;
        set_enabled(&db, "test.a", true).await.unwrap();
        set_enabled(&db, "test.b", false).await.unwrap();

        let snapshot = load_at_boot(&db).await.unwrap();
        assert!(snapshot.enabled("test.a"));
        assert!(!snapshot.enabled("test.b"));
        assert_eq!(snapshot.iter().count(), 2);
    }

    #[tokio::test]
    async fn snapshot_with_upserted_updates_in_memory() {
        let snapshot = InstancePluginSnapshot::empty();
        assert!(!snapshot.enabled("my.plugin"));

        let row = InstancePluginRow {
            enabled: true,
            config: serde_json::json!({}),
            updated_at: OffsetDateTime::now_utc(),
        };
        let snapshot = snapshot.with_upserted("my.plugin".to_string(), row);
        assert!(snapshot.enabled("my.plugin"));
    }
}
