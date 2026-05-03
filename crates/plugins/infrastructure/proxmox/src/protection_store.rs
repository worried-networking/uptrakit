//! DB-backed Proxmox protection store — moved from `plugin-infrastructure-core`.
//!
//! This module houses the data types and DB implementation for the Proxmox
//! update-protection persistence boundary. The trait and its implementation
//! are `pub(crate)` — they are not part of the public API of this crate.

use async_trait::async_trait;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_core::error::{PluginError, Result};
use uptrakit_shared_db::entity::{
    plugin_config, prelude::*, proxmox_backup_target_cache, proxmox_host_mapping,
    proxmox_protection_audit, proxmox_protection_default, proxmox_protection_item_override,
};
use uuid::Uuid;

const PROXMOX_INFRA_CONFIG_TYPE: &str = "infrastructure_proxmox";

// ── Data types (moved from plugin-infrastructure-core) ────────────────────────

/// Typed Proxmox host mapping required by update-protection workflows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProxmoxHostMappingRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Option<Uuid>,
    pub plugin_config_id: Uuid,
    pub proxmox_node: String,
    pub proxmox_vmid: i64,
    pub proxmox_type: String,
}

/// Typed protection mode for Proxmox controller protection workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ProxmoxProtectionMode {
    #[default]
    DoNothing,
    Snapshot,
    Backup,
}

/// Typed effective protection policy used during pre-update planning.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ProxmoxProtectionPolicyRecord {
    pub mode: ProxmoxProtectionMode,
    pub backup_target_key: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
}

/// Typed persisted audit row used by Proxmox protection reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProxmoxProtectionAuditRecord {
    pub update_history_id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    pub mapping_id: Option<Uuid>,
    pub mode: ProxmoxProtectionMode,
    pub status: String,
    pub artifact_kind: Option<String>,
    pub artifact_ref: Option<String>,
    pub backup_target_key: Option<String>,
    pub detail: Option<String>,
    pub error_message: Option<String>,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// DB persistence boundary for Proxmox update-protection workflows.
#[async_trait]
pub(crate) trait ProxmoxProtectionStore: Send + Sync {
    /// Load the Proxmox host mapping for a tenant/host pair.
    async fn load_host_mapping(
        &self,
        tenant_id: Uuid,
        host_id: Uuid,
    ) -> Result<Option<ProxmoxHostMappingRecord>>;

    /// Load raw Proxmox plugin config JSON for a tenant-scoped plugin config row.
    async fn load_plugin_config_payload(
        &self,
        tenant_id: Uuid,
        plugin_config_id: Uuid,
    ) -> Result<serde_json::Value>;

    /// Load the effective update-protection policy for a software item.
    async fn load_effective_policy(
        &self,
        tenant_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
    ) -> Result<ProxmoxProtectionPolicyRecord>;

    /// Load persisted protection audit state for a dispatch row.
    async fn load_audit(
        &self,
        update_history_id: Uuid,
    ) -> Result<Option<ProxmoxProtectionAuditRecord>>;

    /// Upsert protection audit state for a dispatch row.
    async fn upsert_audit(&self, audit: &ProxmoxProtectionAuditRecord) -> Result<()>;

    /// Resolve a cached backup target by plugin config and logical key.
    async fn find_cached_backup_target(
        &self,
        plugin_config_id: Uuid,
        target_key: &str,
    ) -> Result<Option<String>>;
}

// ── Helper functions ──────────────────────────────────────────────────────────

pub(crate) fn plugin_internal_error(
    error: impl std::fmt::Display,
) -> rootcause::Report<PluginError> {
    rootcause::report!(PluginError::PluginInternal(error.to_string()))
}

pub(crate) fn proxmox_mode_from_db(value: &str) -> ProxmoxProtectionMode {
    match value {
        "snapshot" => ProxmoxProtectionMode::Snapshot,
        "backup" => ProxmoxProtectionMode::Backup,
        _ => ProxmoxProtectionMode::DoNothing,
    }
}

pub(crate) fn proxmox_mode_to_db(value: ProxmoxProtectionMode) -> &'static str {
    match value {
        ProxmoxProtectionMode::DoNothing => "do_nothing",
        ProxmoxProtectionMode::Snapshot => "snapshot",
        ProxmoxProtectionMode::Backup => "backup",
    }
}

// ── DB implementation ─────────────────────────────────────────────────────────

/// DB-backed implementation of [`ProxmoxProtectionStore`].
pub(crate) struct DbProxmoxProtectionStore<'a> {
    pub db: &'a DatabaseConnection,
}

#[async_trait]
impl ProxmoxProtectionStore for DbProxmoxProtectionStore<'_> {
    async fn load_host_mapping(
        &self,
        tenant_id: Uuid,
        host_id: Uuid,
    ) -> Result<Option<ProxmoxHostMappingRecord>> {
        let mut mappings = ProxmoxHostMapping::find()
            .filter(proxmox_host_mapping::Column::TenantId.eq(tenant_id))
            .filter(proxmox_host_mapping::Column::HostId.eq(Some(host_id)))
            .all(self.db)
            .await
            .map_err(plugin_internal_error)?;

        if mappings.len() > 1 {
            return Err(plugin_internal_error(format!(
                "multiple proxmox host mappings found for tenant={tenant_id}, host_id={host_id}"
            )));
        }

        Ok(mappings.pop().map(|row| ProxmoxHostMappingRecord {
            id: row.id,
            tenant_id: row.tenant_id,
            host_id: row.host_id,
            plugin_config_id: row.plugin_config_id,
            proxmox_node: row.proxmox_node,
            proxmox_vmid: i64::from(row.proxmox_vmid),
            proxmox_type: row.proxmox_type,
        }))
    }

    async fn load_plugin_config_payload(
        &self,
        tenant_id: Uuid,
        plugin_config_id: Uuid,
    ) -> Result<serde_json::Value> {
        let config = PluginConfig::find_by_id(plugin_config_id)
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq(PROXMOX_INFRA_CONFIG_TYPE))
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?
            .ok_or_else(|| {
                plugin_internal_error(format!(
                    "proxmox plugin config not found for tenant={tenant_id}, plugin_config_id={plugin_config_id}"
                ))
            })?;

        if config.plugin_type != PROXMOX_INFRA_CONFIG_TYPE {
            return Err(plugin_internal_error(format!(
                "plugin config {plugin_config_id} is not an {PROXMOX_INFRA_CONFIG_TYPE} config"
            )));
        }

        Ok(config.config)
    }

    async fn load_effective_policy(
        &self,
        tenant_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
    ) -> Result<ProxmoxProtectionPolicyRecord> {
        let item_override = ProxmoxProtectionItemOverride::find()
            .filter(proxmox_protection_item_override::Column::SoftwareItemId.eq(software_item_id))
            .filter(proxmox_protection_item_override::Column::PluginConfigId.eq(plugin_config_id))
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        let global_default = ProxmoxProtectionDefault::find()
            .filter(proxmox_protection_default::Column::TenantId.eq(tenant_id))
            .filter(proxmox_protection_default::Column::PluginConfigId.eq(plugin_config_id))
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        let item_mode = item_override
            .as_ref()
            .map(|row| proxmox_mode_from_db(&row.mode));
        let global_mode = global_default
            .as_ref()
            .map(|row| proxmox_mode_from_db(&row.mode));

        let snapshot_timeout_seconds = item_override
            .as_ref()
            .and_then(|row| row.snapshot_timeout_seconds)
            .or_else(|| {
                global_default
                    .as_ref()
                    .and_then(|row| row.snapshot_timeout_seconds)
            });

        let backup_timeout_seconds = item_override
            .as_ref()
            .and_then(|row| row.backup_timeout_seconds)
            .or_else(|| {
                global_default
                    .as_ref()
                    .and_then(|row| row.backup_timeout_seconds)
            });

        Ok(ProxmoxProtectionPolicyRecord {
            mode: item_mode
                .or(global_mode)
                .unwrap_or(ProxmoxProtectionMode::DoNothing),
            backup_target_key: item_override
                .as_ref()
                .and_then(|row| row.backup_target_key.clone())
                .or_else(|| {
                    global_default
                        .as_ref()
                        .and_then(|row| row.backup_target_key.clone())
                }),
            snapshot_timeout_seconds,
            backup_timeout_seconds,
        })
    }

    async fn load_audit(
        &self,
        update_history_id: Uuid,
    ) -> Result<Option<ProxmoxProtectionAuditRecord>> {
        let row = ProxmoxProtectionAudit::find_by_id(update_history_id)
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        Ok(row.map(|row| ProxmoxProtectionAuditRecord {
            update_history_id: row.update_history_id,
            tenant_id: row.tenant_id,
            host_id: row.host_id,
            software_item_id: row.software_item_id,
            plugin_config_id: row.plugin_config_id,
            mapping_id: row.mapping_id,
            mode: proxmox_mode_from_db(&row.mode),
            status: row.status,
            artifact_kind: row.artifact_kind,
            artifact_ref: row.artifact_ref,
            backup_target_key: row.backup_target_key,
            detail: row.detail,
            error_message: row.error_message,
        }))
    }

    async fn upsert_audit(&self, audit: &ProxmoxProtectionAuditRecord) -> Result<()> {
        let now = OffsetDateTime::now_utc();
        let existing = ProxmoxProtectionAudit::find_by_id(audit.update_history_id)
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        if let Some(existing) = existing {
            let mut active: proxmox_protection_audit::ActiveModel = existing.into();
            active.tenant_id = Set(audit.tenant_id);
            active.host_id = Set(audit.host_id);
            active.software_item_id = Set(audit.software_item_id);
            active.plugin_config_id = Set(audit.plugin_config_id);
            active.mapping_id = Set(audit.mapping_id);
            active.mode = Set(proxmox_mode_to_db(audit.mode).to_string());
            active.status = Set(audit.status.clone());
            active.artifact_kind = Set(audit.artifact_kind.clone());
            active.artifact_ref = Set(audit.artifact_ref.clone());
            active.backup_target_key = Set(audit.backup_target_key.clone());
            active.detail = Set(audit.detail.clone());
            active.error_message = Set(audit.error_message.clone());
            active.updated_at = Set(now);
            active
                .update(self.db)
                .await
                .map_err(plugin_internal_error)?;
        } else {
            let active = proxmox_protection_audit::ActiveModel {
                update_history_id: Set(audit.update_history_id),
                tenant_id: Set(audit.tenant_id),
                host_id: Set(audit.host_id),
                software_item_id: Set(audit.software_item_id),
                plugin_config_id: Set(audit.plugin_config_id),
                mapping_id: Set(audit.mapping_id),
                mode: Set(proxmox_mode_to_db(audit.mode).to_string()),
                status: Set(audit.status.clone()),
                artifact_kind: Set(audit.artifact_kind.clone()),
                artifact_ref: Set(audit.artifact_ref.clone()),
                backup_target_key: Set(audit.backup_target_key.clone()),
                detail: Set(audit.detail.clone()),
                error_message: Set(audit.error_message.clone()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            active
                .insert(self.db)
                .await
                .map_err(plugin_internal_error)?;
        }

        Ok(())
    }

    async fn find_cached_backup_target(
        &self,
        plugin_config_id: Uuid,
        target_key: &str,
    ) -> Result<Option<String>> {
        let row = ProxmoxBackupTargetCache::find()
            .filter(proxmox_backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
            .filter(proxmox_backup_target_cache::Column::TargetKey.eq(target_key))
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        Ok(row.map(|row| row.storage_id))
    }
}
