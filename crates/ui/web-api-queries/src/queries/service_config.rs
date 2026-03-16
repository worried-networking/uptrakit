//! DB query helpers for the generic service config store.
//!
//! Used by the controller WebSocket handler to:
//! - Deliver stored config entries to connecting services.
//! - Upsert and delete entries on behalf of services.

use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};
use time::OffsetDateTime;
use uuid::Uuid;

use uptrakit_shared_db::entity::{global_service_config, tenant_service_config};

/// A single stored config entry (after decryption).
pub struct ServiceConfigRow {
    pub tenant_id: Option<Uuid>,
    pub key: String,
    /// Plaintext JSON value (decrypted if sensitive).
    pub value: serde_json::Value,
}

/// Load all config entries for a given `service_name`.
///
/// Returns both tenant-scoped and global entries. Sensitive values are
/// decrypted using the master key if available.
pub async fn load_for_service(
    db: &DatabaseConnection,
    service_name: &str,
) -> Result<Vec<ServiceConfigRow>, sea_orm::DbErr> {
    let mut rows = Vec::new();

    // Load tenant-scoped entries.
    let tenant_entries = tenant_service_config::Entity::find()
        .filter(tenant_service_config::Column::ServiceName.eq(service_name))
        .all(db)
        .await?;

    for entry in tenant_entries {
        let value = decrypt_value(&entry.value, entry.is_sensitive, service_name, &entry.key);
        rows.push(ServiceConfigRow {
            tenant_id: Some(entry.tenant_id),
            key: entry.key,
            value,
        });
    }

    // Load global entries.
    let global_entries = global_service_config::Entity::find()
        .filter(global_service_config::Column::ServiceName.eq(service_name))
        .all(db)
        .await?;

    for entry in global_entries {
        let value = decrypt_value(&entry.value, entry.is_sensitive, service_name, &entry.key);
        rows.push(ServiceConfigRow {
            tenant_id: None,
            key: entry.key,
            value,
        });
    }

    Ok(rows)
}

/// Upsert a config entry. Encrypts the value if `sensitive` is true.
///
/// Returns the plaintext value (for broadcasting to other instances).
pub async fn upsert(
    db: &DatabaseConnection,
    service_name: &str,
    tenant_id: Option<Uuid>,
    key: &str,
    value: serde_json::Value,
    sensitive: bool,
) -> Result<serde_json::Value, sea_orm::DbErr> {
    let stored_value = if sensitive {
        encrypt_value(&value, service_name, key)
    } else {
        value.to_string()
    };

    let now = OffsetDateTime::now_utc();

    if let Some(tenant_id) = tenant_id {
        // Tenant-scoped upsert.
        let existing = tenant_service_config::Entity::find()
            .filter(tenant_service_config::Column::ServiceName.eq(service_name))
            .filter(tenant_service_config::Column::TenantId.eq(tenant_id))
            .filter(tenant_service_config::Column::Key.eq(key))
            .one(db)
            .await?;

        if let Some(existing) = existing {
            let mut active: tenant_service_config::ActiveModel = existing.into();
            active.value = Set(stored_value);
            active.is_sensitive = Set(sensitive);
            active.updated_at = Set(now);
            active.update(db).await?;
        } else {
            let active = tenant_service_config::ActiveModel {
                id: Set(Uuid::now_v7()),
                service_name: Set(service_name.to_string()),
                tenant_id: Set(tenant_id),
                key: Set(key.to_string()),
                value: Set(stored_value),
                is_sensitive: Set(sensitive),
                created_at: Set(now),
                updated_at: Set(now),
            };
            active.insert(db).await?;
        }
    } else {
        // Global upsert.
        let existing = global_service_config::Entity::find()
            .filter(global_service_config::Column::ServiceName.eq(service_name))
            .filter(global_service_config::Column::Key.eq(key))
            .one(db)
            .await?;

        if let Some(existing) = existing {
            let mut active: global_service_config::ActiveModel = existing.into();
            active.value = Set(stored_value);
            active.is_sensitive = Set(sensitive);
            active.updated_at = Set(now);
            active.update(db).await?;
        } else {
            let active = global_service_config::ActiveModel {
                id: Set(Uuid::now_v7()),
                service_name: Set(service_name.to_string()),
                key: Set(key.to_string()),
                value: Set(stored_value),
                is_sensitive: Set(sensitive),
                created_at: Set(now),
                updated_at: Set(now),
            };
            active.insert(db).await?;
        }
    }

    Ok(value)
}

/// Delete a config entry.
///
/// Returns `true` if an entry was deleted, `false` if not found.
pub async fn delete(
    db: &DatabaseConnection,
    service_name: &str,
    tenant_id: Option<Uuid>,
    key: &str,
) -> Result<bool, sea_orm::DbErr> {
    if let Some(tenant_id) = tenant_id {
        let result = tenant_service_config::Entity::delete_many()
            .filter(tenant_service_config::Column::ServiceName.eq(service_name))
            .filter(tenant_service_config::Column::TenantId.eq(tenant_id))
            .filter(tenant_service_config::Column::Key.eq(key))
            .exec(db)
            .await?;
        Ok(result.rows_affected > 0)
    } else {
        let result = global_service_config::Entity::delete_many()
            .filter(global_service_config::Column::ServiceName.eq(service_name))
            .filter(global_service_config::Column::Key.eq(key))
            .exec(db)
            .await?;
        Ok(result.rows_affected > 0)
    }
}

/// Encrypt a config value using the master key.
///
/// Falls back to plaintext JSON if encryption is not enabled.
fn encrypt_value(value: &serde_json::Value, service_name: &str, key: &str) -> String {
    let plaintext = value.to_string();
    let aad = format!("uptrakit:service_config:{service_name}:{key}");
    match uptrakit_crypto::encrypt_str(&plaintext, &aad) {
        Ok(encrypted) => encrypted,
        Err(_) => {
            tracing::warn!(
                service_name,
                key,
                "failed to encrypt service config value; storing plaintext"
            );
            plaintext
        }
    }
}

/// Decrypt a config value.
///
/// Returns the value as-is if `is_sensitive` is false or decryption fails.
fn decrypt_value(
    stored: &str,
    is_sensitive: bool,
    service_name: &str,
    key: &str,
) -> serde_json::Value {
    if !is_sensitive {
        return serde_json::from_str(stored)
            .unwrap_or(serde_json::Value::String(stored.to_string()));
    }
    let aad = format!("uptrakit:service_config:{service_name}:{key}");
    match uptrakit_crypto::decrypt_str(stored, &aad) {
        Ok(plaintext) => {
            serde_json::from_str(&plaintext).unwrap_or(serde_json::Value::String(plaintext))
        }
        Err(_) => {
            tracing::warn!(
                service_name,
                key,
                "failed to decrypt service config value; returning stored value as-is"
            );
            serde_json::from_str(stored).unwrap_or(serde_json::Value::String(stored.to_string()))
        }
    }
}
