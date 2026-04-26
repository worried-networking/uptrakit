//! Controller-side pre/post update protection for Proxmox-backed hosts.

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::error::{PluginError, Result};
use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ControllerPostUpdateContext, ControllerProtectionContext,
    ControllerProtectionDecision, ControllerUpdateProtection, PluginMeta, PluginTypeId,
    PostUpdateOutcome, ProxmoxHostMappingRecord, ProxmoxProtectionAuditRecord,
    ProxmoxProtectionMode, ProxmoxProtectionPolicyRecord, ProxmoxProtectionStore,
    UpdateProtectionController,
};
use uuid::Uuid;

use crate::client::ProxmoxClient;
use crate::config::ProxmoxConfig;
use crate::policy_store::{ProtectionAudit, ProtectionMode, ProtectionPolicy};

pub(crate) const DEFAULT_SNAPSHOT_TIMEOUT_SECONDS: i64 = 120;
pub(crate) const DEFAULT_BACKUP_TIMEOUT_SECONDS: i64 = 900;
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
    #[tracing::instrument(skip_all, fields(
        tenant_id = %ctx.tenant_id,
        host_id = %ctx.host_id,
        update_history_id = %ctx.update_history_id,
    ))]
    async fn prepare_pre_update_protection(
        &self,
        ctx: &ControllerProtectionContext<'_>,
    ) -> Result<ControllerProtectionDecision> {
        let store = proxmox_protection_store(ctx.controller)?;

        let mapping = match load_unique_mapping(store, ctx.tenant_id, ctx.host_id).await? {
            Some(mapping) => mapping,
            None => {
                tracing::debug!("no Proxmox host mapping found — skipping protection");
                return Ok(ControllerProtectionDecision::skipped(Some(
                    SUMMARY_SKIPPED.to_string(),
                )));
            }
        };

        tracing::debug!(
            node = %mapping.proxmox_node,
            vmid = mapping.proxmox_vmid,
            guest_type = %mapping.proxmox_type,
            "found Proxmox host mapping"
        );

        let proxmox_cfg =
            load_proxmox_config(store, ctx.tenant_id, mapping.plugin_config_id).await?;
        let policy = map_policy_record(
            store
                .load_effective_policy(
                    ctx.tenant_id,
                    ctx.software_item_id,
                    mapping.plugin_config_id,
                )
                .await
                .map_err(plugin_internal)?,
        );

        tracing::debug!(
            mode = policy.mode.as_str(),
            "resolved effective protection policy"
        );

        if let Some(existing) = store
            .load_audit(ctx.update_history_id)
            .await
            .map_err(plugin_internal)?
            .map(map_audit_record)
            && is_reusable_success(&existing, &policy)
        {
            tracing::info!(
                mode = policy.mode.as_str(),
                artifact_ref = ?existing.artifact_ref,
                "reusing existing successful protection artifact"
            );
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
                tracing::debug!("protection policy is do_nothing — skipping");
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
                store
                    .upsert_audit(&to_audit_record(&audit))
                    .await
                    .map_err(plugin_internal)?;
                Ok(ControllerProtectionDecision::skipped(Some(
                    SUMMARY_SKIPPED.to_string(),
                )))
            }
            ProtectionMode::Snapshot => {
                prepare_snapshot_protection(store, ctx, &mapping, &proxmox_cfg, &policy).await
            }
            ProtectionMode::Backup => {
                prepare_backup_protection(store, ctx, &mapping, &proxmox_cfg, &policy).await
            }
        }
    }

    #[tracing::instrument(skip_all, fields(
        update_history_id = %ctx.update_history_id,
        final_status = %ctx.final_status,
    ))]
    async fn finalize_post_update(
        &self,
        ctx: &ControllerPostUpdateContext<'_>,
    ) -> Result<PostUpdateOutcome> {
        let store = proxmox_protection_store(ctx.controller)?;
        let audit = store
            .load_audit(ctx.update_history_id)
            .await
            .map_err(plugin_internal)?
            .map(map_audit_record);

        if ctx.final_status == uptrakit_shared_types::UpdateStatus::Failed
            && audit
                .as_ref()
                .is_some_and(|row| row.status == "succeeded" && row.artifact_kind.is_some())
        {
            tracing::info!(
                artifact_kind = ?audit.as_ref().and_then(|r| r.artifact_kind.as_deref()),
                artifact_ref = ?audit.as_ref().and_then(|r| r.artifact_ref.as_deref()),
                "update failed — protection artifact available for manual recovery"
            );
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

fn proxmox_protection_store(
    controller: &dyn UpdateProtectionController,
) -> Result<&dyn ProxmoxProtectionStore> {
    controller.proxmox_protection_store().ok_or_else(|| {
        report!(PluginError::PluginInternal(
            "ControllerUpdateProtection expected a ProxmoxProtectionStore capability".to_string()
        ))
    })
}

async fn load_unique_mapping(
    store: &dyn ProxmoxProtectionStore,
    tenant_id: Uuid,
    host_id: Uuid,
) -> Result<Option<ProxmoxHostMappingRecord>> {
    store
        .load_host_mapping(tenant_id, host_id)
        .await
        .map_err(plugin_internal)
}

async fn load_proxmox_config(
    store: &dyn ProxmoxProtectionStore,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
) -> Result<ProxmoxConfig> {
    let payload = store
        .load_plugin_config_payload(tenant_id, plugin_config_id)
        .await
        .map_err(plugin_internal)?;

    serde_json::from_value(payload).map_err(|e| {
        report!(PluginError::Configuration(format!(
            "invalid Proxmox plugin configuration payload: {e}"
        )))
    })
}

fn map_policy_record(policy: ProxmoxProtectionPolicyRecord) -> ProtectionPolicy {
    ProtectionPolicy {
        mode: map_protection_mode_from_record(policy.mode),
        backup_target_key: policy.backup_target_key,
        snapshot_timeout_seconds: Some(
            policy
                .snapshot_timeout_seconds
                .unwrap_or(DEFAULT_SNAPSHOT_TIMEOUT_SECONDS),
        ),
        backup_timeout_seconds: Some(
            policy
                .backup_timeout_seconds
                .unwrap_or(DEFAULT_BACKUP_TIMEOUT_SECONDS),
        ),
    }
}

fn snapshot_wait_timeout(policy: &ProtectionPolicy) -> Duration {
    Duration::from_secs(
        policy
            .snapshot_timeout_seconds
            .expect("effective policy must already resolve snapshot timeout")
            .max(1) as u64,
    )
}

fn backup_wait_timeout(policy: &ProtectionPolicy) -> Duration {
    Duration::from_secs(
        policy
            .backup_timeout_seconds
            .expect("effective policy must already resolve backup timeout")
            .max(1) as u64,
    )
}

fn map_protection_mode_from_record(mode: ProxmoxProtectionMode) -> ProtectionMode {
    match mode {
        ProxmoxProtectionMode::DoNothing => ProtectionMode::DoNothing,
        ProxmoxProtectionMode::Snapshot => ProtectionMode::Snapshot,
        ProxmoxProtectionMode::Backup => ProtectionMode::Backup,
    }
}

fn map_protection_mode_to_record(mode: ProtectionMode) -> ProxmoxProtectionMode {
    match mode {
        ProtectionMode::DoNothing => ProxmoxProtectionMode::DoNothing,
        ProtectionMode::Snapshot => ProxmoxProtectionMode::Snapshot,
        ProtectionMode::Backup => ProxmoxProtectionMode::Backup,
    }
}

fn map_audit_record(row: ProxmoxProtectionAuditRecord) -> ProtectionAudit {
    ProtectionAudit {
        update_history_id: row.update_history_id,
        tenant_id: row.tenant_id,
        host_id: row.host_id,
        software_item_id: row.software_item_id,
        plugin_config_id: row.plugin_config_id,
        mapping_id: row.mapping_id,
        mode: map_protection_mode_from_record(row.mode),
        status: row.status,
        artifact_kind: row.artifact_kind,
        artifact_ref: row.artifact_ref,
        backup_target_key: row.backup_target_key,
        detail: row.detail,
        error_message: row.error_message,
    }
}

fn to_audit_record(row: &ProtectionAudit) -> ProxmoxProtectionAuditRecord {
    ProxmoxProtectionAuditRecord {
        update_history_id: row.update_history_id,
        tenant_id: row.tenant_id,
        host_id: row.host_id,
        software_item_id: row.software_item_id,
        plugin_config_id: row.plugin_config_id,
        mapping_id: row.mapping_id,
        mode: map_protection_mode_to_record(row.mode),
        status: row.status.clone(),
        artifact_kind: row.artifact_kind.clone(),
        artifact_ref: row.artifact_ref.clone(),
        backup_target_key: row.backup_target_key.clone(),
        detail: row.detail.clone(),
        error_message: row.error_message.clone(),
    }
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
    store: &dyn ProxmoxProtectionStore,
    ctx: &ControllerProtectionContext<'_>,
    mapping: &ProxmoxHostMappingRecord,
    proxmox_cfg: &ProxmoxConfig,
    policy: &ProtectionPolicy,
) -> Result<ControllerProtectionDecision> {
    let client = ProxmoxClient::new(proxmox_cfg).map_err(plugin_internal)?;
    let snapshot_name = snapshot_name_for_update_history(ctx.update_history_id);

    tracing::info!(
        node = %mapping.proxmox_node,
        vmid = mapping.proxmox_vmid,
        guest_type = %mapping.proxmox_type,
        snapshot_name = %snapshot_name,
        "creating Proxmox snapshot for pre-update protection"
    );
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(
            format!(
                "Creating Proxmox snapshot for {} (VMID {})…\n",
                mapping.proxmox_node, mapping.proxmox_vmid
            )
            .into_bytes(),
        );
    }

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
            tracing::warn!(
                node = %mapping.proxmox_node,
                vmid = mapping.proxmox_vmid,
                snapshot_name = %snapshot_name,
                error = %error,
                "Proxmox snapshot creation failed"
            );
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
            store
                .upsert_audit(&to_audit_record(&audit))
                .await
                .map_err(plugin_internal)?;
            return Ok(snapshot_decision_failure());
        }
    };

    tracing::debug!(node = %mapping.proxmox_node, vmid = mapping.proxmox_vmid, upid = %task, "snapshot task started — waiting for completion");

    if let Err(error) = client
        .wait_for_task_completion(&mapping.proxmox_node, &task, snapshot_wait_timeout(policy))
        .await
    {
        tracing::warn!(
            node = %mapping.proxmox_node,
            vmid = mapping.proxmox_vmid,
            snapshot_name = %snapshot_name,
            upid = %task,
            error = %error,
            "Proxmox snapshot task did not complete successfully"
        );
        if let Some(tx) = ctx.output_tx.as_ref() {
            let _ = tx.send(format!("Proxmox snapshot task failed: {error}\n").into_bytes());
        }
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
        store
            .upsert_audit(&to_audit_record(&audit))
            .await
            .map_err(plugin_internal)?;
        return Ok(snapshot_decision_failure());
    }

    tracing::info!(
        node = %mapping.proxmox_node,
        vmid = mapping.proxmox_vmid,
        snapshot_name = %snapshot_name,
        "Proxmox snapshot created successfully"
    );
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(
            format!(
                "Proxmox snapshot '{}' created successfully.\n",
                snapshot_name
            )
            .into_bytes(),
        );
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
    store
        .upsert_audit(&to_audit_record(&audit))
        .await
        .map_err(plugin_internal)?;

    Ok(ControllerProtectionDecision::success(
        Some(STATUS_PROTECTED.to_string()),
        Some(SUMMARY_SUCCESS.to_string()),
    ))
}

async fn prepare_backup_protection(
    store: &dyn ProxmoxProtectionStore,
    ctx: &ControllerProtectionContext<'_>,
    mapping: &ProxmoxHostMappingRecord,
    proxmox_cfg: &ProxmoxConfig,
    policy: &ProtectionPolicy,
) -> Result<ControllerProtectionDecision> {
    let Some(target_key) = policy.backup_target_key.as_deref() else {
        tracing::warn!(
            node = %mapping.proxmox_node,
            vmid = mapping.proxmox_vmid,
            "backup policy has no target key configured — cannot run backup"
        );
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
            backup_target_key: None,
            detail: Some(SUMMARY_FAILURE.to_string()),
            error_message: Some("missing backup target key for backup policy".to_string()),
        };
        store
            .upsert_audit(&to_audit_record(&audit))
            .await
            .map_err(plugin_internal)?;
        return Ok(snapshot_decision_failure());
    };

    let Some(target_storage_id) = store
        .find_cached_backup_target(mapping.plugin_config_id, target_key)
        .await
        .map_err(plugin_internal)?
    else {
        tracing::warn!(
            target_key,
            "configured backup target not found in cache — run discovery to refresh"
        );
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
            error_message: Some("configured backup target was not found in cache".to_string()),
        };
        store
            .upsert_audit(&to_audit_record(&audit))
            .await
            .map_err(plugin_internal)?;
        return Ok(snapshot_decision_failure());
    };

    tracing::info!(
        node = %mapping.proxmox_node,
        vmid = mapping.proxmox_vmid,
        guest_type = %mapping.proxmox_type,
        storage = %target_storage_id,
        "starting Proxmox backup for pre-update protection"
    );
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(
            format!(
                "Starting Proxmox backup for {} (VMID {}) to storage '{}'…\n",
                mapping.proxmox_node, mapping.proxmox_vmid, target_storage_id
            )
            .into_bytes(),
        );
    }

    let client = ProxmoxClient::new(proxmox_cfg).map_err(plugin_internal)?;

    let task = match client
        .start_backup(
            &mapping.proxmox_node,
            mapping.proxmox_vmid as u32,
            &mapping.proxmox_type,
            &target_storage_id,
        )
        .await
    {
        Ok(task) => task,
        Err(error) => {
            tracing::warn!(
                node = %mapping.proxmox_node,
                vmid = mapping.proxmox_vmid,
                storage = %target_storage_id,
                error = %error,
                "Proxmox backup task failed to start"
            );
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
            store
                .upsert_audit(&to_audit_record(&audit))
                .await
                .map_err(plugin_internal)?;
            return Ok(snapshot_decision_failure());
        }
    };

    tracing::debug!(node = %mapping.proxmox_node, vmid = mapping.proxmox_vmid, upid = %task, "backup task started — waiting for completion");

    if let Err(error) = client
        .wait_for_task_completion(&mapping.proxmox_node, &task, backup_wait_timeout(policy))
        .await
    {
        tracing::warn!(
            node = %mapping.proxmox_node,
            vmid = mapping.proxmox_vmid,
            storage = %target_storage_id,
            upid = %task,
            error = %error,
            "Proxmox backup task did not complete successfully"
        );
        if let Some(tx) = ctx.output_tx.as_ref() {
            let _ = tx.send(format!("Proxmox backup task failed: {error}\n").into_bytes());
        }
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
        store
            .upsert_audit(&to_audit_record(&audit))
            .await
            .map_err(plugin_internal)?;
        return Ok(snapshot_decision_failure());
    }

    tracing::info!(
        node = %mapping.proxmox_node,
        vmid = mapping.proxmox_vmid,
        storage = %target_storage_id,
        "Proxmox backup completed successfully"
    );
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(b"Proxmox backup completed successfully.\n".to_vec());
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
    store
        .upsert_audit(&to_audit_record(&audit))
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
    use async_trait::async_trait;
    use sea_orm::{ColumnTrait, DbBackend, EntityTrait, MockDatabase, MockExecResult, QueryFilter};
    use time::OffsetDateTime;
    use uptrakit_plugin_infrastructure_core::{
        ControllerProtectionContext, ProxmoxHostMappingRecord, ProxmoxProtectionAuditRecord,
        ProxmoxProtectionPolicyRecord, ProxmoxProtectionStore, SecretString,
        UpdateProtectionController,
    };
    use uptrakit_shared_db::entity::{
        prelude::ProxmoxBackupTargetCache, proxmox_backup_target_cache,
    };

    struct TestProtectionStore {
        db: sea_orm::DatabaseConnection,
    }

    #[async_trait]
    impl ProxmoxProtectionStore for TestProtectionStore {
        async fn load_host_mapping(
            &self,
            _tenant_id: Uuid,
            _host_id: Uuid,
        ) -> Result<Option<ProxmoxHostMappingRecord>> {
            unreachable!("load_host_mapping is not used in these tests")
        }

        async fn load_plugin_config_payload(
            &self,
            _tenant_id: Uuid,
            _plugin_config_id: Uuid,
        ) -> Result<serde_json::Value> {
            unreachable!("load_plugin_config_payload is not used in these tests")
        }

        async fn load_effective_policy(
            &self,
            _tenant_id: Uuid,
            _software_item_id: Uuid,
            _plugin_config_id: Uuid,
        ) -> Result<ProxmoxProtectionPolicyRecord> {
            unreachable!("load_effective_policy is not used in these tests")
        }

        async fn load_audit(
            &self,
            _update_history_id: Uuid,
        ) -> Result<Option<ProxmoxProtectionAuditRecord>> {
            unreachable!("load_audit is not used in these tests")
        }

        async fn upsert_audit(&self, audit: &ProxmoxProtectionAuditRecord) -> Result<()> {
            crate::policy_store::upsert_protection_audit(
                &self.db,
                &ProtectionAudit {
                    update_history_id: audit.update_history_id,
                    tenant_id: audit.tenant_id,
                    host_id: audit.host_id,
                    software_item_id: audit.software_item_id,
                    plugin_config_id: audit.plugin_config_id,
                    mapping_id: audit.mapping_id,
                    mode: map_protection_mode_from_record(audit.mode),
                    status: audit.status.clone(),
                    artifact_kind: audit.artifact_kind.clone(),
                    artifact_ref: audit.artifact_ref.clone(),
                    backup_target_key: audit.backup_target_key.clone(),
                    detail: audit.detail.clone(),
                    error_message: audit.error_message.clone(),
                },
            )
            .await
            .map_err(plugin_internal)
        }

        async fn find_cached_backup_target(
            &self,
            plugin_config_id: Uuid,
            target_key: &str,
        ) -> Result<Option<String>> {
            let row = ProxmoxBackupTargetCache::find()
                .filter(proxmox_backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
                .filter(proxmox_backup_target_cache::Column::TargetKey.eq(target_key))
                .one(&self.db)
                .await
                .map_err(plugin_internal)?;
            Ok(row.map(|row| row.storage_id))
        }
    }

    struct TestController;

    impl UpdateProtectionController for TestController {}

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
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
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
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
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

    #[tokio::test]
    async fn backup_mode_missing_target_key_persists_failed_audit() {
        let update_history_id = Uuid::now_v7();
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([Vec::<crate::policy_store::protection_audit::Model>::new()])
            .append_query_results([vec![mock_audit_model(
                update_history_id,
                tenant_id,
                host_id,
                software_item_id,
                plugin_config_id,
                None,
            )]])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let store = TestProtectionStore { db: db.clone() };
        let controller = TestController;

        let mapping = make_mapping(tenant_id, host_id, plugin_config_id);
        let policy = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
        };
        let proxmox_cfg = dummy_proxmox_config();
        let ctx = ControllerProtectionContext::new(
            &controller,
            tenant_id,
            host_id,
            software_item_id,
            update_history_id,
        );

        let decision = prepare_backup_protection(&store, &ctx, &mapping, &proxmox_cfg, &policy)
            .await
            .expect("prepare returns decision");
        assert!(decision.attempted);
        assert!(!decision.succeeded);

        let logs = db.into_transaction_log();
        assert!(
            logs.iter()
                .flat_map(|tx| tx.statements().iter())
                .any(|stmt| stmt.sql.contains("INSERT INTO `proxmox_protection_audit`")),
            "expected failed backup path to persist an audit insert"
        );
    }

    #[tokio::test]
    async fn backup_mode_missing_cached_target_persists_failed_audit() {
        let update_history_id = Uuid::now_v7();
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([Vec::<crate::policy_store::backup_target_cache::Model>::new()])
            .append_query_results([Vec::<crate::policy_store::protection_audit::Model>::new()])
            .append_query_results([vec![mock_audit_model(
                update_history_id,
                tenant_id,
                host_id,
                software_item_id,
                plugin_config_id,
                Some("pve1:local:dir".to_string()),
            )]])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let store = TestProtectionStore { db: db.clone() };
        let controller = TestController;

        let mapping = make_mapping(tenant_id, host_id, plugin_config_id);
        let target_key = "pve1:local:dir".to_string();
        let policy = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: Some(target_key.clone()),
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
        };
        let proxmox_cfg = dummy_proxmox_config();
        let ctx = ControllerProtectionContext::new(
            &controller,
            tenant_id,
            host_id,
            software_item_id,
            update_history_id,
        );

        let decision = prepare_backup_protection(&store, &ctx, &mapping, &proxmox_cfg, &policy)
            .await
            .expect("prepare returns decision");
        assert!(decision.attempted);
        assert!(!decision.succeeded);

        let logs = db.into_transaction_log();
        let insert_stmt = logs
            .iter()
            .flat_map(|tx| tx.statements().iter())
            .find(|stmt| stmt.sql.contains("INSERT INTO `proxmox_protection_audit`"))
            .expect("expected failed backup path to persist an audit insert");
        let rendered = insert_stmt.to_string();
        assert!(
            rendered.contains(&target_key),
            "audit insert should record missing target key"
        );
    }

    fn make_mapping(
        tenant_id: Uuid,
        host_id: Uuid,
        plugin_config_id: Uuid,
    ) -> ProxmoxHostMappingRecord {
        ProxmoxHostMappingRecord {
            id: Uuid::now_v7(),
            tenant_id,
            plugin_config_id,
            host_id: Some(host_id),
            proxmox_node: "pve1".to_string(),
            proxmox_vmid: 101,
            proxmox_type: "qemu".to_string(),
        }
    }

    fn mock_audit_model(
        update_history_id: Uuid,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
        backup_target_key: Option<String>,
    ) -> crate::policy_store::protection_audit::Model {
        crate::policy_store::protection_audit::Model {
            update_history_id,
            tenant_id,
            host_id,
            software_item_id,
            plugin_config_id,
            mapping_id: None,
            mode: "backup".to_string(),
            status: "failed".to_string(),
            artifact_kind: Some("backup".to_string()),
            artifact_ref: None,
            backup_target_key,
            detail: Some(SUMMARY_FAILURE.to_string()),
            error_message: Some("test".to_string()),
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    fn dummy_proxmox_config() -> ProxmoxConfig {
        ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!token=secret"),
            verify_tls: false,
            node_filter: vec![],
        }
    }

    #[test]
    fn snapshot_wait_timeout_prefers_policy_value() {
        let policy = ProtectionPolicy {
            mode: ProtectionMode::Snapshot,
            backup_target_key: None,
            snapshot_timeout_seconds: Some(240),
            backup_timeout_seconds: Some(900),
        };

        assert_eq!(snapshot_wait_timeout(&policy), Duration::from_secs(240));
    }

    #[test]
    fn backup_wait_timeout_prefers_policy_value() {
        let policy = ProtectionPolicy {
            mode: ProtectionMode::Backup,
            backup_target_key: Some("pbs-home:pbs".to_string()),
            snapshot_timeout_seconds: Some(120),
            backup_timeout_seconds: Some(900),
        };

        assert_eq!(backup_wait_timeout(&policy), Duration::from_secs(900));
    }

    #[test]
    fn map_policy_record_applies_default_snapshot_timeout() {
        let policy = map_policy_record(ProxmoxProtectionPolicyRecord {
            mode: ProxmoxProtectionMode::Snapshot,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
        });
        assert_eq!(snapshot_wait_timeout(&policy), Duration::from_secs(120));
    }

    #[test]
    fn map_policy_record_applies_default_backup_timeout() {
        let policy = map_policy_record(ProxmoxProtectionPolicyRecord {
            mode: ProxmoxProtectionMode::Backup,
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
        });
        assert_eq!(backup_wait_timeout(&policy), Duration::from_secs(900));
    }
}
