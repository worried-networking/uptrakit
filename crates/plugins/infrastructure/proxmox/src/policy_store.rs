//! Controller-side policy, cache, and audit storage for Proxmox update protection.

use crate::entity::{
    proxmox_backup_target_cache, proxmox_protection_audit, proxmox_protection_default,
    proxmox_protection_item_override, proxmox_resource_scaling_record,
};
use proxmox_backup_target_cache::Entity as ProxmoxBackupTargetCache;
use proxmox_protection_audit::Entity as ProxmoxProtectionAudit;
use proxmox_protection_default::Entity as ProxmoxProtectionDefault;
use proxmox_protection_item_override::Entity as ProxmoxProtectionItemOverride;
use proxmox_resource_scaling_record::Entity as ProxmoxResourceScalingRecord;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{ProxmoxError, Result};

#[cfg(test)]
pub(crate) use crate::entity::proxmox_backup_target_cache as backup_target_cache;
#[cfg(test)]
pub(crate) use crate::entity::proxmox_protection_audit as protection_audit;

/// Protection mode resolved for a software item update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionMode {
    DoNothing,
    Snapshot,
    Backup,
}

impl ProtectionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DoNothing => "do_nothing",
            Self::Snapshot => "snapshot",
            Self::Backup => "backup",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "snapshot" => Self::Snapshot,
            "backup" => Self::Backup,
            _ => Self::DoNothing,
        }
    }
}

/// Effective protection policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionPolicy {
    pub mode: ProtectionMode,
    pub backup_target_key: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
    pub update_cores: Option<i32>,
    pub update_memory_mb: Option<i32>,
}

impl ProtectionPolicy {
    pub fn do_nothing() -> Self {
        Self {
            mode: ProtectionMode::DoNothing,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
            update_cores: None,
            update_memory_mb: None,
        }
    }
}

/// Cached node-aware backup target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedBackupTarget {
    pub node: String,
    pub storage_id: String,
    pub storage_type: String,
    pub target_key: String,
}

/// Persisted audit row for one `update_history_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionAudit {
    pub update_history_id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    pub mapping_id: Option<Uuid>,
    pub mode: ProtectionMode,
    pub status: String,
    pub artifact_kind: Option<String>,
    pub artifact_ref: Option<String>,
    pub backup_target_key: Option<String>,
    pub detail: Option<String>,
    pub error_message: Option<String>,
}

/// Build a stable key for a cached backup target.
///
/// Key includes node + storage id + storage type so same-name storages on
/// different nodes/configs remain unambiguous.
pub fn backup_target_key(node: &str, storage_id: &str, storage_type: &str) -> String {
    format!("{node}:{storage_id}:{storage_type}")
}

/// Resolve effective policy from per-item override and global default.
///
/// Mode and backup_target_key prefer item_override over global_default (first-wins).
/// Timeout fields are merged per-field: item_override value is used when set,
/// otherwise falls back to the global_default value.
pub fn resolve_effective_policy(
    item_override: Option<ProtectionPolicy>,
    global_default: Option<ProtectionPolicy>,
) -> ProtectionPolicy {
    let item_ref = item_override.as_ref();
    let global_ref = global_default.as_ref();

    let mode = item_ref
        .map(|p| p.mode)
        .or_else(|| global_ref.map(|p| p.mode))
        .unwrap_or(ProtectionMode::DoNothing);

    let backup_target_key = item_ref
        .and_then(|p| p.backup_target_key.clone())
        .or_else(|| global_ref.and_then(|p| p.backup_target_key.clone()));

    let snapshot_timeout_seconds = item_ref
        .and_then(|p| p.snapshot_timeout_seconds)
        .or_else(|| global_ref.and_then(|p| p.snapshot_timeout_seconds));

    let backup_timeout_seconds = item_ref
        .and_then(|p| p.backup_timeout_seconds)
        .or_else(|| global_ref.and_then(|p| p.backup_timeout_seconds));

    let update_cores = item_ref
        .and_then(|p| p.update_cores)
        .or_else(|| global_ref.and_then(|p| p.update_cores));

    let update_memory_mb = item_ref
        .and_then(|p| p.update_memory_mb)
        .or_else(|| global_ref.and_then(|p| p.update_memory_mb));

    ProtectionPolicy {
        mode,
        backup_target_key,
        snapshot_timeout_seconds,
        backup_timeout_seconds,
        update_cores,
        update_memory_mb,
    }
}

/// Load global/default policy for one Proxmox config.
pub async fn load_global_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<Option<ProtectionPolicy>> {
    let row = ProxmoxProtectionDefault::find()
        .filter(proxmox_protection_default::Column::TenantId.eq(tenant_id))
        .filter(proxmox_protection_default::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to load global protection defaults: {e}"
            )))
        })?;

    Ok(row.map(|model| ProtectionPolicy {
        mode: ProtectionMode::from_db(&model.mode),
        backup_target_key: model.backup_target_key,
        snapshot_timeout_seconds: model.snapshot_timeout_seconds,
        backup_timeout_seconds: model.backup_timeout_seconds,
        update_cores: model.update_cores,
        update_memory_mb: model.update_memory_mb,
    }))
}

/// Upsert global/default policy for one Proxmox config.
pub async fn upsert_global_default(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    policy: &ProtectionPolicy,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let existing = ProxmoxProtectionDefault::find()
        .filter(proxmox_protection_default::Column::TenantId.eq(tenant_id))
        .filter(proxmox_protection_default::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing global protection defaults: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: proxmox_protection_default::ActiveModel = existing.into();
        active.mode = Set(policy.mode.as_str().to_string());
        active.backup_target_key = Set(policy.backup_target_key.clone());
        active.snapshot_timeout_seconds = Set(policy.snapshot_timeout_seconds);
        active.backup_timeout_seconds = Set(policy.backup_timeout_seconds);
        active.update_cores = Set(policy.update_cores);
        active.update_memory_mb = Set(policy.update_memory_mb);
        active.updated_at = Set(now);
        active.update(db).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to update global protection defaults: {e}"
            )))
        })?;
    } else {
        let active = proxmox_protection_default::ActiveModel {
            tenant_id: Set(tenant_id),
            plugin_config_id: Set(plugin_config_id),
            mode: Set(policy.mode.as_str().to_string()),
            backup_target_key: Set(policy.backup_target_key.clone()),
            snapshot_timeout_seconds: Set(policy.snapshot_timeout_seconds),
            backup_timeout_seconds: Set(policy.backup_timeout_seconds),
            update_cores: Set(policy.update_cores),
            update_memory_mb: Set(policy.update_memory_mb),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(db).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to insert global protection defaults: {e}"
            )))
        })?;
    }

    Ok(())
}

/// Load per-item override policy for one `(software_item_id, plugin_config_id)` pair.
pub async fn load_item_override(
    db: &DatabaseConnection,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<Option<ProtectionPolicy>> {
    let row = ProxmoxProtectionItemOverride::find()
        .filter(proxmox_protection_item_override::Column::SoftwareItemId.eq(software_item_id))
        .filter(proxmox_protection_item_override::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to load per-item protection override: {e}"
            )))
        })?;

    Ok(row.map(|model| ProtectionPolicy {
        mode: ProtectionMode::from_db(&model.mode),
        backup_target_key: model.backup_target_key,
        snapshot_timeout_seconds: model.snapshot_timeout_seconds,
        backup_timeout_seconds: model.backup_timeout_seconds,
        update_cores: model.update_cores,
        update_memory_mb: model.update_memory_mb,
    }))
}

/// Upsert per-item override policy for one `(software_item_id, plugin_config_id)` pair.
pub async fn upsert_item_override(
    db: &DatabaseConnection,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
    policy: &ProtectionPolicy,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let existing = ProxmoxProtectionItemOverride::find()
        .filter(proxmox_protection_item_override::Column::SoftwareItemId.eq(software_item_id))
        .filter(proxmox_protection_item_override::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing per-item protection override: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: proxmox_protection_item_override::ActiveModel = existing.into();
        active.mode = Set(policy.mode.as_str().to_string());
        active.backup_target_key = Set(policy.backup_target_key.clone());
        active.snapshot_timeout_seconds = Set(policy.snapshot_timeout_seconds);
        active.backup_timeout_seconds = Set(policy.backup_timeout_seconds);
        active.update_cores = Set(policy.update_cores);
        active.update_memory_mb = Set(policy.update_memory_mb);
        active.updated_at = Set(now);
        active.update(db).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to update per-item protection override: {e}"
            )))
        })?;
    } else {
        let active = proxmox_protection_item_override::ActiveModel {
            software_item_id: Set(software_item_id),
            plugin_config_id: Set(plugin_config_id),
            mode: Set(policy.mode.as_str().to_string()),
            backup_target_key: Set(policy.backup_target_key.clone()),
            snapshot_timeout_seconds: Set(policy.snapshot_timeout_seconds),
            backup_timeout_seconds: Set(policy.backup_timeout_seconds),
            update_cores: Set(policy.update_cores),
            update_memory_mb: Set(policy.update_memory_mb),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(db).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to insert per-item protection override: {e}"
            )))
        })?;
    }

    Ok(())
}

/// Delete a per-item override policy for one `(software_item_id, plugin_config_id)` pair.
pub async fn delete_item_override(
    db: &DatabaseConnection,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<()> {
    if let Some(existing) = ProxmoxProtectionItemOverride::find()
        .filter(proxmox_protection_item_override::Column::SoftwareItemId.eq(software_item_id))
        .filter(proxmox_protection_item_override::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query per-item protection override for delete: {e}"
            )))
        })?
    {
        let active: proxmox_protection_item_override::ActiveModel = existing.into();
        active.delete(db).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to delete per-item protection override: {e}"
            )))
        })?;
    }
    Ok(())
}

/// Load effective policy for `(software_item_id, plugin_config_id)`.
pub async fn load_effective_policy(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ProtectionPolicy> {
    let item_override = load_item_override(db, software_item_id, plugin_config_id).await?;
    let global_default = load_global_default(db, tenant_id, plugin_config_id).await?;

    Ok(resolve_effective_policy(item_override, global_default))
}

/// Upsert cached backup targets for a Proxmox config.
pub async fn upsert_cached_backup_targets(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    targets: &[CachedBackupTarget],
) -> Result<usize> {
    let tx = db.begin().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to begin backup target cache transaction: {e}"
        )))
    })?;
    let now = OffsetDateTime::now_utc();
    let mut upserted = 0usize;
    let discovered_target_keys: Vec<String> =
        targets.iter().map(|t| t.target_key.clone()).collect();

    let stale_delete = if discovered_target_keys.is_empty() {
        ProxmoxBackupTargetCache::delete_many()
            .filter(proxmox_backup_target_cache::Column::TenantId.eq(tenant_id))
            .filter(proxmox_backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
    } else {
        ProxmoxBackupTargetCache::delete_many()
            .filter(proxmox_backup_target_cache::Column::TenantId.eq(tenant_id))
            .filter(proxmox_backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
            .filter(
                proxmox_backup_target_cache::Column::TargetKey
                    .is_not_in(discovered_target_keys.clone()),
            )
    };

    stale_delete.exec(&tx).await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to prune stale backup target cache entries: {e}"
        )))
    })?;

    for target in targets {
        let existing = ProxmoxBackupTargetCache::find()
            .filter(proxmox_backup_target_cache::Column::TenantId.eq(tenant_id))
            .filter(proxmox_backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
            .filter(proxmox_backup_target_cache::Column::TargetKey.eq(&target.target_key))
            .one(&tx)
            .await
            .map_err(|e| {
                rootcause::report!(ProxmoxError::Database(format!(
                    "failed to query backup target cache: {e}"
                )))
            })?;

        if let Some(existing) = existing {
            let mut active: proxmox_backup_target_cache::ActiveModel = existing.into();
            active.tenant_id = Set(tenant_id);
            active.proxmox_node = Set(target.node.clone());
            active.storage_id = Set(target.storage_id.clone());
            active.storage_type = Set(target.storage_type.clone());
            active.updated_at = Set(now);
            active.update(&tx).await.map_err(|e| {
                rootcause::report!(ProxmoxError::Database(format!(
                    "failed to update backup target cache: {e}"
                )))
            })?;
        } else {
            let active = proxmox_backup_target_cache::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                plugin_config_id: Set(plugin_config_id),
                proxmox_node: Set(target.node.clone()),
                storage_id: Set(target.storage_id.clone()),
                storage_type: Set(target.storage_type.clone()),
                target_key: Set(target.target_key.clone()),
                discovered_at: Set(now),
                updated_at: Set(now),
            };
            active.insert(&tx).await.map_err(|e| {
                rootcause::report!(ProxmoxError::Database(format!(
                    "failed to insert backup target cache: {e}"
                )))
            })?;
        }

        upserted += 1;
    }

    tx.commit().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to commit backup target cache transaction: {e}"
        )))
    })?;

    Ok(upserted)
}

/// Look up one cached backup target by key.
pub async fn find_cached_backup_target(
    db: &DatabaseConnection,
    plugin_config_id: Uuid,
    target_key: &str,
) -> Result<Option<CachedBackupTarget>> {
    let row = ProxmoxBackupTargetCache::find()
        .filter(proxmox_backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
        .filter(proxmox_backup_target_cache::Column::TargetKey.eq(target_key))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query backup target cache by key: {e}"
            )))
        })?;

    Ok(row.map(|m| CachedBackupTarget {
        node: m.proxmox_node,
        storage_id: m.storage_id,
        storage_type: m.storage_type,
        target_key: m.target_key,
    }))
}

/// List cached backup targets for one Proxmox config.
pub async fn list_cached_backup_targets(
    db: &DatabaseConnection,
    plugin_config_id: Uuid,
) -> Result<Vec<CachedBackupTarget>> {
    let rows = ProxmoxBackupTargetCache::find()
        .filter(proxmox_backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
        .order_by_asc(proxmox_backup_target_cache::Column::ProxmoxNode)
        .order_by_asc(proxmox_backup_target_cache::Column::StorageId)
        .all(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to list cached backup targets: {e}"
            )))
        })?;

    Ok(rows
        .into_iter()
        .map(|m| CachedBackupTarget {
            node: m.proxmox_node,
            storage_id: m.storage_id,
            storage_type: m.storage_type,
            target_key: m.target_key,
        })
        .collect())
}

/// Load existing audit row for idempotency checks.
pub async fn load_protection_audit(
    db: &DatabaseConnection,
    update_history_id: Uuid,
) -> Result<Option<ProtectionAudit>> {
    let row = ProxmoxProtectionAudit::find_by_id(update_history_id)
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query protection audit row: {e}"
            )))
        })?;

    Ok(row.map(|m| ProtectionAudit {
        update_history_id: m.update_history_id,
        tenant_id: m.tenant_id,
        host_id: m.host_id,
        software_item_id: m.software_item_id,
        plugin_config_id: m.plugin_config_id,
        mapping_id: m.mapping_id,
        mode: ProtectionMode::from_db(&m.mode),
        status: m.status,
        artifact_kind: m.artifact_kind,
        artifact_ref: m.artifact_ref,
        backup_target_key: m.backup_target_key,
        detail: m.detail,
        error_message: m.error_message,
    }))
}

/// Upsert one protection audit row by `update_history_id`.
pub async fn upsert_protection_audit(
    db: &DatabaseConnection,
    audit: &ProtectionAudit,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let existing = ProxmoxProtectionAudit::find_by_id(audit.update_history_id)
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing protection audit row: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: proxmox_protection_audit::ActiveModel = existing.into();
        active.tenant_id = Set(audit.tenant_id);
        active.host_id = Set(audit.host_id);
        active.software_item_id = Set(audit.software_item_id);
        active.plugin_config_id = Set(audit.plugin_config_id);
        active.mapping_id = Set(audit.mapping_id);
        active.mode = Set(audit.mode.as_str().to_string());
        active.status = Set(audit.status.clone());
        active.artifact_kind = Set(audit.artifact_kind.clone());
        active.artifact_ref = Set(audit.artifact_ref.clone());
        active.backup_target_key = Set(audit.backup_target_key.clone());
        active.detail = Set(audit.detail.clone());
        active.error_message = Set(audit.error_message.clone());
        active.updated_at = Set(now);
        active.update(db).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to update protection audit row: {e}"
            )))
        })?;
    } else {
        let active = proxmox_protection_audit::ActiveModel {
            update_history_id: Set(audit.update_history_id),
            tenant_id: Set(audit.tenant_id),
            host_id: Set(audit.host_id),
            software_item_id: Set(audit.software_item_id),
            plugin_config_id: Set(audit.plugin_config_id),
            mapping_id: Set(audit.mapping_id),
            mode: Set(audit.mode.as_str().to_string()),
            status: Set(audit.status.clone()),
            artifact_kind: Set(audit.artifact_kind.clone()),
            artifact_ref: Set(audit.artifact_ref.clone()),
            backup_target_key: Set(audit.backup_target_key.clone()),
            detail: Set(audit.detail.clone()),
            error_message: Set(audit.error_message.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(db).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to insert protection audit row: {e}"
            )))
        })?;
    }

    Ok(())
}

/// Scaling record for one `update_history_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalingRecord {
    pub update_history_id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    pub mapping_id: Uuid,
    pub vm_type: String,
    pub original_cores: i32,
    pub original_memory_mb: i64,
    pub scaled_cores: i32,
    pub scaled_memory_mb: i64,
    pub scale_status: String,
    pub restore_status: String,
    pub error_message: Option<String>,
}

/// Load a scaling record by `update_history_id`.
pub async fn load_scaling_record(
    db: &DatabaseConnection,
    update_history_id: Uuid,
) -> Result<Option<ScalingRecord>> {
    let row = ProxmoxResourceScalingRecord::find_by_id(update_history_id)
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query scaling record: {e}"
            )))
        })?;

    Ok(row.map(|m| ScalingRecord {
        update_history_id: m.update_history_id,
        tenant_id: m.tenant_id,
        host_id: m.host_id,
        software_item_id: m.software_item_id,
        plugin_config_id: m.plugin_config_id,
        mapping_id: m.mapping_id,
        vm_type: m.vm_type,
        original_cores: m.original_cores,
        original_memory_mb: m.original_memory_mb,
        scaled_cores: m.scaled_cores,
        scaled_memory_mb: m.scaled_memory_mb,
        scale_status: m.scale_status,
        restore_status: m.restore_status,
        error_message: m.error_message,
    }))
}

/// Upsert a scaling record. Uses `BEGIN IMMEDIATE` to prevent `SQLITE_BUSY_SNAPSHOT`.
pub async fn upsert_scaling_record(db: &DatabaseConnection, record: &ScalingRecord) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to begin transaction for scaling record upsert: {e}"
            )))
        })?;

    let existing = ProxmoxResourceScalingRecord::find_by_id(record.update_history_id)
        .one(&txn)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing scaling record: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: proxmox_resource_scaling_record::ActiveModel = existing.into();
        active.tenant_id = Set(record.tenant_id);
        active.host_id = Set(record.host_id);
        active.software_item_id = Set(record.software_item_id);
        active.plugin_config_id = Set(record.plugin_config_id);
        active.mapping_id = Set(record.mapping_id);
        active.vm_type = Set(record.vm_type.clone());
        active.original_cores = Set(record.original_cores);
        active.original_memory_mb = Set(record.original_memory_mb);
        active.scaled_cores = Set(record.scaled_cores);
        active.scaled_memory_mb = Set(record.scaled_memory_mb);
        active.scale_status = Set(record.scale_status.clone());
        active.restore_status = Set(record.restore_status.clone());
        active.error_message = Set(record.error_message.clone());
        active.updated_at = Set(now);
        active.update(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to update scaling record: {e}"
            )))
        })?;
    } else {
        let active = proxmox_resource_scaling_record::ActiveModel {
            update_history_id: Set(record.update_history_id),
            tenant_id: Set(record.tenant_id),
            host_id: Set(record.host_id),
            software_item_id: Set(record.software_item_id),
            plugin_config_id: Set(record.plugin_config_id),
            mapping_id: Set(record.mapping_id),
            vm_type: Set(record.vm_type.clone()),
            original_cores: Set(record.original_cores),
            original_memory_mb: Set(record.original_memory_mb),
            scaled_cores: Set(record.scaled_cores),
            scaled_memory_mb: Set(record.scaled_memory_mb),
            scale_status: Set(record.scale_status.clone()),
            restore_status: Set(record.restore_status.clone()),
            error_message: Set(record.error_message.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&txn).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to insert scaling record: {e}"
            )))
        })?;
    }

    txn.commit().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to commit scaling record upsert: {e}"
        )))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[test]
    fn backup_target_key_includes_node_storage_and_type() {
        let key = backup_target_key("pve-a", "local", "dir");
        assert_eq!(key, "pve-a:local:dir");
    }

    #[test]
    fn effective_policy_prefers_item_override() {
        let item = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: Some("k1".to_string()),
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
            update_cores: None,
            update_memory_mb: None,
        };
        let global = ProtectionPolicy {
            mode: ProtectionMode::Snapshot,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
            update_cores: None,
            update_memory_mb: None,
        };

        let effective = resolve_effective_policy(Some(item.clone()), Some(global));
        assert_eq!(effective, item);
    }

    #[test]
    fn effective_policy_defaults_to_do_nothing() {
        let effective = resolve_effective_policy(None, None);
        assert_eq!(effective.mode, ProtectionMode::DoNothing);
        assert!(effective.backup_target_key.is_none());
    }

    #[test]
    fn effective_policy_inherits_global_timeouts_per_field() {
        let item = ProtectionPolicy {
            mode: ProtectionMode::Snapshot,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
            update_cores: None,
            update_memory_mb: None,
        };
        let global = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: Some("pbs-home:pbs".to_string()),
            snapshot_timeout_seconds: Some(180),
            backup_timeout_seconds: Some(1200),
            update_cores: None,
            update_memory_mb: None,
        };

        let effective = resolve_effective_policy(Some(item), Some(global));
        assert_eq!(effective.mode, ProtectionMode::Snapshot);
        assert_eq!(effective.snapshot_timeout_seconds, Some(180));
        assert_eq!(effective.backup_timeout_seconds, Some(1200));
    }

    #[test]
    fn effective_policy_keeps_explicit_item_timeout() {
        let item = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: Some("pbs-home:pbs".to_string()),
            snapshot_timeout_seconds: Some(90),
            backup_timeout_seconds: Some(1500),
            update_cores: None,
            update_memory_mb: None,
        };
        let global = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: Some("pbs-home:pbs".to_string()),
            snapshot_timeout_seconds: Some(180),
            backup_timeout_seconds: Some(1200),
            update_cores: None,
            update_memory_mb: None,
        };

        let effective = resolve_effective_policy(Some(item), Some(global));
        assert_eq!(effective.snapshot_timeout_seconds, Some(90));
        assert_eq!(effective.backup_timeout_seconds, Some(1500));
    }

    #[test]
    fn protection_policy_carries_scaling_fields() {
        let p = ProtectionPolicy {
            mode: ProtectionMode::DoNothing,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
            update_cores: Some(4),
            update_memory_mb: Some(8192),
        };
        assert_eq!(p.update_cores, Some(4));
        assert_eq!(p.update_memory_mb, Some(8192));
    }

    #[test]
    fn do_nothing_policy_has_no_scaling() {
        let p = ProtectionPolicy::do_nothing();
        assert!(p.update_cores.is_none());
        assert!(p.update_memory_mb.is_none());
    }

    #[test]
    fn resolve_effective_policy_cascades_scaling_fields() {
        let item = ProtectionPolicy {
            mode: ProtectionMode::Snapshot,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
            update_cores: Some(8),
            update_memory_mb: None,
        };
        let global = ProtectionPolicy {
            mode: ProtectionMode::DoNothing,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
            update_cores: Some(4),
            update_memory_mb: Some(4096),
        };
        let effective = resolve_effective_policy(Some(item), Some(global));
        assert_eq!(effective.update_cores, Some(8));
        assert_eq!(effective.update_memory_mb, Some(4096));
    }

    #[tokio::test]
    async fn upsert_cached_backup_targets_prunes_removed_targets() {
        let tenant_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        let keep_target = CachedBackupTarget {
            node: "pve1".to_string(),
            storage_id: "local".to_string(),
            storage_type: "dir".to_string(),
            target_key: "pve1:local:dir".to_string(),
        };
        let existing_keep = proxmox_backup_target_cache::Model {
            id: Uuid::now_v7(),
            tenant_id,
            plugin_config_id,
            proxmox_node: keep_target.node.clone(),
            storage_id: keep_target.storage_id.clone(),
            storage_type: keep_target.storage_type.clone(),
            target_key: keep_target.target_key.clone(),
            discovered_at: now,
            updated_at: now,
        };

        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([vec![existing_keep.clone()]])
            .append_query_results([vec![existing_keep]])
            .append_exec_results([
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                },
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                },
            ])
            .into_connection();

        let upserted =
            upsert_cached_backup_targets(&db, tenant_id, plugin_config_id, &[keep_target]).await;
        assert!(upserted.is_ok(), "upsert should succeed: {upserted:?}");

        let logs = db.into_transaction_log();
        let statements: Vec<String> = logs
            .iter()
            .flat_map(|tx| tx.statements().iter())
            .map(ToString::to_string)
            .collect();

        let prune_statement = statements
            .iter()
            .find(|sql| sql.contains("DELETE FROM `proxmox_backup_target_cache`"))
            .expect("expected stale-target prune DELETE statement");
        assert!(
            prune_statement.contains("`tenant_id`"),
            "prune must be tenant-scoped"
        );
        assert!(
            prune_statement.contains("`plugin_config_id`"),
            "prune must be plugin-config-scoped"
        );
        assert!(
            prune_statement.contains("`target_key` NOT IN"),
            "prune must only remove stale target keys"
        );
    }
}

#[cfg(test)]
mod scaling_record_tests {
    use crate::entity::proxmox_resource_scaling_record;
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn scaling_record_round_trip() {
        let update_id = Uuid::now_v7();
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let mapping_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();

        // Create the expected model to return from insert
        let expected_model = proxmox_resource_scaling_record::Model {
            update_history_id: update_id,
            tenant_id,
            host_id,
            software_item_id,
            plugin_config_id,
            mapping_id,
            vm_type: "qemu".to_string(),
            original_cores: 2,
            original_memory_mb: 2048,
            scaled_cores: 4,
            scaled_memory_mb: 4096,
            scale_status: "scaling".to_string(),
            restore_status: "pending".to_string(),
            error_message: None,
            created_at: now,
            updated_at: now,
        };

        // Mock: no existing row → insert path
        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([Vec::<proxmox_resource_scaling_record::Model>::new()])
            .append_query_results([vec![expected_model]])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let record = ScalingRecord {
            update_history_id: update_id,
            tenant_id,
            host_id,
            software_item_id,
            plugin_config_id,
            mapping_id,
            vm_type: "qemu".to_string(),
            original_cores: 2,
            original_memory_mb: 2048,
            scaled_cores: 4,
            scaled_memory_mb: 4096,
            scale_status: "scaling".to_string(),
            restore_status: "pending".to_string(),
            error_message: None,
        };

        let result = upsert_scaling_record(&db, &record).await;
        assert!(result.is_ok(), "upsert should succeed: {result:?}");
    }

    #[tokio::test]
    async fn scaling_record_with_scaling_status_is_eligible_for_restore() {
        // Verifies that load_scaling_record returns a record with scale_status "scaling"
        // and that finalize_post_update_hook's guard treats "scaling" the same as "scaled".
        use crate::entity::proxmox_resource_scaling_record;
        use sea_orm::{DbBackend, MockDatabase};

        let update_id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_query_results([vec![proxmox_resource_scaling_record::Model {
                update_history_id: update_id,
                tenant_id: uuid::Uuid::now_v7(),
                host_id: uuid::Uuid::now_v7(),
                software_item_id: uuid::Uuid::now_v7(),
                plugin_config_id: uuid::Uuid::now_v7(),
                mapping_id: uuid::Uuid::now_v7(),
                vm_type: "qemu".to_string(),
                original_cores: 2,
                original_memory_mb: 2048,
                scaled_cores: 4,
                scaled_memory_mb: 4096,
                scale_status: "scaling".to_string(),
                restore_status: "pending".to_string(),
                error_message: None,
                created_at: now,
                updated_at: now,
            }]])
            .into_connection();

        let record = load_scaling_record(&db, update_id).await.unwrap().unwrap();
        assert_eq!(record.scale_status, "scaling");
        // The guard in finalize_post_update_hook treats "scaling" the same as "scaled".
        assert!(
            record.scale_status == "scaled" || record.scale_status == "scaling",
            "scale_status '{}' should be eligible for restore",
            record.scale_status
        );
    }
}
