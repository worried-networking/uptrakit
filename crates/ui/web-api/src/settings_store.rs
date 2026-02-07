use std::collections::HashMap;

use crate::SettingKey;
use crate::auth::Result;
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter, Set,
    sea_query::Expr,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{prelude::*, setting, settings_version};
use uuid::Uuid;

/// All settings from the DB, keyed by setting name.
pub type RawSettings = HashMap<String, serde_json::Value>;

/// Extension trait for typed lookups on [`RawSettings`].
pub trait RawSettingsExt {
    /// Look up a setting by its typed key.
    fn get_setting(&self, key: SettingKey) -> Option<&serde_json::Value>;
}

impl RawSettingsExt for RawSettings {
    fn get_setting(&self, key: SettingKey) -> Option<&serde_json::Value> {
        self.get(key.as_str())
    }
}

/// Resolve which tenant_id to use for a given setting key.
///
/// Global settings are always stored under the default tenant.
pub fn resolve_tenant_for_key(key: SettingKey, tenant_id: Uuid, default_tenant_id: Uuid) -> Uuid {
    if key.is_global() {
        default_tenant_id
    } else {
        tenant_id
    }
}

/// Load every row from the `settings` table for a given tenant in a single query.
pub async fn load_all_settings(db: &DatabaseConnection, tenant_id: Uuid) -> Result<RawSettings> {
    let rows = Setting::find()
        .filter(setting::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .context_to()?;
    Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
}

pub async fn upsert_setting(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    key: SettingKey,
    value: serde_json::Value,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let db_key = key.as_str();
    let existing = Setting::find_by_id((tenant_id, db_key.to_string()))
        .one(db)
        .await
        .context_to()?;

    if let Some(existing) = existing {
        let mut model: setting::ActiveModel = existing.into();
        model.value = Set(value);
        model.updated_at = Set(now);
        model.update(db).await.context_to()?;
    } else {
        let model = setting::ActiveModel {
            tenant_id: Set(tenant_id),
            key: Set(db_key.to_string()),
            value: Set(value),
            updated_at: Set(now),
        };
        model.insert(db).await.context_to()?;
    }

    // Bump the version counter (non-fatal on failure)
    if let Err(e) = bump_settings_version(db, tenant_id, key.is_global()).await {
        tracing::warn!(error = ?e, key = db_key, "failed to bump settings version counter");
    }

    Ok(())
}

pub async fn load_setting(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    key: SettingKey,
) -> Result<Option<serde_json::Value>> {
    let setting = Setting::find_by_id((tenant_id, key.as_str().to_string()))
        .one(db)
        .await
        .context_to()?;
    Ok(setting.map(|s| s.value))
}

pub async fn delete_setting(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    key: SettingKey,
) -> Result<()> {
    Setting::delete_many()
        .filter(setting::Column::TenantId.eq(tenant_id))
        .filter(setting::Column::Key.eq(key.as_str()))
        .exec(db)
        .await
        .context_to()?;

    // Bump the version counter (non-fatal on failure)
    if let Err(e) = bump_settings_version(db, tenant_id, key.is_global()).await {
        tracing::warn!(
            error = ?e,
            key = key.as_str(),
            "failed to bump settings version counter"
        );
    }

    Ok(())
}

/// Bump the settings version counter after a settings write.
///
/// If `is_global` is true, increments `global_version` on ALL tenant rows.
/// If false, increments `version` on the specific tenant's row only.
/// Non-fatal on failure: callers should log and continue.
pub async fn bump_settings_version(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    is_global: bool,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();

    if is_global {
        // Increment global_version on ALL rows
        SettingsVersion::update_many()
            .col_expr(
                settings_version::Column::GlobalVersion,
                Expr::col(settings_version::Column::GlobalVersion).add(1),
            )
            .col_expr(settings_version::Column::UpdatedAt, Expr::value(now))
            .exec(db)
            .await
            .context_to()?;
    } else {
        // Increment version on just this tenant's row
        let result = SettingsVersion::update_many()
            .col_expr(
                settings_version::Column::Version,
                Expr::col(settings_version::Column::Version).add(1),
            )
            .col_expr(settings_version::Column::UpdatedAt, Expr::value(now))
            .filter(settings_version::Column::TenantId.eq(tenant_id))
            .exec(db)
            .await
            .context_to()?;

        // Defensive: if the row didn't exist (tenant created after migration), insert it
        if result.rows_affected == 0 {
            let model = settings_version::ActiveModel {
                tenant_id: Set(tenant_id),
                version: Set(1),
                global_version: Set(0),
                revocation_version: Set(0),
                updated_at: Set(now),
            };
            model.insert(db).await.context_to()?;
        }
    }

    Ok(())
}

/// Read both version counters for a tenant.
///
/// Returns `(version, global_version)`.
pub async fn get_settings_versions(db: &DatabaseConnection, tenant_id: Uuid) -> Result<(i64, i64)> {
    let row = SettingsVersion::find_by_id(tenant_id)
        .one(db)
        .await
        .context_to()?;

    match row {
        Some(model) => Ok((model.version, model.global_version)),
        // No row yet — treat as (0, 0)
        None => Ok((0, 0)),
    }
}

/// Atomically bump the revocation version counter after a certificate revocation.
///
/// Non-fatal on failure: callers should log and continue.
pub async fn bump_revocation_version(db: &DatabaseConnection, tenant_id: Uuid) -> Result<()> {
    let now = OffsetDateTime::now_utc();

    let result = SettingsVersion::update_many()
        .col_expr(
            settings_version::Column::RevocationVersion,
            Expr::col(settings_version::Column::RevocationVersion).add(1),
        )
        .col_expr(settings_version::Column::UpdatedAt, Expr::value(now))
        .filter(settings_version::Column::TenantId.eq(tenant_id))
        .exec(db)
        .await
        .context_to()?;

    // Defensive: if the row didn't exist (tenant created after migration), insert it
    if result.rows_affected == 0 {
        let model = settings_version::ActiveModel {
            tenant_id: Set(tenant_id),
            version: Set(0),
            global_version: Set(0),
            revocation_version: Set(1),
            updated_at: Set(now),
        };
        model.insert(db).await.context_to()?;
    }

    Ok(())
}

/// Read the revocation version counter for a tenant.
///
/// Returns `0` if no row exists yet.
pub async fn get_revocation_version(db: &DatabaseConnection, tenant_id: Uuid) -> Result<i64> {
    let row = SettingsVersion::find_by_id(tenant_id)
        .one(db)
        .await
        .context_to()?;

    match row {
        Some(model) => Ok(model.revocation_version),
        None => Ok(0),
    }
}
