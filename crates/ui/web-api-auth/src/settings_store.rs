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
use uptrakit_crypto::{decrypt_str, encrypt_str, is_encrypted};
use uptrakit_shared_db::entity::{global_setting, prelude::*, setting, settings_version};
use uuid::Uuid;

const JWT_KEY_LENGTH: usize = 64;

/// AAD bound to the JWT signing key ciphertext.
///
/// Using a dedicated AAD ensures the JWT key ciphertext cannot be reused as a
/// valid ciphertext in any other column, even if an attacker obtains the
/// master key and attempts a ciphertext relocation attack.
const JWT_KEY_AAD: &str = "uptrakit:settings:jwt_signing_key";

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

// ── Per-tenant settings (settings table) ─────────────────────────────────────

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
    debug_assert!(
        !key.is_global(),
        "upsert_setting called with global key {key}; use upsert_global_setting instead"
    );

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
    if let Err(e) = bump_settings_version(db, tenant_id).await {
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
    debug_assert!(
        !key.is_global(),
        "insert_setting_if_absent called with global key {key}; use insert_global_setting_if_absent instead"
    );

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
            if let Err(e) = bump_settings_version(db, tenant_id).await {
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
    if let Err(e) = bump_settings_version(db, tenant_id).await {
        tracing::warn!(
            error = ?e,
            key = key.as_str(),
            "failed to bump settings version counter"
        );
    }

    Ok(())
}

// ── Global settings (global_settings table) ──────────────────────────────────

/// Load every row from the `global_settings` table in a single query.
pub async fn load_all_global_settings(db: &DatabaseConnection) -> Result<RawSettings> {
    let rows = GlobalSetting::find().all(db).await.context_to()?;
    Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
}

/// Insert or update a single global setting and bump the global version counter.
pub async fn upsert_global_setting(
    db: &impl ConnectionTrait,
    key: SettingKey,
    value: serde_json::Value,
) -> Result<()> {
    debug_assert!(
        key.is_global(),
        "upsert_global_setting called with per-tenant key {key}; use upsert_setting instead"
    );

    let now = OffsetDateTime::now_utc();
    let db_key = key.as_str();

    let model = global_setting::ActiveModel {
        key: Set(db_key.to_string()),
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

    // Bump the global version counter (non-fatal on failure)
    if let Err(e) = bump_global_settings_version(db).await {
        tracing::warn!(error = ?e, key = db_key, "failed to bump global settings version counter");
    }

    Ok(())
}

/// Load a single global setting by key.
pub async fn load_global_setting(
    db: &impl ConnectionTrait,
    key: SettingKey,
) -> Result<Option<serde_json::Value>> {
    debug_assert!(
        key.is_global(),
        "load_global_setting called with per-tenant key {key}; use load_setting instead"
    );

    let row = GlobalSetting::find_by_id(key.as_str().to_string())
        .one(db)
        .await
        .context_to()?;
    Ok(row.map(|r| r.value))
}

/// Delete a single global setting by key.
pub async fn delete_global_setting(db: &impl ConnectionTrait, key: SettingKey) -> Result<()> {
    debug_assert!(
        key.is_global(),
        "delete_global_setting called with per-tenant key {key}; use delete_setting instead"
    );

    GlobalSetting::delete_many()
        .filter(global_setting::Column::Key.eq(key.as_str()))
        .exec(db)
        .await
        .context_to()?;

    // Bump the global version counter (non-fatal on failure)
    if let Err(e) = bump_global_settings_version(db).await {
        tracing::warn!(
            error = ?e,
            key = key.as_str(),
            "failed to bump global settings version counter"
        );
    }

    Ok(())
}

/// Returns whether multi-tenancy is enabled.
///
/// Missing or malformed values default to `false`, matching the current
/// single-tenant runtime behavior.
pub async fn is_multi_tenancy_enabled(db: &impl ConnectionTrait) -> Result<bool> {
    Ok(load_global_setting(db, SettingKey::MultiTenancyEnabled)
        .await?
        .and_then(|value| value.as_bool())
        .unwrap_or(false))
}

/// Insert a global setting only if it does not already exist.
///
/// Returns `true` if the row was inserted, `false` if a row with that key
/// already exists.
pub async fn insert_global_setting_if_absent(
    db: &impl ConnectionTrait,
    key: SettingKey,
    value: serde_json::Value,
) -> Result<bool> {
    debug_assert!(
        key.is_global(),
        "insert_global_setting_if_absent called with per-tenant key {key}; use insert_setting_if_absent instead"
    );

    let now = OffsetDateTime::now_utc();
    let db_key = key.as_str();

    let model = global_setting::ActiveModel {
        key: Set(db_key.to_string()),
        value: Set(value),
        updated_at: Set(now),
    };

    let result = GlobalSetting::insert(model)
        .on_conflict(
            OnConflict::column(global_setting::Column::Key)
                .do_nothing()
                .to_owned(),
        )
        .try_insert()
        .exec(db)
        .await;

    match result {
        Ok(_) => {
            if let Err(e) = bump_global_settings_version(db).await {
                tracing::warn!(error = ?e, key = db_key, "failed to bump global settings version counter");
            }
            Ok(true)
        }
        Err(sea_orm::DbErr::RecordNotInserted) => Ok(false),
        Err(e) => Err(report!(AuthError::Internal(format!(
            "failed to insert global setting {db_key}: {e}"
        )))),
    }
}

// ── Raw-key settings (bypass SettingKey validation) ──────────────────────────
//
// These functions accept raw `&str` keys, allowing plugin crates to manage
// their own settings without adding variants to the `SettingKey` enum.
//
// The implementations live in `uptrakit_shared_db::raw_settings` so that
// plugin crates can use them without depending on `web-api-auth`. These
// re-exports keep existing callers in the `web-api` layer working.

/// Insert or update a per-tenant setting using a raw key string.
pub async fn upsert_setting_raw(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    key: &str,
    value: serde_json::Value,
) -> Result<()> {
    uptrakit_shared_db::raw_settings::upsert_setting_raw(db, tenant_id, key, value)
        .await
        .map_err(|e| report!(AuthError::Internal(e.to_string())))
}

/// Insert or update a global setting using a raw key string.
pub async fn upsert_global_setting_raw(
    db: &impl ConnectionTrait,
    key: &str,
    value: serde_json::Value,
) -> Result<()> {
    uptrakit_shared_db::raw_settings::upsert_global_setting_raw(db, key, value)
        .await
        .map_err(|e| report!(AuthError::Internal(e.to_string())))
}

/// Load all per-tenant settings whose key starts with `prefix`.
pub async fn load_settings_by_prefix(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    prefix: &str,
) -> Result<HashMap<String, serde_json::Value>> {
    uptrakit_shared_db::raw_settings::load_settings_by_prefix(db, tenant_id, prefix)
        .await
        .map_err(|e| report!(AuthError::Internal(e.to_string())))
}

/// Load and decode all per-tenant settings whose key starts with `prefix`.
pub async fn load_typed_settings_by_prefix<T>(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    prefix: &str,
) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let raw = uptrakit_shared_db::raw_settings::load_settings_by_prefix(db, tenant_id, prefix)
        .await
        .map_err(|error| report!(AuthError::Internal(error.to_string())))?;

    uptrakit_shared_db::raw_settings::decode_prefixed_settings(prefix, &raw)
        .map_err(|error| report!(AuthError::Internal(error.to_string())))
}

/// Load all global settings whose key starts with `prefix`.
pub async fn load_global_settings_by_prefix(
    db: &DatabaseConnection,
    prefix: &str,
) -> Result<HashMap<String, serde_json::Value>> {
    uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(db, prefix)
        .await
        .map_err(|e| report!(AuthError::Internal(e.to_string())))
}

/// Load and decode all global settings whose key starts with `prefix`.
pub async fn load_typed_global_settings_by_prefix<T>(
    db: &DatabaseConnection,
    prefix: &str,
) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let raw = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(db, prefix)
        .await
        .map_err(|error| report!(AuthError::Internal(error.to_string())))?;

    uptrakit_shared_db::raw_settings::decode_prefixed_settings(prefix, &raw)
        .map_err(|error| report!(AuthError::Internal(error.to_string())))
}

// ── Version tracking ─────────────────────────────────────────────────────────

/// Bump the per-tenant settings version counter after a per-tenant settings write.
///
/// Increments `version` on the specific tenant's `settings_version` row only.
/// Non-fatal on failure: callers should log and continue.
pub async fn bump_settings_version(db: &impl ConnectionTrait, tenant_id: Uuid) -> Result<()> {
    uptrakit_shared_db::raw_settings::bump_settings_version(db, tenant_id)
        .await
        .map_err(|e| report!(AuthError::Internal(e.to_string())))
}

/// Bump the global settings version counter after a global settings write.
///
/// Increments `global_version` on ALL tenant rows in `settings_version`.
/// Non-fatal on failure: callers should log and continue.
pub async fn bump_global_settings_version(db: &impl ConnectionTrait) -> Result<()> {
    uptrakit_shared_db::raw_settings::bump_global_settings_version(db)
        .await
        .map_err(|e| report!(AuthError::Internal(e.to_string())))
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

// ── JWT key management ───────────────────────────────────────────────────────

/// Load or generate the JWT signing key from the database.
///
/// The JWT signing key is a global setting stored in the `global_settings` table.
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
pub async fn load_or_generate_jwt_key(db: &DatabaseConnection) -> Result<Vec<u8>> {
    let b64_engine = base64::engine::general_purpose::STANDARD;

    // Try loading existing key from DB
    if let Some(value) = load_global_setting(db, SettingKey::JwtSigningKey).await?
        && let Some(stored) = value.as_str()
    {
        let b64 = if is_encrypted(stored) {
            // Encrypted path (current): use decrypt_str so ENC:v2: tokens
            // are verified against the correct context; ENC:v1: tokens are accepted
            // with empty AAD for backward compatibility with existing installations.
            decrypt_str(stored, JWT_KEY_AAD).map_err(|e| {
                report!(AuthError::Internal(format!(
                    "failed to decrypt JWT signing key from database: {e}"
                )))
            })?
        } else {
            // Legacy plaintext base64 — re-encrypt with context-bound ENC:v2: format
            tracing::info!("migrating JWT signing key to encrypted storage");
            let encrypted = encrypt_str(stored, JWT_KEY_AAD).map_err(|e| {
                report!(AuthError::Internal(format!(
                    "failed to encrypt legacy JWT signing key: {e}"
                )))
            })?;
            upsert_global_setting(db, SettingKey::JwtSigningKey, serde_json::json!(encrypted))
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

    // Encrypt before storing (context-bound ENC:v2: format)
    let encrypted = encrypt_str(&b64, JWT_KEY_AAD).map_err(|e| {
        report!(AuthError::Internal(format!(
            "failed to encrypt new JWT signing key: {e}"
        )))
    })?;

    // Store with upsert (race-safe: another instance may store concurrently)
    upsert_global_setting(db, SettingKey::JwtSigningKey, serde_json::json!(encrypted)).await?;

    // Re-read to get the canonical value (in case another instance won the race)
    if let Some(value) = load_global_setting(db, SettingKey::JwtSigningKey).await?
        && let Some(stored) = value.as_str()
    {
        let b64 = if is_encrypted(stored) {
            decrypt_str(stored, JWT_KEY_AAD).map_err(|e| {
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
/// reads the file and stores it in the `global_settings` table. Returns `true`
/// if a migration was performed.
pub async fn migrate_file_jwt_key(db: &DatabaseConnection, data_dir: &Path) -> Result<bool> {
    let key_path = data_dir.join("jwt_signing.key");
    if !key_path.exists() {
        return Ok(false);
    }

    // Check if DB already has a key — don't overwrite it
    if load_global_setting(db, SettingKey::JwtSigningKey)
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
    let encrypted = encrypt_str(&b64, JWT_KEY_AAD).map_err(|e| {
        report!(AuthError::Internal(format!(
            "failed to encrypt JWT signing key during file migration: {e}"
        )))
    })?;
    upsert_global_setting(db, SettingKey::JwtSigningKey, serde_json::json!(encrypted)).await?;

    // Remove the plaintext key file now that it has been migrated to encrypted
    // DB storage. Failure is non-fatal: warn and continue so the controller
    // still starts, but log clearly so operators know to remove it manually.
    if let Err(e) = std::fs::remove_file(&key_path) {
        tracing::warn!(
            path = %key_path.display(),
            error = %e,
            "JWT key migration: could not delete plaintext key file — remove it manually"
        );
    } else {
        tracing::info!(
            path = %key_path.display(),
            "deleted plaintext JWT signing key file after migration to encrypted DB storage"
        );
    }

    tracing::info!("migrated JWT signing key from file to encrypted database storage");
    Ok(true)
}
