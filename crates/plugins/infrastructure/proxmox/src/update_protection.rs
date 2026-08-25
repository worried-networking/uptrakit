#![expect(
    clippy::let_underscore_must_use,
    clippy::expect_used,
    reason = "best-effort fire-and-forget operations where failure is intentionally ignored; infallible literal surface ID and value constructions; panic would indicate a programming error in the surface manifest"
)]
//! Controller-side pre/post update protection for Proxmox-backed hosts.

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::error::{PluginError, Result};
use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ControllerPostUpdateContext, ControllerProtectionContext,
    ControllerProtectionDecision, ControllerUpdateProtection, PluginMeta, PluginTypeId,
    PostUpdateOutcome,
};

use crate::protection_store::{
    DbProxmoxProtectionStore, ProxmoxHostMappingRecord, ProxmoxProtectionAuditRecord,
    ProxmoxProtectionMode, ProxmoxProtectionPolicyRecord, ProxmoxProtectionStore,
};
use uuid::Uuid;

use crate::client::ProxmoxClient;
use crate::config::ProxmoxConfig;
use crate::policy_store::{ProtectionAudit, ProtectionMode, ProtectionPolicy};
use uptrakit_shared_db::entity::prelude::{SoftwareItem, UpdateHistory};

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
        let store = DbProxmoxProtectionStore {
            db: ctx.controller.tenant_db().db(),
        };

        let mapping = match load_unique_mapping(&store, ctx.tenant_id, ctx.host_id).await? {
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
            load_proxmox_config(&store, ctx.tenant_id, mapping.plugin_config_id).await?;
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
                let artifact_meta = load_update_artifact_context(
                    ctx.controller.tenant_db(),
                    ctx.software_item_id,
                    ctx.update_history_id,
                )
                .await;
                prepare_snapshot_protection(
                    &store,
                    ctx,
                    &mapping,
                    &proxmox_cfg,
                    &policy,
                    &artifact_meta,
                )
                .await
            }
            ProtectionMode::Backup => {
                let artifact_meta = load_update_artifact_context(
                    ctx.controller.tenant_db(),
                    ctx.software_item_id,
                    ctx.update_history_id,
                )
                .await;
                prepare_backup_protection(
                    &store,
                    ctx,
                    &mapping,
                    &proxmox_cfg,
                    &policy,
                    &artifact_meta,
                )
                .await
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
        let store = DbProxmoxProtectionStore {
            db: ctx.controller.tenant_db().db(),
        };
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
        PluginTypeId::from_static("infrastructure.proxmox")
    }
}

fn snapshot_decision_failure() -> ControllerProtectionDecision {
    ControllerProtectionDecision::failure(
        Some(STATUS_FAILED.to_string()),
        Some(SUMMARY_FAILURE.to_string()),
    )
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
    artifact_meta: &ProxmoxUpdateArtifactContext,
) -> Result<ControllerProtectionDecision> {
    let client = ProxmoxClient::new(proxmox_cfg).map_err(plugin_internal)?;
    let snapshot_name = snapshot_name_for_update_history(
        ctx.update_history_id,
        artifact_meta.software_name.as_deref(),
    );
    let description = snapshot_description(ctx.update_history_id, artifact_meta);

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
                &description,
            )
            .await
    } else {
        client
            .create_qemu_snapshot(
                &mapping.proxmox_node,
                mapping.proxmox_vmid as u32,
                &snapshot_name,
                &description,
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
            if let Some(tx) = ctx.output_tx.as_ref() {
                let _ =
                    tx.send(format!("Proxmox snapshot failed to start: {error}\n").into_bytes());
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

    let wait_result = if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(b"\n--- Proxmox snapshot log ---\n".to_vec());
        let result = client
            .wait_for_task_completion_with_logs(
                &mapping.proxmox_node,
                &task,
                snapshot_wait_timeout(policy),
                tx,
            )
            .await;
        let _ = tx.send(b"--- end ---\n\n".to_vec());
        result
    } else {
        client
            .wait_for_task_completion(&mapping.proxmox_node, &task, snapshot_wait_timeout(policy))
            .await
    };

    if let Err(error) = wait_result {
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
            format!("Proxmox snapshot '{snapshot_name}' created successfully.\n").into_bytes(),
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
    artifact_meta: &ProxmoxUpdateArtifactContext,
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
    let notes = backup_notes_template(ctx.update_history_id, artifact_meta);

    let task = match client
        .start_backup(
            &mapping.proxmox_node,
            mapping.proxmox_vmid as u32,
            &mapping.proxmox_type,
            &target_storage_id,
            &notes,
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
            if let Some(tx) = ctx.output_tx.as_ref() {
                let _ = tx.send(format!("Proxmox backup failed to start: {error}\n").into_bytes());
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

    let wait_result = if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(b"\n--- Proxmox backup log ---\n".to_vec());
        let result = client
            .wait_for_task_completion_with_logs(
                &mapping.proxmox_node,
                &task,
                backup_wait_timeout(policy),
                tx,
            )
            .await;
        let _ = tx.send(b"--- end ---\n\n".to_vec());
        result
    } else {
        client
            .wait_for_task_completion(&mapping.proxmox_node, &task, backup_wait_timeout(policy))
            .await
    };

    if let Err(error) = wait_result {
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

/// Decorative metadata used to build human-readable protection artifact names,
/// descriptions, and notes. Never affects the protection workflow's success/
/// failure outcome — loading it is best-effort and infallible.
#[derive(Clone, Debug, Default, PartialEq)]
struct ProxmoxUpdateArtifactContext {
    software_name: Option<String>,
    from_version: Option<String>,
    to_version: Option<String>,
}

/// Load software name and version-transition metadata for a pre-update
/// protection artifact's name/description/notes. Infallible by design: a DB
/// error or missing row degrades the affected field(s) to `None` rather than
/// blocking the protection workflow, since this metadata is purely
/// decorative display text.
async fn load_update_artifact_context(
    tenant_db: &uptrakit_tenant_db::TenantDb,
    software_item_id: Uuid,
    update_history_id: Uuid,
) -> ProxmoxUpdateArtifactContext {
    let software_name = match tenant_db
        .find_by_id::<SoftwareItem, _>(software_item_id)
        .one(tenant_db.db())
        .await
    {
        Ok(Some(item)) => Some(item.name),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                software_item_id = %software_item_id,
                error = %error,
                "failed to load software item for protection artifact metadata"
            );
            None
        }
    };

    let (from_version, to_version) = match tenant_db
        .find_by_id::<UpdateHistory, _>(update_history_id)
        .one(tenant_db.db())
        .await
    {
        Ok(Some(history)) => (history.from_version, history.to_version),
        Ok(None) => (None, None),
        Err(error) => {
            tracing::warn!(
                update_history_id = %update_history_id,
                error = %error,
                "failed to load update history for protection artifact metadata"
            );
            (None, None)
        }
    };

    ProxmoxUpdateArtifactContext {
        software_name,
        from_version,
        to_version,
    }
}

/// Derive a Proxmox-safe snapshot name for an update-history row.
///
/// Format: `upk-<sanitized-software-name>-<hex8>`, where `hex8` is the first
/// 8 hex characters of `update_history_id`'s simple (no-dashes) form, and the
/// sanitized software name is lowercased ASCII alphanumerics with runs of
/// other characters collapsed to a single `-` (leading/trailing separators
/// stripped), capped at 27 chars. Total budget: `upk-` (4) + name (<=27) +
/// `-` (1) + hex8 (8) = <=40 chars, satisfying Proxmox's snapname length
/// limit.
///
/// When `software_name` is `None`, or it sanitizes to an empty string (e.g. a
/// fully non-ASCII name), the name degrades to `upk-<hex8>` — no name
/// segment, no double dash.
///
/// The first character is always `u`, satisfying PVE's snapname regex
/// `[A-Za-z][A-Za-z0-9_\-]*`.
///
/// The name is deterministic for a given `(update_history_id, software_name)`
/// pair, but retry-reuse logic does not depend on this determinism — the
/// persisted `artifact_ref` in the protection audit row is what's reused on
/// retry, never recomputed (see `is_reusable_success`).
pub fn snapshot_name_for_update_history(
    update_history_id: Uuid,
    software_name: Option<&str>,
) -> String {
    let mut hex8 = update_history_id.simple().to_string();
    hex8.truncate(8);

    let sanitized = software_name
        .map(sanitize_snapshot_segment)
        .unwrap_or_default();

    if sanitized.is_empty() {
        format!("upk-{hex8}")
    } else {
        format!("upk-{sanitized}-{hex8}")
    }
}

/// Sanitize a software name into a Proxmox-safe filename segment (lowercase
/// ASCII alphanumerics, `-` separators, capped at 27 chars). Distinct from
/// [`sanitize_label`], which sanitizes text for human-readable descriptions.
fn sanitize_snapshot_segment(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if out.len() >= 27 {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Sanitize untrusted display text (software name, version strings) for
/// embedding in a PVE snapshot description or vzdump notes-template: control
/// characters (including `\n`, `\r`, `\t`) are replaced with a single space
/// so untrusted metadata can never forge an extra line in operator-facing
/// text, then the result is truncated to at most `max_chars` characters.
fn sanitize_label(text: &str, max_chars: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max_chars)
        .collect()
}

/// Sanitized `(name, version_clause)` pieces shared by [`snapshot_description`]
/// and [`backup_notes_template`].
struct ArtifactLabelParts {
    name: String,
    version_clause: String,
}

fn artifact_label_parts(meta: &ProxmoxUpdateArtifactContext) -> ArtifactLabelParts {
    let name = meta
        .software_name
        .as_deref()
        .map(|n| sanitize_label(n, 64))
        .unwrap_or_else(|| "unknown software".to_string());

    let version_clause = match (meta.from_version.as_deref(), meta.to_version.as_deref()) {
        (Some(from), Some(to)) => format!(
            " ({} -> {})",
            sanitize_label(from, 32),
            sanitize_label(to, 32)
        ),
        (None, Some(to)) => format!(" (-> {})", sanitize_label(to, 32)),
        _ => String::new(),
    };

    ArtifactLabelParts {
        name,
        version_clause,
    }
}

/// Build the multi-line PVE snapshot `description` for a pre-update
/// protection snapshot.
fn snapshot_description(update_history_id: Uuid, meta: &ProxmoxUpdateArtifactContext) -> String {
    let parts = artifact_label_parts(meta);
    format!(
        "Uptrakit pre-update protection snapshot.\nSoftware: {}{}\nUpdate ID: {update_history_id}\nCreated automatically by Uptrakit before applying this update. Safe to remove once the update is verified.",
        parts.name, parts.version_clause
    )
}

/// Build the single-line vzdump `notes-template` for a pre-update protection
/// backup. vzdump's own `notes-template` mechanism treats a literal `{{` in
/// the value as the start of its variable-substitution syntax, so the
/// interpolated (untrusted) pieces are escaped (`\` -> `/`, `{` -> `(`,
/// `}` -> `)`) after [`sanitize_label`] neutralizes control characters.
fn backup_notes_template(update_history_id: Uuid, meta: &ProxmoxUpdateArtifactContext) -> String {
    let parts = artifact_label_parts(meta);
    let interpolated = escape_notes_template(&format!("{}{}", parts.name, parts.version_clause));
    format!(
        "Uptrakit pre-update protection backup - Software: {interpolated} - Update ID: {update_history_id} - Created automatically by Uptrakit"
    )
}

fn escape_notes_template(text: &str) -> String {
    text.replace('\\', "/").replace('{', "(").replace('}', ")")
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::unreachable,
        reason = "test stub methods use unreachable! to signal unexpected calls"
    )]

    use super::*;
    use crate::entity::{
        proxmox_backup_target_cache,
        proxmox_backup_target_cache::Entity as ProxmoxBackupTargetCache,
    };
    use async_trait::async_trait;
    use sea_orm::{ColumnTrait, DbBackend, EntityTrait, MockDatabase, MockExecResult, QueryFilter};
    use time::OffsetDateTime;
    use uptrakit_plugin_infrastructure_core::{
        ControllerProtectionContext, SecretString, UpdateProtectionController,
    };

    use crate::protection_store::{
        ProxmoxHostMappingRecord, ProxmoxProtectionAuditRecord, ProxmoxProtectionPolicyRecord,
        ProxmoxProtectionStore,
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

    impl UpdateProtectionController for TestController {
        #[expect(
            clippy::unimplemented,
            reason = "tenant_db is never called by these unit tests"
        )]
        fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
            unimplemented!("tenant_db not called by these unit tests")
        }
    }

    #[test]
    fn snapshot_name_is_deterministic_and_safe() {
        let update_history_id =
            Uuid::parse_str("5bfb4e73-a0b4-4c81-b43d-b986589f6205").expect("valid uuid");
        let a = snapshot_name_for_update_history(update_history_id, Some("Test Software 2!"));
        let b = snapshot_name_for_update_history(update_history_id, Some("Test Software 2!"));

        assert_eq!(a, b);
        assert!(a.len() <= 40, "snapshot name must be <= 40 chars");
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "snapshot name must be Proxmox-safe"
        );
        assert!(
            a.starts_with("upk-test-software-2-"),
            "expected sanitized name prefix, got {a}"
        );
    }

    #[test]
    fn snapshot_name_truncates_long_software_name_without_trailing_dash() {
        let update_history_id = Uuid::now_v7();
        let long_name = "a".repeat(60);
        let name = snapshot_name_for_update_history(update_history_id, Some(&long_name));

        assert!(name.len() <= 40, "snapshot name must be <= 40 chars");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "snapshot name must be Proxmox-safe"
        );
        let segment = name
            .strip_prefix("upk-")
            .and_then(|rest| rest.rsplit_once('-'))
            .map(|(segment, _hex8)| segment)
            .expect("name must have a name segment and hex8 suffix");
        assert!(
            !segment.ends_with('-'),
            "truncated name segment must not leak a trailing separator: {segment:?}"
        );
    }

    #[test]
    fn snapshot_name_falls_back_to_hex8_only_for_non_ascii_or_missing_name() {
        let update_history_id = Uuid::now_v7();

        let non_ascii = snapshot_name_for_update_history(update_history_id, Some("производство"));
        let missing = snapshot_name_for_update_history(update_history_id, None);

        let mut hex8 = update_history_id.simple().to_string();
        hex8.truncate(8);
        let expected = format!("upk-{hex8}");

        assert_eq!(non_ascii, expected);
        assert_eq!(missing, expected);
        assert_eq!(
            snapshot_name_for_update_history(update_history_id, Some("производство")),
            non_ascii,
            "must be deterministic"
        );
        assert_eq!(
            snapshot_name_for_update_history(update_history_id, None),
            missing,
            "must be deterministic"
        );
    }

    #[test]
    fn snapshot_description_includes_metadata_and_neutralizes_injected_newline() {
        let update_history_id = Uuid::now_v7();
        let meta = ProxmoxUpdateArtifactContext {
            software_name: Some("Home Assistant".to_string()),
            from_version: Some("1.0.0".to_string()),
            to_version: Some("2.0.0".to_string()),
        };
        let description = snapshot_description(update_history_id, &meta);

        assert!(description.contains("Home Assistant"));
        assert!(description.contains("1.0.0 -> 2.0.0"));
        assert!(description.contains(&update_history_id.to_string()));
        assert!(description.contains("Uptrakit"));

        let malicious_meta = ProxmoxUpdateArtifactContext {
            software_name: Some("Evil\nSoftware".to_string()),
            from_version: None,
            to_version: None,
        };
        let malicious_description = snapshot_description(update_history_id, &malicious_meta);
        assert_eq!(
            malicious_description.lines().count(),
            4,
            "embedded newline in software name must not add an extra line: {malicious_description:?}"
        );
    }

    #[test]
    fn backup_notes_template_is_single_line_and_strips_vzdump_special_chars() {
        let update_history_id = Uuid::now_v7();
        let meta = ProxmoxUpdateArtifactContext {
            software_name: Some("bad{{name}}\nx\\".to_string()),
            from_version: Some("1{{\n0".to_string()),
            to_version: Some("2}}\n0".to_string()),
        };
        let notes = backup_notes_template(update_history_id, &meta);

        assert!(
            !notes.contains('\n'),
            "notes must be single line: {notes:?}"
        );
        assert!(
            !notes.contains('{'),
            "notes must not contain '{{': {notes:?}"
        );
        assert!(
            !notes.contains('}'),
            "notes must not contain '}}': {notes:?}"
        );
        assert!(
            !notes.contains('\\'),
            "notes must not contain '\\\\': {notes:?}"
        );
    }

    #[test]
    fn sanitize_label_truncates_by_char_and_strips_control_chars() {
        let multi_byte = "😀".repeat(10) + &"я".repeat(10);
        let truncated = sanitize_label(&multi_byte, 5);
        assert_eq!(
            truncated.chars().count(),
            5,
            "must truncate by chars, not bytes"
        );

        let with_control = "a\tb\nc\rd";
        let cleaned = sanitize_label(with_control, 64);
        assert!(
            !cleaned.chars().any(|c| c.is_control()),
            "control chars must be replaced: {cleaned:?}"
        );
    }

    #[tokio::test]
    async fn load_update_artifact_context_returns_both_rows_when_present() {
        let software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();
        let tenant_id = Uuid::now_v7();

        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([vec![mock_software_item_model(
                software_item_id,
                tenant_id,
                "Test App",
            )]])
            .append_query_results([vec![mock_update_history_model(
                update_history_id,
                tenant_id,
                software_item_id,
                Some("9.9.9".to_string()),
                Some("10.0.0".to_string()),
            )]])
            .into_connection();
        let tenant_db = uptrakit_tenant_db::TenantDb::new(db, tenant_id);

        let meta =
            load_update_artifact_context(&tenant_db, software_item_id, update_history_id).await;

        assert_eq!(meta.software_name, Some("Test App".to_string()));
        assert_eq!(meta.from_version, Some("9.9.9".to_string()));
        assert_eq!(meta.to_version, Some("10.0.0".to_string()));
    }

    #[tokio::test]
    async fn load_update_artifact_context_defaults_when_rows_missing() {
        use uptrakit_shared_db::entity::prelude::{SoftwareItemModel, UpdateHistoryModel};

        let software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();
        let tenant_id = Uuid::now_v7();

        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([Vec::<SoftwareItemModel>::new()])
            .append_query_results([Vec::<UpdateHistoryModel>::new()])
            .into_connection();
        let tenant_db = uptrakit_tenant_db::TenantDb::new(db, tenant_id);

        let meta =
            load_update_artifact_context(&tenant_db, software_item_id, update_history_id).await;

        assert_eq!(meta.software_name, None);
        assert_eq!(meta.from_version, None);
        assert_eq!(meta.to_version, None);
    }

    #[tokio::test]
    async fn load_update_artifact_context_defaults_on_db_error_without_panicking() {
        let software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();
        let tenant_id = Uuid::now_v7();

        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_errors([sea_orm::DbErr::Custom("boom".to_string())])
            .append_query_errors([sea_orm::DbErr::Custom("boom".to_string())])
            .into_connection();
        let tenant_db = uptrakit_tenant_db::TenantDb::new(db, tenant_id);

        let meta =
            load_update_artifact_context(&tenant_db, software_item_id, update_history_id).await;

        assert_eq!(meta, ProxmoxUpdateArtifactContext::default());
    }

    fn mock_software_item_model(
        id: Uuid,
        tenant_id: Uuid,
        name: &str,
    ) -> uptrakit_shared_db::entity::prelude::SoftwareItemModel {
        uptrakit_shared_db::entity::prelude::SoftwareItemModel {
            id,
            tenant_id,
            name: name.to_string(),
            featured: false,
            icon_url: None,
            last_checked_at: None,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
            deactivated_at: None,
            awaiting_restart_timeout: None,
        }
    }

    fn mock_update_history_model(
        id: Uuid,
        tenant_id: Uuid,
        software_item_id: Uuid,
        from_version: Option<String>,
        to_version: Option<String>,
    ) -> uptrakit_shared_db::entity::prelude::UpdateHistoryModel {
        uptrakit_shared_db::entity::prelude::UpdateHistoryModel {
            id,
            tenant_id,
            host_id: Uuid::now_v7(),
            software_item_id,
            host_software_item_id: None,
            from_version,
            to_version,
            status: uptrakit_shared_db::entity::update_history::UpdateStatus::Pending,
            output: String::new(),
            output_bytes: 0,
            actor_type: "user".to_string(),
            actor_id: "test".to_string(),
            execution_owner_service_id: None,
            execution_owner_instance_id: None,
            started_at: None,
            completed_at: None,
            awaiting_restart_since: None,
            created_at: OffsetDateTime::now_utc(),
            update_category: "unknown".to_string(),
            batch_id: None,
            interactive: false,
            output_truncated: false,
            pre_update_protection_status: None,
            pre_update_protection_summary: None,
            recovery_hint: None,
            timeout_seconds: None,
        }
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
            artifact_ref: Some("upk-abc".to_string()),
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
            artifact_ref: Some("upk-abc".to_string()),
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

        let decision = prepare_backup_protection(
            &store,
            &ctx,
            &mapping,
            &proxmox_cfg,
            &policy,
            &ProxmoxUpdateArtifactContext::default(),
        )
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

        let decision = prepare_backup_protection(
            &store,
            &ctx,
            &mapping,
            &proxmox_cfg,
            &policy,
            &ProxmoxUpdateArtifactContext::default(),
        )
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
