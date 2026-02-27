use std::collections::HashMap;
use std::path::Path;

use crate::SettingKey;
use crate::auth::{AuthError, Result};
use base64::Engine;
use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter, Set,
    sea_query::{Expr, OnConflict},
};
use time::OffsetDateTime;
use uptrakit_shared_db::crypto::{decrypt_str, encrypt_str, is_encrypted};
use uptrakit_shared_db::entity::{prelude::*, setting, settings_version};
use uuid::Uuid;

const JWT_KEY_LENGTH: usize = 64;

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
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    key: SettingKey,
    value: serde_json::Value,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let db_key = key.as_str();

    let model = setting::ActiveModel {
        tenant_id: Set(tenant_id),
        key: Set(db_key.to_string()),
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

    // Bump the version counter (non-fatal on failure)
    if let Err(e) = bump_settings_version(db, tenant_id, key.is_global()).await {
        tracing::warn!(error = ?e, key = db_key, "failed to bump settings version counter");
    }

    Ok(())
}

/// Insert a setting only if it does not already exist (INSERT without ON CONFLICT UPDATE).
///
/// Returns `true` if the row was inserted, or `false` if a conflicting row
/// already exists (duplicate tenant_id + key). This is used by HA startup
/// to avoid silently overwriting a verification token written by another instance.
pub async fn insert_setting_if_absent(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    key: SettingKey,
    value: serde_json::Value,
) -> Result<bool> {
    let now = OffsetDateTime::now_utc();
    let db_key = key.as_str();

    let model = setting::ActiveModel {
        tenant_id: Set(tenant_id),
        key: Set(db_key.to_string()),
        value: Set(value),
        updated_at: Set(now),
    };

    let result = Setting::insert(model)
        .on_conflict(
            OnConflict::columns([setting::Column::TenantId, setting::Column::Key])
                .do_nothing()
                .to_owned(),
        )
        .try_insert()
        .exec(db)
        .await;

    match result {
        Ok(_) => {
            // Row was inserted — bump version counter (non-fatal on failure)
            if let Err(e) = bump_settings_version(db, tenant_id, key.is_global()).await {
                tracing::warn!(error = ?e, key = db_key, "failed to bump settings version counter");
            }
            Ok(true)
        }
        Err(sea_orm::DbErr::RecordNotInserted) => Ok(false),
        Err(e) => Err(report!(AuthError::Internal(format!(
            "failed to insert setting {db_key}: {e}"
        )))),
    }
}

pub async fn load_setting(
    db: &impl ConnectionTrait,
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
    db: &impl ConnectionTrait,
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
    db: &impl ConnectionTrait,
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
pub async fn bump_revocation_version(db: &impl ConnectionTrait, tenant_id: Uuid) -> Result<()> {
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

    // Defensive: if the row didn't exist (tenant created after migration), insert it.
    // Use on_conflict(do_nothing) to avoid racing with a concurrent insert.
    if result.rows_affected == 0 {
        let model = settings_version::ActiveModel {
            tenant_id: Set(tenant_id),
            version: Set(0),
            global_version: Set(0),
            revocation_version: Set(1),
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

/// Load or generate the JWT signing key from the database.
///
/// If a key already exists in the DB, returns it. Otherwise generates a new
/// 64-byte random key, stores it via upsert, and re-reads to handle races
/// (another instance may have stored a different key concurrently).
///
/// The key is stored encrypted at rest using the master encryption key. Legacy
/// unencrypted entries (base64-only) are transparently re-encrypted on read and
/// written back to the database.
///
/// This ensures all controller instances in an HA deployment share the same
/// JWT signing key.
pub async fn load_or_generate_jwt_key(db: &DatabaseConnection, tenant_id: Uuid) -> Result<Vec<u8>> {
    let b64_engine = base64::engine::general_purpose::STANDARD;

    // Try loading existing key from DB
    if let Some(value) = load_setting(db, tenant_id, SettingKey::JwtSigningKey).await?
        && let Some(stored) = value.as_str()
    {
        let b64 = if is_encrypted(stored) {
            // Encrypted path (current)
            decrypt_str(stored).map_err(|e| {
                report!(AuthError::Internal(format!(
                    "failed to decrypt JWT signing key from database: {e}"
                )))
            })?
        } else {
            // Legacy plaintext base64 — re-encrypt and write back
            tracing::info!("migrating JWT signing key to encrypted storage");
            let encrypted = encrypt_str(stored).map_err(|e| {
                report!(AuthError::Internal(format!(
                    "failed to encrypt legacy JWT signing key: {e}"
                )))
            })?;
            upsert_setting(
                db,
                tenant_id,
                SettingKey::JwtSigningKey,
                serde_json::json!(encrypted),
            )
            .await?;
            stored.to_string()
        };
        return b64_engine.decode(&b64).map_err(|e| {
            report!(AuthError::Internal(format!(
                "failed to decode JWT signing key from database: {e}"
            )))
        });
    }

    // Generate new random key
    let mut bytes = vec![0u8; JWT_KEY_LENGTH];
    rand::Rng::fill(&mut rand::rng(), &mut bytes[..]);
    let b64 = b64_engine.encode(&bytes);

    // Encrypt before storing
    let encrypted = encrypt_str(&b64).map_err(|e| {
        report!(AuthError::Internal(format!(
            "failed to encrypt new JWT signing key: {e}"
        )))
    })?;

    // Store with upsert (race-safe: another instance may store concurrently)
    upsert_setting(
        db,
        tenant_id,
        SettingKey::JwtSigningKey,
        serde_json::json!(encrypted),
    )
    .await?;

    // Re-read to get the canonical value (in case another instance won the race)
    if let Some(value) = load_setting(db, tenant_id, SettingKey::JwtSigningKey).await?
        && let Some(stored) = value.as_str()
    {
        let b64 = if is_encrypted(stored) {
            decrypt_str(stored).map_err(|e| {
                report!(AuthError::Internal(format!(
                    "failed to decrypt JWT signing key after store: {e}"
                )))
            })?
        } else {
            stored.to_string()
        };
        return b64_engine.decode(&b64).map_err(|e| {
            report!(AuthError::Internal(format!(
                "failed to decode JWT signing key after store: {e}"
            )))
        });
    }

    // Fallback: use the key we generated (should not normally reach here)
    Ok(bytes)
}

/// Migrate a file-based JWT signing key to the database.
///
/// If `{data_dir}/jwt_signing.key` exists and the DB does not yet have a key,
/// reads the file and stores it in the settings table. Returns `true` if a
/// migration was performed.
pub async fn migrate_file_jwt_key(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    data_dir: &Path,
) -> Result<bool> {
    let key_path = data_dir.join("jwt_signing.key");
    if !key_path.exists() {
        return Ok(false);
    }

    // Check if DB already has a key — don't overwrite it
    if load_setting(db, tenant_id, SettingKey::JwtSigningKey)
        .await?
        .is_some()
    {
        return Ok(false);
    }

    let bytes = std::fs::read(&key_path).map_err(|e| {
        report!(AuthError::Internal(format!(
            "failed to read JWT signing key file: {e}"
        )))
    })?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let encrypted = encrypt_str(&b64).map_err(|e| {
        report!(AuthError::Internal(format!(
            "failed to encrypt JWT signing key during file migration: {e}"
        )))
    })?;
    upsert_setting(
        db,
        tenant_id,
        SettingKey::JwtSigningKey,
        serde_json::json!(encrypted),
    )
    .await?;

    tracing::info!("migrated JWT signing key from file to encrypted database storage");
    Ok(true)
}
