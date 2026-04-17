//! Controller-side policy, cache, and audit storage for Proxmox update protection.

use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{ProxmoxError, Result};

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
}

impl ProtectionPolicy {
    pub fn do_nothing() -> Self {
        Self {
            mode: ProtectionMode::DoNothing,
            backup_target_key: None,
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
pub fn resolve_effective_policy(
    item_override: Option<ProtectionPolicy>,
    global_default: Option<ProtectionPolicy>,
) -> ProtectionPolicy {
    item_override
        .or(global_default)
        .unwrap_or_else(ProtectionPolicy::do_nothing)
}

/// Load effective policy for `(software_item_id, plugin_config_id)`.
pub async fn load_effective_policy(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ProtectionPolicy> {
    let item_override = item_override::Entity::find()
        .filter(item_override::Column::SoftwareItemId.eq(software_item_id))
        .filter(item_override::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to load per-item protection override: {e}"
            )))
        })?
        .map(|row| ProtectionPolicy {
            mode: ProtectionMode::from_db(&row.mode),
            backup_target_key: row.backup_target_key,
        });

    let global_default = global_default::Entity::find()
        .filter(global_default::Column::TenantId.eq(tenant_id))
        .filter(global_default::Column::PluginConfigId.eq(plugin_config_id))
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to load global protection defaults: {e}"
            )))
        })?
        .map(|row| ProtectionPolicy {
            mode: ProtectionMode::from_db(&row.mode),
            backup_target_key: row.backup_target_key,
        });

    Ok(resolve_effective_policy(item_override, global_default))
}

/// Upsert cached backup targets for a Proxmox config.
pub async fn upsert_cached_backup_targets(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    targets: &[CachedBackupTarget],
) -> Result<usize> {
    let now = OffsetDateTime::now_utc();
    let mut upserted = 0usize;

    for target in targets {
        let existing = backup_target_cache::Entity::find()
            .filter(backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
            .filter(backup_target_cache::Column::TargetKey.eq(&target.target_key))
            .one(db)
            .await
            .map_err(|e| {
                rootcause::report!(ProxmoxError::Database(format!(
                    "failed to query backup target cache: {e}"
                )))
            })?;

        if let Some(existing) = existing {
            let mut active: backup_target_cache::ActiveModel = existing.into();
            active.tenant_id = Set(tenant_id);
            active.proxmox_node = Set(target.node.clone());
            active.storage_id = Set(target.storage_id.clone());
            active.storage_type = Set(target.storage_type.clone());
            active.updated_at = Set(now);
            active.update(db).await.map_err(|e| {
                rootcause::report!(ProxmoxError::Database(format!(
                    "failed to update backup target cache: {e}"
                )))
            })?;
        } else {
            let active = backup_target_cache::ActiveModel {
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
            active.insert(db).await.map_err(|e| {
                rootcause::report!(ProxmoxError::Database(format!(
                    "failed to insert backup target cache: {e}"
                )))
            })?;
        }

        upserted += 1;
    }

    Ok(upserted)
}

/// Look up one cached backup target by key.
pub async fn find_cached_backup_target(
    db: &DatabaseConnection,
    plugin_config_id: Uuid,
    target_key: &str,
) -> Result<Option<CachedBackupTarget>> {
    let row = backup_target_cache::Entity::find()
        .filter(backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
        .filter(backup_target_cache::Column::TargetKey.eq(target_key))
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

/// Load existing audit row for idempotency checks.
pub async fn load_protection_audit(
    db: &DatabaseConnection,
    update_history_id: Uuid,
) -> Result<Option<ProtectionAudit>> {
    let row = protection_audit::Entity::find_by_id(update_history_id)
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
    let existing = protection_audit::Entity::find_by_id(audit.update_history_id)
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query existing protection audit row: {e}"
            )))
        })?;

    if let Some(existing) = existing {
        let mut active: protection_audit::ActiveModel = existing.into();
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
        let active = protection_audit::ActiveModel {
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

// ── Local SeaORM models (Proxmox-owned tables) ─────────────────────────────

pub mod global_default {
    use sea_orm::entity::prelude::*;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "proxmox_protection_defaults")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub plugin_config_id: Uuid,
        pub mode: String,
        pub backup_target_key: Option<String>,
        pub created_at: OffsetDateTime,
        pub updated_at: OffsetDateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum Relation {}

    impl RelationTrait for Relation {
        fn def(&self) -> RelationDef {
            panic!("no relations")
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod item_override {
    use sea_orm::entity::prelude::*;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "proxmox_protection_item_overrides")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub software_item_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub plugin_config_id: Uuid,
        pub mode: String,
        pub backup_target_key: Option<String>,
        pub created_at: OffsetDateTime,
        pub updated_at: OffsetDateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum Relation {}

    impl RelationTrait for Relation {
        fn def(&self) -> RelationDef {
            panic!("no relations")
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod backup_target_cache {
    use sea_orm::entity::prelude::*;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "proxmox_backup_target_cache")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub plugin_config_id: Uuid,
        pub proxmox_node: String,
        pub storage_id: String,
        pub storage_type: String,
        pub target_key: String,
        pub discovered_at: OffsetDateTime,
        pub updated_at: OffsetDateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum Relation {}

    impl RelationTrait for Relation {
        fn def(&self) -> RelationDef {
            panic!("no relations")
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod protection_audit {
    use sea_orm::entity::prelude::*;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "proxmox_protection_audit")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub update_history_id: Uuid,
        pub tenant_id: Uuid,
        pub host_id: Uuid,
        pub software_item_id: Uuid,
        pub plugin_config_id: Uuid,
        pub mapping_id: Option<Uuid>,
        pub mode: String,
        pub status: String,
        pub artifact_kind: Option<String>,
        pub artifact_ref: Option<String>,
        pub backup_target_key: Option<String>,
        pub detail: Option<String>,
        pub error_message: Option<String>,
        pub created_at: OffsetDateTime,
        pub updated_at: OffsetDateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum Relation {}

    impl RelationTrait for Relation {
        fn def(&self) -> RelationDef {
            panic!("no relations")
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let global = ProtectionPolicy {
            mode: ProtectionMode::Snapshot,
            backup_target_key: None,
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
}
