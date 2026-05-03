//! Controller-side policy, cache, and audit storage for Proxmox update protection.

use crate::entity::{
    proxmox_backup_target_cache, proxmox_protection_audit, proxmox_protection_default,
    proxmox_protection_item_override,
};
use proxmox_backup_target_cache::Entity as ProxmoxBackupTargetCache;
use proxmox_protection_audit::Entity as ProxmoxProtectionAudit;
use proxmox_protection_default::Entity as ProxmoxProtectionDefault;
use proxmox_protection_item_override::Entity as ProxmoxProtectionItemOverride;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
    TransactionTrait,
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
}

impl ProtectionPolicy {
    pub fn do_nothing() -> Self {
        Self {
            mode: ProtectionMode::DoNothing,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
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

    ProtectionPolicy {
        mode,
        backup_target_key,
        snapshot_timeout_seconds,
        backup_timeout_seconds,
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
        };
        let global = ProtectionPolicy {
            mode: ProtectionMode::Snapshot,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
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
        };
        let global = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: Some("pbs-home:pbs".to_string()),
            snapshot_timeout_seconds: Some(180),
            backup_timeout_seconds: Some(1200),
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
        };
        let global = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: Some("pbs-home:pbs".to_string()),
            snapshot_timeout_seconds: Some(180),
            backup_timeout_seconds: Some(1200),
        };

        let effective = resolve_effective_policy(Some(item), Some(global));
        assert_eq!(effective.snapshot_timeout_seconds, Some(90));
        assert_eq!(effective.backup_timeout_seconds, Some(1500));
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
