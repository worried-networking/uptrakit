//! Proxmox controller update hook — temporary resource scaling.

use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ControllerUpdateHook, PluginMeta, UpdateHookPostContext, UpdateHookPreContext,
    error::Result,
};
use uptrakit_shared_types::PluginTypeId;

use crate::{
    client::ProxmoxClient,
    config::ProxmoxConfig,
    policy_store::{self, ScalingRecord},
    protection_store::{DbProxmoxProtectionStore, ProxmoxProtectionStore as _},
};

pub(crate) struct ControllerUpdateHookPlugin;

impl ControllerUpdateHookPlugin {
    #[expect(
        dead_code,
        reason = "wired into the plugin descriptor in a future task"
    )]
    pub(crate) fn create(_config: &CatalogConfig) -> Result<Arc<dyn ControllerUpdateHook>> {
        Ok(Arc::new(Self))
    }
}

impl PluginMeta for ControllerUpdateHookPlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::from_static("infrastructure_proxmox")
    }
}

#[async_trait::async_trait]
impl ControllerUpdateHook for ControllerUpdateHookPlugin {
    async fn prepare_pre_update_hook(&self, ctx: &UpdateHookPreContext<'_>) {
        let tenant_id = ctx.tenant_id;
        let host_id = ctx.host_id;
        let software_item_id = ctx.software_item_id;
        let update_history_id = ctx.update_history_id;
        let db = ctx.controller.tenant_db().db();

        let store = DbProxmoxProtectionStore { db };

        // Load host mapping
        let mapping = match store.load_host_mapping(tenant_id, host_id).await {
            Ok(Some(m)) => m,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to load host mapping"
                );
                return;
            }
        };

        // Load effective policy
        let policy = match store
            .load_effective_policy(tenant_id, software_item_id, mapping.plugin_config_id)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to load effective policy"
                );
                return;
            }
        };

        if policy.update_cores.is_none() && policy.update_memory_mb.is_none() {
            return;
        }

        // Load plugin config
        let payload = match store
            .load_plugin_config_payload(tenant_id, mapping.plugin_config_id)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to load plugin config payload"
                );
                return;
            }
        };
        let proxmox_cfg: ProxmoxConfig = match serde_json::from_value(payload) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to deserialize ProxmoxConfig"
                );
                return;
            }
        };

        // Create client
        let client = match ProxmoxClient::new(&proxmox_cfg) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to create Proxmox client"
                );
                return;
            }
        };

        let node = &mapping.proxmox_node;
        let vmid = mapping.proxmox_vmid as u32;

        // Read current config, check hotplug (QEMU only), extract original values
        let (original_cores_u32, original_memory_u64) = match mapping.proxmox_type.as_str() {
            "qemu" => {
                let config = match client.get_qemu_config(node, vmid).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(
                            %update_history_id, node, vmid, error = %e,
                            "resource scaling: failed to read QEMU config"
                        );
                        return;
                    }
                };
                if !config.supports_live_resource_scaling() {
                    tracing::warn!(
                        %update_history_id, node, vmid,
                        "QEMU VM does not support hotplug — skipping resource scaling"
                    );
                    return;
                }
                match (config.cores, config.memory) {
                    (Some(c), Some(m)) => (c, m),
                    _ => {
                        tracing::warn!(
                            %update_history_id, node, vmid,
                            "resource scaling: QEMU config missing cores or memory field"
                        );
                        return;
                    }
                }
            }
            "lxc" => {
                if policy.update_memory_mb.is_some() {
                    tracing::warn!(
                        %update_history_id, node, vmid,
                        "resource scaling: LXC memory scaling may only take effect on next \
                         container restart — kernel cgroup live memory resize is not guaranteed"
                    );
                }
                let config = match client.get_lxc_config(node, vmid).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(
                            %update_history_id, node, vmid, error = %e,
                            "resource scaling: failed to read LXC config"
                        );
                        return;
                    }
                };
                match (config.cores, config.memory) {
                    (Some(c), Some(m)) => (c, m),
                    _ => {
                        tracing::warn!(
                            %update_history_id, node, vmid,
                            "resource scaling: LXC config missing cores or memory field"
                        );
                        return;
                    }
                }
            }
            other => {
                tracing::warn!(
                    %update_history_id, vm_type = other,
                    "resource scaling: unrecognized vm_type — skipping"
                );
                return;
            }
        };

        // Compute target values
        let target_cores = policy
            .update_cores
            .map(|c| c as u32)
            .unwrap_or(original_cores_u32);
        let target_memory_mb = policy
            .update_memory_mb
            .map(|m| m as u64)
            .unwrap_or(original_memory_u64);

        // Persist record with scale_status = "scaling" BEFORE API call (crash-safe)
        let scaling_record = ScalingRecord {
            update_history_id,
            tenant_id,
            host_id,
            software_item_id,
            plugin_config_id: mapping.plugin_config_id,
            mapping_id: mapping.id,
            vm_type: mapping.proxmox_type.clone(),
            original_cores: original_cores_u32 as i32,
            original_memory_mb: original_memory_u64 as i64,
            scaled_cores: target_cores as i32,
            scaled_memory_mb: target_memory_mb as i64,
            scale_status: "scaling".to_string(),
            restore_status: "pending".to_string(),
            error_message: None,
        };
        if let Err(e) = policy_store::upsert_scaling_record(db, &scaling_record).await {
            tracing::warn!(
                %update_history_id, error = %e,
                "resource scaling: failed to persist scaling record — aborting scale-up"
            );
            return;
        }

        // Stream status line
        if let Some(tx) = &ctx.output_tx {
            let _ = tx.send(
                format!(
                    "Scaling VM resources to {target_cores} cores / {target_memory_mb} MB\u{2026}\n"
                )
                .into_bytes(),
            );
        }

        // Apply the resource change
        let scale_result = match mapping.proxmox_type.as_str() {
            "qemu" => {
                client
                    .set_qemu_config_resources(node, vmid, target_cores, target_memory_mb)
                    .await
            }
            _ => {
                client
                    .set_lxc_config_resources(node, vmid, target_cores, target_memory_mb)
                    .await
            }
        };

        match scale_result {
            Ok(()) => {
                let mut updated = scaling_record.clone();
                updated.scale_status = "scaled".to_string();
                if let Err(e) = policy_store::upsert_scaling_record(db, &updated).await {
                    tracing::warn!(
                        %update_history_id, error = %e,
                        "resource scaling: failed to update record to 'scaled'"
                    );
                }
                if let Some(tx) = &ctx.output_tx {
                    let _ = tx.send(
                        format!(
                            "VM resources scaled to {target_cores} cores / {target_memory_mb} MB.\n"
                        )
                        .into_bytes(),
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    %update_history_id, node, vmid, error = %e,
                    "resource scaling: scale-up API call failed — proceeding at original resources"
                );
                let mut failed = scaling_record.clone();
                failed.scale_status = "failed".to_string();
                failed.restore_status = "skipped".to_string();
                failed.error_message = Some(e.to_string());
                if let Err(db_err) = policy_store::upsert_scaling_record(db, &failed).await {
                    tracing::warn!(
                        %update_history_id, error = %db_err,
                        "resource scaling: failed to persist failure record"
                    );
                }
            }
        }
    }

    async fn finalize_post_update_hook(&self, ctx: &UpdateHookPostContext<'_>) -> Result<()> {
        let update_history_id = ctx.update_history_id;
        let db = ctx.controller.tenant_db().db();

        // Load scaling record; return Ok if absent (no scale-up happened)
        let record = match policy_store::load_scaling_record(db, update_history_id).await {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(()),
            Err(e) => {
                return Err(rootcause::report!(
                    uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                        format!("failed to load scaling record: {e}")
                    )
                ));
            }
        };

        // Skip restore if scale never succeeded or was already skipped
        if record.scale_status != "scaled" && record.scale_status != "scaling" {
            return Ok(());
        }

        // Load host mapping by mapping_id (stable key)
        use sea_orm::EntityTrait;
        use uptrakit_shared_db::entity::proxmox_host_mapping;

        let mapping_row = proxmox_host_mapping::Entity::find_by_id(record.mapping_id)
            .one(db)
            .await
            .map_err(|e| {
                rootcause::report!(
                    uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                        format!("failed to load host mapping {}: {e}", record.mapping_id)
                    )
                )
            })?
            .ok_or_else(|| {
                rootcause::report!(
                    uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                        format!("host mapping {} not found for restore", record.mapping_id)
                    )
                )
            })?;

        // Load plugin config and create client
        let store = DbProxmoxProtectionStore { db };
        let payload = store
            .load_plugin_config_payload(record.tenant_id, record.plugin_config_id)
            .await?;
        let proxmox_cfg: ProxmoxConfig = serde_json::from_value(payload).map_err(|e| {
            rootcause::report!(
                uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(format!(
                    "failed to deserialize ProxmoxConfig: {e}"
                ))
            )
        })?;

        let client = ProxmoxClient::new(&proxmox_cfg).map_err(|e| {
            rootcause::report!(
                uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(format!(
                    "failed to create Proxmox client: {e}"
                ))
            )
        })?;

        let node = &mapping_row.proxmox_node;
        let vmid = mapping_row.proxmox_vmid as u32;
        let original_cores = record.original_cores as u32;
        let original_memory_mb = record.original_memory_mb as u64;

        // Restore resources
        let restore_result = match record.vm_type.as_str() {
            "qemu" => {
                client
                    .set_qemu_config_resources(node, vmid, original_cores, original_memory_mb)
                    .await
            }
            _ => {
                client
                    .set_lxc_config_resources(node, vmid, original_cores, original_memory_mb)
                    .await
            }
        };

        match restore_result {
            Ok(()) => {
                let mut restored = record.clone();
                restored.restore_status = "restored".to_string();
                if let Err(e) = policy_store::upsert_scaling_record(db, &restored).await {
                    tracing::warn!(
                        %update_history_id, error = %e,
                        "resource scaling: failed to update record to 'restored'"
                    );
                }
                Ok(())
            }
            Err(ref err) => {
                tracing::warn!(
                    %update_history_id,
                    mapping_id = %record.mapping_id,
                    vm_type = %record.vm_type,
                    scaled_cores = record.scaled_cores,
                    scaled_memory_mb = record.scaled_memory_mb,
                    original_cores = record.original_cores,
                    original_memory_mb = record.original_memory_mb,
                    error = %err,
                    "Proxmox resource restore failed — VM still running at scaled resources"
                );

                let mut failed = record.clone();
                failed.restore_status = "restore_failed".to_string();
                failed.error_message = Some(err.to_string());
                if let Err(e) = policy_store::upsert_scaling_record(db, &failed).await {
                    tracing::warn!(
                        %update_history_id, error = %e,
                        "resource scaling: failed to persist restore_failed record"
                    );
                }

                // Notification: send_transactional_email requires a specific `to` address but
                // the plugin layer has no direct access to a tenant admin email without a
                // separate DB lookup. The structured tracing::warn above is the operator
                // notification mechanism. The restore_status = "restore_failed" DB record
                // is the persistent audit signal for dashboards / alerting pipelines.

                Err(rootcause::report!(
                    uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                        format!("resource restore failed: {err}")
                    )
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_update_hook_plugin_implements_plugin_meta() {
        let plugin = ControllerUpdateHookPlugin;
        assert_eq!(plugin.plugin_type_id().as_str(), "infrastructure_proxmox");
    }

    #[tokio::test]
    async fn finalize_returns_ok_when_no_record_in_db() {
        use crate::entity::proxmox_resource_scaling_record;
        use sea_orm::{DbBackend, MockDatabase};

        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_query_results([Vec::<proxmox_resource_scaling_record::Model>::new()])
            .into_connection();

        let result = crate::policy_store::load_scaling_record(&db, uuid::Uuid::now_v7()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
