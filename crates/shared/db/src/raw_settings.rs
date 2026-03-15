//! Raw-key settings helpers for managing plugin-owned settings.
//!
//! These functions accept raw `&str` keys, allowing plugin crates to manage
//! their own settings without adding variants to the `SettingKey` enum in
//! `web-api-auth`. Version counters are bumped as usual so cross-instance
//! invalidation works.
//!
//! By living in `shared-db`, notification plugins can use these functions
//! directly instead of depending on the `web-api-auth` crate, breaking the
//! cross-layer dependency cycle.

use std::collections::HashMap;

use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter, Set,
    sea_query::{Expr, OnConflict},
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::entity::{global_setting, prelude::*, setting, settings_version};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error returned by raw settings operations.
#[derive(Debug, thiserror::Error)]
pub enum RawSettingsError {
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<RawSettingsError>>;

uptrakit_shared_macros::impl_report_conversion!(sea_orm::DbErr => RawSettingsError::Database);

// ---------------------------------------------------------------------------
// Version tracking (internal helpers)
// ---------------------------------------------------------------------------

/// Bump the per-tenant settings version counter after a per-tenant settings write.
///
/// Increments `version` on the specific tenant's `settings_version` row only.
/// Non-fatal on failure: callers should log and continue.
pub async fn bump_settings_version(db: &impl ConnectionTrait, tenant_id: Uuid) -> Result<()> {
    let now = OffsetDateTime::now_utc();

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

    // Defensive: if the row didn't exist (tenant created after migration), insert it.
    // Use on_conflict(do_nothing) to avoid racing with a concurrent insert.
    if result.rows_affected == 0 {
        let model = settings_version::ActiveModel {
            tenant_id: Set(tenant_id),
            version: Set(1),
            global_version: Set(0),
            revocation_version: Set(0),
            updated_at: Set(now),
        };
        SettingsVersion::insert(model)
            .on_conflict(
                OnConflict::column(settings_version::Column::TenantId)
                    .do_nothing()
                    .to_owned(),
            )
            .try_insert()
            .exec(db)
            .await
            .context_to()?;
    }

    Ok(())
}

/// Bump the global settings version counter after a global settings write.
///
/// Increments `global_version` on ALL tenant rows in `settings_version`.
/// Non-fatal on failure: callers should log and continue.
pub async fn bump_global_settings_version(db: &impl ConnectionTrait) -> Result<()> {
    let now = OffsetDateTime::now_utc();

    SettingsVersion::update_many()
        .col_expr(
            settings_version::Column::GlobalVersion,
            Expr::col(settings_version::Column::GlobalVersion).add(1),
        )
        .col_expr(settings_version::Column::UpdatedAt, Expr::value(now))
        .exec(db)
        .await
        .context_to()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Raw-key settings (bypass SettingKey validation)
// ---------------------------------------------------------------------------

/// Insert or update a per-tenant setting using a raw key string.
pub async fn upsert_setting_raw(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    key: &str,
    value: serde_json::Value,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();

    let model = setting::ActiveModel {
        tenant_id: Set(tenant_id),
        key: Set(key.to_string()),
        value: Set(value),
        updated_at: Set(now),
    };

    Setting::insert(model)
        .on_conflict(
            OnConflict::columns([setting::Column::TenantId, setting::Column::Key])
                .update_columns([setting::Column::Value, setting::Column::UpdatedAt])
                .to_owned(),
        )
        .exec(db)
        .await
        .context_to()?;

    if let Err(e) = bump_settings_version(db, tenant_id).await {
        tracing::warn!(error = ?e, key, "failed to bump settings version counter");
    }

    Ok(())
}

/// Insert or update a global setting using a raw key string.
pub async fn upsert_global_setting_raw(
    db: &impl ConnectionTrait,
    key: &str,
    value: serde_json::Value,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();

    let model = global_setting::ActiveModel {
        key: Set(key.to_string()),
        value: Set(value),
        updated_at: Set(now),
    };

    GlobalSetting::insert(model)
        .on_conflict(
            OnConflict::column(global_setting::Column::Key)
                .update_columns([
                    global_setting::Column::Value,
                    global_setting::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await
        .context_to()?;

    if let Err(e) = bump_global_settings_version(db).await {
        tracing::warn!(error = ?e, key, "failed to bump global settings version counter");
    }

    Ok(())
}

/// Load all per-tenant settings whose key starts with `prefix`.
pub async fn load_settings_by_prefix(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    prefix: &str,
) -> Result<HashMap<String, serde_json::Value>> {
    let rows = Setting::find()
        .filter(setting::Column::TenantId.eq(tenant_id))
        .filter(setting::Column::Key.starts_with(prefix))
        .all(db)
        .await
        .context_to()?;
    Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
}

/// Load all global settings whose key starts with `prefix`.
pub async fn load_global_settings_by_prefix(
    db: &DatabaseConnection,
    prefix: &str,
) -> Result<HashMap<String, serde_json::Value>> {
    let rows = GlobalSetting::find()
        .filter(global_setting::Column::Key.starts_with(prefix))
        .all(db)
        .await
        .context_to()?;
    Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
}
