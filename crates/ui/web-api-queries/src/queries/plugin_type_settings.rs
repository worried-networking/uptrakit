//! Database query helpers for per-tenant plugin type settings.
//!
//! Plugin type settings allow tenants to store default configuration for an
//! entire plugin type (e.g. global Docker registry credentials) separately
//! from individual plugin configs.

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::plugin_type_setting;
use uptrakit_shared_db::is_unique_constraint_violation;
use uptrakit_shared_macros::impl_report_conversion;
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
