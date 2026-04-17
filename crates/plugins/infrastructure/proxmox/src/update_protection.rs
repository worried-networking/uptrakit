//! Controller-side pre/post update protection for Proxmox-backed hosts.

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uptrakit_plugin_infrastructure_core::error::{PluginError, Result};
use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ControllerPostUpdateContext, ControllerProtectionContext,
    ControllerProtectionDecision, ControllerUpdateProtection, PluginMeta, PluginTypeId,
    PostUpdateOutcome,
};
use uptrakit_shared_db::entity::{plugin_config, proxmox_host_mapping};
use uuid::Uuid;

use crate::client::ProxmoxClient;
use crate::config::ProxmoxConfig;
use crate::policy_store::{
    ProtectionAudit, ProtectionMode, ProtectionPolicy, find_cached_backup_target,
    load_effective_policy, load_protection_audit, upsert_protection_audit,
};

const PROTECTION_WAIT_TIMEOUT: Duration = Duration::from_secs(120);
const STATUS_PROTECTED: &str = "protected";
const STATUS_FAILED: &str = "failed";
const SUMMARY_SKIPPED: &str = "No controller pre-update protection applied.";
const SUMMARY_SUCCESS: &str = "Controller pre-update protection prepared.";
const SUMMARY_REUSED: &str = "Controller pre-update protection reused.";
const SUMMARY_FAILURE: &str = "Controller pre-update protection failed.";

/// Controller singleton implementing Proxmox update protection workflow.
pub struct ControllerUpdateProtectionPlugin;

impl ControllerUpdateProtectionPlugin {
    pub fn create(_config: &CatalogConfig) -> Result<Arc<dyn ControllerUpdateProtection>> {
        Ok(Arc::new(Self))
    }
}

#[async_trait::async_trait]
impl ControllerUpdateProtection for ControllerUpdateProtectionPlugin {
    async fn prepare_pre_update_protection(
        &self,
        ctx: &ControllerProtectionContext<'_>,
    ) -> Result<ControllerProtectionDecision> {
        let db = db_from_any(ctx.db)?;

        let mapping = match load_unique_mapping(db, ctx.tenant_id, ctx.host_id).await? {
            Some(mapping) => mapping,
            None => {
                return Ok(ControllerProtectionDecision::skipped(Some(
                    SUMMARY_SKIPPED.to_string(),
                )));
            }
        };

        let proxmox_cfg = load_proxmox_config(db, ctx.tenant_id, mapping.plugin_config_id).await?;
        let policy = load_effective_policy(
            db,
            ctx.tenant_id,
            ctx.software_item_id,
            mapping.plugin_config_id,
        )
        .await
        .map_err(plugin_internal)?;

        if let Some(existing) = load_protection_audit(db, ctx.update_history_id)
            .await
            .map_err(plugin_internal)?
            && is_reusable_success(&existing, &policy)
        {
            let decision = match policy.mode {
                ProtectionMode::DoNothing => {
                    ControllerProtectionDecision::skipped(Some(SUMMARY_REUSED.to_string()))
                }
                _ => ControllerProtectionDecision::success(
                    Some(STATUS_PROTECTED.to_string()),
                    Some(SUMMARY_REUSED.to_string()),
                ),
            };
            return Ok(decision);
        }

        match policy.mode {
            ProtectionMode::DoNothing => {
                let audit = ProtectionAudit {
                    update_history_id: ctx.update_history_id,
                    tenant_id: ctx.tenant_id,
                    host_id: ctx.host_id,
                    software_item_id: ctx.software_item_id,
                    plugin_config_id: mapping.plugin_config_id,
                    mapping_id: Some(mapping.id),
                    mode: ProtectionMode::DoNothing,
                    status: "skipped".to_string(),
                    artifact_kind: None,
                    artifact_ref: None,
                    backup_target_key: None,
                    detail: Some(SUMMARY_SKIPPED.to_string()),
                    error_message: None,
                };
                upsert_protection_audit(db, &audit)
                    .await
                    .map_err(plugin_internal)?;
                Ok(ControllerProtectionDecision::skipped(Some(
                    SUMMARY_SKIPPED.to_string(),
                )))
            }
            ProtectionMode::Snapshot => {
                prepare_snapshot_protection(db, ctx, &mapping, &proxmox_cfg).await
            }
            ProtectionMode::Backup => {
                prepare_backup_protection(db, ctx, &mapping, &proxmox_cfg, &policy).await
            }
        }
    }

    async fn finalize_post_update(
        &self,
        ctx: &ControllerPostUpdateContext<'_>,
    ) -> Result<PostUpdateOutcome> {
        let db = db_from_any(ctx.db)?;
        let audit = load_protection_audit(db, ctx.update_history_id)
            .await
            .map_err(plugin_internal)?;

        if ctx.final_status == uptrakit_shared_types::UpdateStatus::Failed
            && audit
                .as_ref()
                .is_some_and(|row| row.status == "succeeded" && row.artifact_kind.is_some())
        {
            return Ok(PostUpdateOutcome::new(Some(
                "Pre-update protection artifact exists and can be used for manual recovery."
                    .to_string(),
            )));
        }

        Ok(PostUpdateOutcome::default())
    }
}

impl PluginMeta for ControllerUpdateProtectionPlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::from_static("infrastructure_proxmox")
    }
}

fn snapshot_decision_failure() -> ControllerProtectionDecision {
    ControllerProtectionDecision::failure(
        Some(STATUS_FAILED.to_string()),
        Some(SUMMARY_FAILURE.to_string()),
    )
}

fn db_from_any(db: &(dyn std::any::Any + Send + Sync)) -> Result<&DatabaseConnection> {
    db.downcast_ref::<DatabaseConnection>().ok_or_else(|| {
        report!(PluginError::PluginInternal(
            "ControllerUpdateProtection expected a DatabaseConnection context".to_string()
        ))
    })
}

async fn load_unique_mapping(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
) -> Result<Option<proxmox_host_mapping::Model>> {
    let mappings = proxmox_host_mapping::Entity::find()
        .filter(proxmox_host_mapping::Column::TenantId.eq(tenant_id))
        .filter(proxmox_host_mapping::Column::HostId.eq(host_id))
        .all(db)
        .await
        .map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to query Proxmox host mapping: {e}"
            )))
        })?;

    if mappings.is_empty() {
        return Ok(None);
    }
    if mappings.len() > 1 {
        return Err(report!(PluginError::Configuration(
            "invalid Proxmox mapping configuration for host".to_string()
        )));
    }

    Ok(mappings.into_iter().next())
}

async fn load_proxmox_config(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ProxmoxConfig> {
    let row = plugin_config::Entity::find_by_id(plugin_config_id)
        .one(db)
        .await
        .map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to query Proxmox plugin config: {e}"
            )))
        })?
        .ok_or_else(|| {
            report!(PluginError::Configuration(
                "missing Proxmox plugin configuration".to_string()
            ))
        })?;

    if row.tenant_id != tenant_id || row.plugin_type != "infrastructure_proxmox" {
        return Err(report!(PluginError::Configuration(
            "invalid Proxmox plugin configuration".to_string()
        )));
    }

    serde_json::from_value(row.config).map_err(|e| {
        report!(PluginError::Configuration(format!(
            "invalid Proxmox plugin configuration payload: {e}"
        )))
    })
}

fn is_reusable_success(audit: &ProtectionAudit, policy: &ProtectionPolicy) -> bool {
    if audit.status != "succeeded" && audit.status != "skipped" {
        return false;
    }
    if audit.mode != policy.mode {
        return false;
    }
    match policy.mode {
        ProtectionMode::DoNothing => audit.status == "skipped",
        ProtectionMode::Snapshot | ProtectionMode::Backup => audit.artifact_ref.is_some(),
    }
}

async fn prepare_snapshot_protection(
    db: &DatabaseConnection,
    ctx: &ControllerProtectionContext<'_>,
    mapping: &proxmox_host_mapping::Model,
    proxmox_cfg: &ProxmoxConfig,
) -> Result<ControllerProtectionDecision> {
    let client = ProxmoxClient::new(proxmox_cfg).map_err(plugin_internal)?;
    let snapshot_name = snapshot_name_for_update_history(ctx.update_history_id);

    let task = if mapping.proxmox_type.eq_ignore_ascii_case("lxc") {
        client
            .create_lxc_snapshot(
                &mapping.proxmox_node,
                mapping.proxmox_vmid as u32,
                &snapshot_name,
            )
            .await
    } else {
        client
            .create_qemu_snapshot(
                &mapping.proxmox_node,
                mapping.proxmox_vmid as u32,
                &snapshot_name,
            )
            .await
    };

    let task = match task {
        Ok(task) => task,
        Err(error) => {
            let audit = ProtectionAudit {
                update_history_id: ctx.update_history_id,
                tenant_id: ctx.tenant_id,
                host_id: ctx.host_id,
                software_item_id: ctx.software_item_id,
                plugin_config_id: mapping.plugin_config_id,
                mapping_id: Some(mapping.id),
                mode: ProtectionMode::Snapshot,
                status: "failed".to_string(),
                artifact_kind: Some("snapshot".to_string()),
                artifact_ref: None,
                backup_target_key: None,
                detail: Some(SUMMARY_FAILURE.to_string()),
                error_message: Some(error.to_string()),
            };
            upsert_protection_audit(db, &audit)
                .await
                .map_err(plugin_internal)?;
            return Ok(snapshot_decision_failure());
        }
    };

    if let Err(error) = client
        .wait_for_task_completion(&mapping.proxmox_node, &task, PROTECTION_WAIT_TIMEOUT)
        .await
    {
        let audit = ProtectionAudit {
            update_history_id: ctx.update_history_id,
            tenant_id: ctx.tenant_id,
            host_id: ctx.host_id,
            software_item_id: ctx.software_item_id,
            plugin_config_id: mapping.plugin_config_id,
            mapping_id: Some(mapping.id),
            mode: ProtectionMode::Snapshot,
            status: "failed".to_string(),
            artifact_kind: Some("snapshot".to_string()),
            artifact_ref: Some(snapshot_name),
            backup_target_key: None,
            detail: Some(SUMMARY_FAILURE.to_string()),
            error_message: Some(error.to_string()),
        };
        upsert_protection_audit(db, &audit)
            .await
            .map_err(plugin_internal)?;
        return Ok(snapshot_decision_failure());
    }

    let audit = ProtectionAudit {
        update_history_id: ctx.update_history_id,
        tenant_id: ctx.tenant_id,
        host_id: ctx.host_id,
        software_item_id: ctx.software_item_id,
        plugin_config_id: mapping.plugin_config_id,
        mapping_id: Some(mapping.id),
        mode: ProtectionMode::Snapshot,
        status: "succeeded".to_string(),
        artifact_kind: Some("snapshot".to_string()),
        artifact_ref: Some(snapshot_name),
        backup_target_key: None,
        detail: Some(SUMMARY_SUCCESS.to_string()),
        error_message: None,
    };
    upsert_protection_audit(db, &audit)
        .await
        .map_err(plugin_internal)?;

    Ok(ControllerProtectionDecision::success(
        Some(STATUS_PROTECTED.to_string()),
        Some(SUMMARY_SUCCESS.to_string()),
    ))
}

async fn prepare_backup_protection(
    db: &DatabaseConnection,
    ctx: &ControllerProtectionContext<'_>,
    mapping: &proxmox_host_mapping::Model,
    proxmox_cfg: &ProxmoxConfig,
    policy: &ProtectionPolicy,
) -> Result<ControllerProtectionDecision> {
    let Some(target_key) = policy.backup_target_key.as_deref() else {
        return Ok(snapshot_decision_failure());
    };

    let Some(target) = find_cached_backup_target(db, mapping.plugin_config_id, target_key)
        .await
        .map_err(plugin_internal)?
    else {
        return Ok(snapshot_decision_failure());
    };

    let client = ProxmoxClient::new(proxmox_cfg).map_err(plugin_internal)?;

    let task = match client
        .start_backup(
            &mapping.proxmox_node,
            mapping.proxmox_vmid as u32,
            &mapping.proxmox_type,
            &target.storage_id,
        )
        .await
    {
        Ok(task) => task,
        Err(error) => {
            let audit = ProtectionAudit {
                update_history_id: ctx.update_history_id,
                tenant_id: ctx.tenant_id,
                host_id: ctx.host_id,
                software_item_id: ctx.software_item_id,
                plugin_config_id: mapping.plugin_config_id,
                mapping_id: Some(mapping.id),
                mode: ProtectionMode::Backup,
                status: "failed".to_string(),
                artifact_kind: Some("backup".to_string()),
                artifact_ref: None,
                backup_target_key: Some(target_key.to_string()),
                detail: Some(SUMMARY_FAILURE.to_string()),
                error_message: Some(error.to_string()),
            };
            upsert_protection_audit(db, &audit)
                .await
                .map_err(plugin_internal)?;
            return Ok(snapshot_decision_failure());
        }
    };

    if let Err(error) = client
        .wait_for_task_completion(&mapping.proxmox_node, &task, PROTECTION_WAIT_TIMEOUT)
        .await
    {
        let audit = ProtectionAudit {
            update_history_id: ctx.update_history_id,
            tenant_id: ctx.tenant_id,
            host_id: ctx.host_id,
            software_item_id: ctx.software_item_id,
            plugin_config_id: mapping.plugin_config_id,
            mapping_id: Some(mapping.id),
            mode: ProtectionMode::Backup,
            status: "failed".to_string(),
            artifact_kind: Some("backup".to_string()),
            artifact_ref: Some(task),
            backup_target_key: Some(target_key.to_string()),
            detail: Some(SUMMARY_FAILURE.to_string()),
            error_message: Some(error.to_string()),
        };
        upsert_protection_audit(db, &audit)
            .await
            .map_err(plugin_internal)?;
        return Ok(snapshot_decision_failure());
    }

    let audit = ProtectionAudit {
        update_history_id: ctx.update_history_id,
        tenant_id: ctx.tenant_id,
        host_id: ctx.host_id,
        software_item_id: ctx.software_item_id,
        plugin_config_id: mapping.plugin_config_id,
        mapping_id: Some(mapping.id),
        mode: ProtectionMode::Backup,
        status: "succeeded".to_string(),
        artifact_kind: Some("backup".to_string()),
        artifact_ref: Some(task),
        backup_target_key: Some(target_key.to_string()),
        detail: Some(SUMMARY_SUCCESS.to_string()),
        error_message: None,
    };
    upsert_protection_audit(db, &audit)
        .await
        .map_err(plugin_internal)?;

    Ok(ControllerProtectionDecision::success(
        Some(STATUS_PROTECTED.to_string()),
        Some(SUMMARY_SUCCESS.to_string()),
    ))
}

fn plugin_internal<E: std::fmt::Display>(error: E) -> Report<PluginError> {
    report!(PluginError::PluginInternal(error.to_string()))
}

/// Derive a deterministic Proxmox-safe snapshot name for an update-history row.
///
/// The output is ASCII-safe and at most 40 chars.
pub fn snapshot_name_for_update_history(update_history_id: Uuid) -> String {
    format!("utk-{}", update_history_id.simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_name_is_deterministic_and_safe() {
        let update_history_id =
            Uuid::parse_str("5bfb4e73-a0b4-4c81-b43d-b986589f6205").expect("valid uuid");
        let a = snapshot_name_for_update_history(update_history_id);
        let b = snapshot_name_for_update_history(update_history_id);

        assert_eq!(a, b);
        assert!(a.len() <= 40, "snapshot name must be <= 40 chars");
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "snapshot name must be Proxmox-safe"
        );
    }

    #[test]
    fn idempotency_helper_reuses_successful_artifact() {
        let policy = ProtectionPolicy {
            mode: ProtectionMode::Snapshot,
            backup_target_key: None,
        };
        let audit = ProtectionAudit {
            update_history_id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            host_id: Uuid::now_v7(),
            software_item_id: Uuid::now_v7(),
            plugin_config_id: Uuid::now_v7(),
            mapping_id: None,
            mode: ProtectionMode::Snapshot,
            status: "succeeded".to_string(),
            artifact_kind: Some("snapshot".to_string()),
            artifact_ref: Some("utk-abc".to_string()),
            backup_target_key: None,
            detail: None,
            error_message: None,
        };
        assert!(is_reusable_success(&audit, &policy));
    }

    #[test]
    fn idempotency_helper_rejects_mode_mismatch() {
        let policy = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: Some("node:local:dir".to_string()),
        };
        let audit = ProtectionAudit {
            update_history_id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            host_id: Uuid::now_v7(),
            software_item_id: Uuid::now_v7(),
            plugin_config_id: Uuid::now_v7(),
            mapping_id: None,
            mode: ProtectionMode::Snapshot,
            status: "succeeded".to_string(),
            artifact_kind: Some("snapshot".to_string()),
            artifact_ref: Some("utk-abc".to_string()),
            backup_target_key: None,
            detail: None,
            error_message: None,
        };
        assert!(!is_reusable_success(&audit, &policy));
    }
}
