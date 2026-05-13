//! Database query helpers for per-tenant plugin type settings.
//!
//! Plugin type settings allow tenants to store default configuration for an
//! entire plugin type (e.g. global Docker registry credentials) separately
//! from individual plugin configs.

use std::collections::HashSet;

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::{PluginOps, SoftwareItemLifecycleContext};
use uptrakit_shared_db::entity::plugin_type_setting;
use uptrakit_shared_db::is_unique_constraint_violation;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_shared_types::PluginTypeId;
use uuid::Uuid;

use crate::token_utils::generate_uuid;

/// Error returned by plugin type settings query helpers.
#[derive(Debug, thiserror::Error)]
pub enum PluginTypeSettingsError {
    /// A database error occurred.
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
    /// The requested plugin type setting was not found.
    #[error("not found")]
    NotFound,
}

pub type Result<T> = std::result::Result<T, rootcause::Report<PluginTypeSettingsError>>;
impl_report_conversion!(sea_orm::DbErr => PluginTypeSettingsError::Db);

/// List all plugin type settings for a tenant.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn list_type_settings(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<plugin_type_setting::Model>> {
    plugin_type_setting::Entity::find()
        .filter(plugin_type_setting::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .context_to()
}

/// Get a single plugin type setting by tenant and plugin type.
///
/// Returns `None` if no setting exists for the given combination.
#[tracing::instrument(skip_all, fields(%tenant_id, %plugin_type))]
pub async fn get_type_settings(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    plugin_type: &str,
) -> Result<Option<plugin_type_setting::Model>> {
    plugin_type_setting::Entity::find()
        .filter(plugin_type_setting::Column::TenantId.eq(tenant_id))
        .filter(plugin_type_setting::Column::PluginType.eq(plugin_type))
        .one(db)
        .await
        .context_to()
}

/// Load tenant type settings for software-item lifecycle plugins.
///
/// This preloads settings once per dispatch and forwards the resulting
/// [`SoftwareItemLifecycleContext`] to all lifecycle plugins.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn preload_lifecycle_type_settings(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    plugin_ops: &dyn PluginOps,
) -> Result<SoftwareItemLifecycleContext> {
    let lifecycle_plugin_types: HashSet<PluginTypeId> = plugin_ops
        .software_item_lifecycle_plugins()
        .iter()
        .map(|plugin| plugin.plugin_type_id())
        .collect();

    if lifecycle_plugin_types.is_empty() {
        return Ok(SoftwareItemLifecycleContext::default());
    }

    let settings = list_type_settings(db, tenant_id).await?;

    let mut ctx = SoftwareItemLifecycleContext::default();
    for setting in settings {
        let plugin_type = PluginTypeId::new(setting.plugin_type);
        if lifecycle_plugin_types.contains(&plugin_type) {
            ctx.insert_type_setting(plugin_type, setting.config);
        }
    }

    Ok(ctx)
}

/// Create or update a plugin type setting.
///
/// If a row for `(tenant_id, plugin_type)` already exists, its `config` and
/// `updated_at` fields are updated. Otherwise a new row is inserted.
/// Handles unique constraint violations from concurrent inserts by falling
/// back to an update of the existing row.
#[tracing::instrument(skip_all, fields(%tenant_id, %plugin_type))]
pub async fn upsert_type_settings(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    plugin_type: &str,
    config: serde_json::Value,
) -> Result<plugin_type_setting::Model> {
    let now = OffsetDateTime::now_utc();

    // Try to find an existing row first.
    let existing = get_type_settings(db, tenant_id, plugin_type).await?;

    if let Some(existing) = existing {
        let mut model: plugin_type_setting::ActiveModel = existing.into();
        model.config = Set(config);
        model.updated_at = Set(now);
        let updated = model.update(db).await.context_to()?;
        return Ok(updated);
    }

    // No existing row — insert a new one.
    let model = plugin_type_setting::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant_id),
        plugin_type: Set(plugin_type.to_string()),
        config: Set(config.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    };

    match model.insert(db).await {
        Ok(inserted) => Ok(inserted),
        Err(e) if is_unique_constraint_violation(&e) => {
            // A concurrent insert won the race. Fetch the winner and update it.
            let existing = get_type_settings(db, tenant_id, plugin_type)
                .await?
                .ok_or_else(|| report!(PluginTypeSettingsError::NotFound))?;
            let mut active: plugin_type_setting::ActiveModel = existing.into();
            active.config = Set(config);
            active.updated_at = Set(now);
            let updated = active.update(db).await.context_to()?;
            Ok(updated)
        }
        Err(e) => Err(report!(PluginTypeSettingsError::Db(e))),
    }
}

/// Delete a plugin type setting, resetting the tenant to defaults for that
/// plugin type.
///
/// Returns `true` if a row was deleted, `false` if no matching row existed.
#[tracing::instrument(skip_all, fields(%tenant_id, %plugin_type))]
pub async fn delete_type_settings(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    plugin_type: &str,
) -> Result<bool> {
    let result = plugin_type_setting::Entity::delete_many()
        .filter(plugin_type_setting::Column::TenantId.eq(tenant_id))
        .filter(plugin_type_setting::Column::PluginType.eq(plugin_type))
        .exec(db)
        .await
        .context_to()?;

    Ok(result.rows_affected > 0)
}

// ── Audit snapshot ─────────────────────────────────────────────────────────────

/// Audit snapshot for a `plugin_type_setting` entity.
///
/// `config` is excluded: it may contain plugin-specific secrets. The entity's
/// UUID `id` is omitted in favour of `plugin_type` as the semantic target
/// identifier.
pub struct PluginTypeSettingsView {
    pub plugin_type: String,
}

impl uptrakit_audit_log::AuditView for PluginTypeSettingsView {
    const TARGET_TYPE: &'static str = "plugin_type_settings";

    fn audit_target_id(&self) -> String {
        self.plugin_type.clone()
    }

    fn audit_target_display(&self) -> Option<String> {
        Some(self.plugin_type.clone())
    }

    fn audit_view(&self) -> serde_json::Value {
        serde_json::json!({ "plugin_type": self.plugin_type })
    }
}

impl From<&plugin_type_setting::Model> for PluginTypeSettingsView {
    fn from(m: &plugin_type_setting::Model) -> Self {
        Self {
            plugin_type: m.plugin_type.clone(),
        }
    }
}

// ── Connection-generic helper ──────────────────────────────────────────────────

async fn get_type_settings_conn(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    plugin_type: &str,
) -> Result<Option<plugin_type_setting::Model>> {
    plugin_type_setting::Entity::find()
        .filter(plugin_type_setting::Column::TenantId.eq(tenant_id))
        .filter(plugin_type_setting::Column::PluginType.eq(plugin_type))
        .one(db)
        .await
        .context_to()
}

// ── Transaction-aware helpers ──────────────────────────────────────────────────

/// Upsert plugin type settings inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Returns `(None, after)` for a create (no prior settings exist), or
/// `(Some(before), after)` for an update. All validation must be done by the
/// caller before opening the transaction.
pub async fn upsert_type_settings_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    tenant_id: Uuid,
    plugin_type: &str,
    config: serde_json::Value,
) -> Result<(
    Option<plugin_type_setting::Model>,
    plugin_type_setting::Model,
)> {
    let now = OffsetDateTime::now_utc();
    let existing = get_type_settings_conn(tx, tenant_id, plugin_type).await?;

    if let Some(before) = existing {
        let mut active: plugin_type_setting::ActiveModel = before.clone().into();
        active.config = Set(config);
        active.updated_at = Set(now);
        let after = active.update(tx).await.context_to()?;
        return Ok((Some(before), after));
    }

    let model = plugin_type_setting::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant_id),
        plugin_type: Set(plugin_type.to_string()),
        config: Set(config.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    };

    match model.insert(tx).await {
        Ok(inserted) => Ok((None, inserted)),
        Err(e) if is_unique_constraint_violation(&e) => {
            // A concurrent insert won the race. Fetch the winner and update it.
            let before = get_type_settings_conn(tx, tenant_id, plugin_type)
                .await?
                .ok_or_else(|| report!(PluginTypeSettingsError::NotFound))?;
            let mut active: plugin_type_setting::ActiveModel = before.clone().into();
            active.config = Set(config);
            active.updated_at = Set(now);
            let after = active.update(tx).await.context_to()?;
            Ok((Some(before), after))
        }
        Err(e) => Err(report!(PluginTypeSettingsError::Db(e))),
    }
}

/// Delete plugin type settings inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Returns `Some(before)` if a row was deleted, `None` if no settings exist for
/// the given `(tenant_id, plugin_type)` pair.
pub async fn delete_type_settings_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    tenant_id: Uuid,
    plugin_type: &str,
) -> Result<Option<plugin_type_setting::Model>> {
    let Some(before) = get_type_settings_conn(tx, tenant_id, plugin_type).await? else {
        return Ok(None);
    };

    plugin_type_setting::Entity::delete_by_id(before.id)
        .exec(tx)
        .await
        .context_to()?;

    Ok(Some(before))
}
