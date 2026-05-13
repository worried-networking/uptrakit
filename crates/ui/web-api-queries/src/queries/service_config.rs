//! DB query helpers for the generic service config store.
//!
//! Used by the controller WebSocket handler to:
//! - Deliver stored config entries to connecting services.
//! - Upsert and delete entries on behalf of services.

use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter,
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

/// Audit view for a service config entry.
///
/// The `value` field is intentionally excluded — it may be sensitive.
pub struct ServiceConfigView {
    pub service_name: String,
    pub key: String,
    pub tenant_id: Option<Uuid>,
    pub sensitive: bool,
}

impl uptrakit_audit_log::AuditView for ServiceConfigView {
    const TARGET_TYPE: &'static str = "service_config";

    fn audit_target_id(&self) -> String {
        match self.tenant_id {
            Some(tid) => format!("{}:{}:{}", self.service_name, tid, self.key),
            None => format!("{}:global:{}", self.service_name, self.key),
        }
    }

    fn audit_target_display(&self) -> Option<String> {
        Some(self.key.clone())
    }

    fn audit_view(&self) -> serde_json::Value {
        serde_json::json!({
            "service_name": self.service_name,
            "key": self.key,
            "tenant_id": self.tenant_id,
            "sensitive": self.sensitive,
        })
    }
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

/// Upsert a config entry within an existing transaction.
///
/// Returns `(before, after, plaintext_value)`:
/// - `before` is `None` when inserting a new row, `Some(old_view)` when updating.
/// - `after` is always the new state.
/// - `plaintext_value` is the decrypted value for broadcasting.
pub async fn upsert_in_tx(
    tx: &impl ConnectionTrait,
    service_name: &str,
    tenant_id: Option<Uuid>,
    key: &str,
    value: serde_json::Value,
    sensitive: bool,
) -> Result<
    (
        Option<ServiceConfigView>,
        ServiceConfigView,
        serde_json::Value,
    ),
    sea_orm::DbErr,
> {
    let stored_value = if sensitive {
        encrypt_value(&value, service_name, key)
    } else {
        value.to_string()
    };

    let now = OffsetDateTime::now_utc();
    let after_view = ServiceConfigView {
        service_name: service_name.to_string(),
        key: key.to_string(),
        tenant_id,
        sensitive,
    };

    let before_view = if let Some(tid) = tenant_id {
        // Tenant-scoped upsert.
        let existing = tenant_service_config::Entity::find()
            .filter(tenant_service_config::Column::ServiceName.eq(service_name))
            .filter(tenant_service_config::Column::TenantId.eq(tid))
            .filter(tenant_service_config::Column::Key.eq(key))
            .one(tx)
            .await?;

        if let Some(existing) = existing {
            let before = ServiceConfigView {
                service_name: service_name.to_string(),
                key: key.to_string(),
                tenant_id,
                sensitive: existing.is_sensitive,
            };
            let mut active: tenant_service_config::ActiveModel = existing.into();
            active.value = Set(stored_value);
            active.is_sensitive = Set(sensitive);
            active.updated_at = Set(now);
            active.update(tx).await?;
            Some(before)
        } else {
            let active = tenant_service_config::ActiveModel {
                id: Set(Uuid::now_v7()),
                service_name: Set(service_name.to_string()),
                tenant_id: Set(tid),
                key: Set(key.to_string()),
                value: Set(stored_value),
                is_sensitive: Set(sensitive),
                created_at: Set(now),
                updated_at: Set(now),
            };
            active.insert(tx).await?;
            None
        }
    } else {
        // Global upsert.
        let existing = global_service_config::Entity::find()
            .filter(global_service_config::Column::ServiceName.eq(service_name))
            .filter(global_service_config::Column::Key.eq(key))
            .one(tx)
            .await?;

        if let Some(existing) = existing {
            let before = ServiceConfigView {
                service_name: service_name.to_string(),
                key: key.to_string(),
                tenant_id,
                sensitive: existing.is_sensitive,
            };
            let mut active: global_service_config::ActiveModel = existing.into();
            active.value = Set(stored_value);
            active.is_sensitive = Set(sensitive);
            active.updated_at = Set(now);
            active.update(tx).await?;
            Some(before)
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
            active.insert(tx).await?;
            None
        }
    };

    Ok((before_view, after_view, value))
}

/// Delete a config entry within an existing transaction.
///
/// Returns `Some(before_view)` if the row existed, `None` if not found.
pub async fn delete_in_tx(
    tx: &impl ConnectionTrait,
    service_name: &str,
    tenant_id: Option<Uuid>,
    key: &str,
) -> Result<Option<ServiceConfigView>, sea_orm::DbErr> {
    if let Some(tid) = tenant_id {
        // Check if entry exists first so we can build the before view.
        let existing = tenant_service_config::Entity::find()
            .filter(tenant_service_config::Column::ServiceName.eq(service_name))
            .filter(tenant_service_config::Column::TenantId.eq(tid))
            .filter(tenant_service_config::Column::Key.eq(key))
            .one(tx)
            .await?;

        if let Some(existing) = existing {
            let before = ServiceConfigView {
                service_name: service_name.to_string(),
                key: key.to_string(),
                tenant_id: Some(tid),
                sensitive: existing.is_sensitive,
            };
            tenant_service_config::Entity::delete_many()
                .filter(tenant_service_config::Column::ServiceName.eq(service_name))
                .filter(tenant_service_config::Column::TenantId.eq(tid))
                .filter(tenant_service_config::Column::Key.eq(key))
                .exec(tx)
                .await?;
            Ok(Some(before))
        } else {
            Ok(None)
        }
    } else {
        let existing = global_service_config::Entity::find()
            .filter(global_service_config::Column::ServiceName.eq(service_name))
            .filter(global_service_config::Column::Key.eq(key))
            .one(tx)
            .await?;

        if let Some(existing) = existing {
            let before = ServiceConfigView {
                service_name: service_name.to_string(),
                key: key.to_string(),
                tenant_id: None,
                sensitive: existing.is_sensitive,
            };
            global_service_config::Entity::delete_many()
                .filter(global_service_config::Column::ServiceName.eq(service_name))
                .filter(global_service_config::Column::Key.eq(key))
                .exec(tx)
                .await?;
            Ok(Some(before))
        } else {
            Ok(None)
        }
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
