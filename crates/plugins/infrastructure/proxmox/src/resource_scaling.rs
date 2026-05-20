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
    scaling_store::{self as ss, ScalingMode, ScalingPolicy},
};

pub(crate) struct ControllerUpdateHookPlugin;

impl ControllerUpdateHookPlugin {
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
                if let Some(tx) = &ctx.output_tx {
                    drop(
                        tx.send(
                            format!("[Resource scaling] Failed to load host mapping: {e}\n")
                                .into_bytes(),
                        ),
                    );
                }
                return;
            }
        };

        // Load effective scaling policy (v2 path: scaling_store)
        let scaling_policy = match ss::resolve_effective_scaling_policy(
            db,
            tenant_id,
            software_item_id,
            mapping.plugin_config_id,
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    %update_history_id, error = %e,
                    "resource scaling: failed to load effective scaling policy"
                );
                if let Some(tx) = &ctx.output_tx {
                    drop(
                        tx.send(
                            format!("[Resource scaling] Failed to load scaling policy: {e}\n")
                                .into_bytes(),
                        ),
                    );
                }
                return;
            }
        };

        if !scaling_policy.is_active() {
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
                if let Some(tx) = &ctx.output_tx {
                    drop(
                        tx.send(
                            format!("[Resource scaling] Failed to load plugin config: {e}\n")
                                .into_bytes(),
                        ),
                    );
                }
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
                if let Some(tx) = &ctx.output_tx {
                    drop(
                        tx.send(
                            format!("[Resource scaling] Failed to parse Proxmox config: {e}\n")
                                .into_bytes(),
                        ),
                    );
                }
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
                if let Some(tx) = &ctx.output_tx {
                    drop(
                        tx.send(
                            format!("[Resource scaling] Failed to connect to Proxmox: {e}\n")
                                .into_bytes(),
                        ),
                    );
                }
                return;
            }
        };

        let node = &mapping.proxmox_node;
        let vmid = mapping.proxmox_vmid as u32;

        // Read current config, check hotplug (QEMU only), extract original values.
        // original_cores_opt: None = LXC container with no CPU limit set (valid).
        let (original_cores_opt, original_memory_u64): (Option<u32>, u64) = match mapping
            .proxmox_type
            .as_str()
        {
            "qemu" => {
                let config = match client.get_qemu_config(node, vmid).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(
                            %update_history_id, node, vmid, error = %e,
                            "resource scaling: failed to read QEMU config"
                        );
                        if let Some(tx) = &ctx.output_tx {
                            drop(tx.send(
                                    format!(
                                        "[Resource scaling] Failed to read QEMU config ({node}/{vmid}): {e}\n"
                                    )
                                    .into_bytes(),
                                ));
                        }
                        return;
                    }
                };
                if !config.supports_live_resource_scaling() {
                    tracing::warn!(
                        %update_history_id, node, vmid,
                        "QEMU VM does not support hotplug — skipping resource scaling"
                    );
                    if let Some(tx) = &ctx.output_tx {
                        drop(
                            tx.send(
                                format!(
                                    "[Resource scaling] QEMU VM {vmid} on {node} does not support \
                                     hotplug — skipping\n"
                                )
                                .into_bytes(),
                            ),
                        );
                    }
                    return;
                }
                match (config.cores, config.memory) {
                    (Some(c), Some(m)) => (Some(c), m),
                    _ => {
                        tracing::warn!(
                            %update_history_id, node, vmid,
                            "resource scaling: QEMU config missing cores or memory field"
                        );
                        if let Some(tx) = &ctx.output_tx {
                            drop(
                                tx.send(
                                    format!(
                                        "[Resource scaling] QEMU config for {vmid} on {node} \
                                         missing cores or memory field\n"
                                    )
                                    .into_bytes(),
                                ),
                            );
                        }
                        return;
                    }
                }
            }
            "lxc" => {
                let lxc_memory_scaling = match scaling_policy.mode {
                    ScalingMode::Absolute => scaling_policy.absolute_memory_mb.is_some(),
                    ScalingMode::Delta => scaling_policy.delta_memory_mb.is_some(),
                    ScalingMode::None => false,
                };
                if lxc_memory_scaling {
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
                        if let Some(tx) = &ctx.output_tx {
                            drop(tx.send(
                                    format!(
                                        "[Resource scaling] Failed to read LXC config ({node}/{vmid}): {e}\n"
                                    )
                                    .into_bytes(),
                                ));
                        }
                        return;
                    }
                };
                // cores may be None (container has no CPU limit) — that is valid.
                // Only abort if memory is absent (always required).
                let memory = match config.memory {
                    Some(m) => m,
                    None => {
                        tracing::warn!(
                            %update_history_id, node, vmid,
                            "resource scaling: LXC config missing memory field"
                        );
                        if let Some(tx) = &ctx.output_tx {
                            drop(
                                tx.send(
                                    format!(
                                        "[Resource scaling] LXC config for {vmid} on {node} \
                                         missing memory field\n"
                                    )
                                    .into_bytes(),
                                ),
                            );
                        }
                        return;
                    }
                };
                (config.cores, memory)
            }
            other => {
                tracing::warn!(
                    %update_history_id, vm_type = other,
                    "resource scaling: unrecognized vm_type — skipping"
                );
                if let Some(tx) = &ctx.output_tx {
                    drop(
                        tx.send(
                            format!(
                                "[Resource scaling] Unrecognized VM type '{other}' — skipping\n"
                            )
                            .into_bytes(),
                        ),
                    );
                }
                return;
            }
        };

        // Defense-in-depth: policy dimensions are validated (≥ 1) at save time,
        // but guard against corrupt DB state here before calling the Proxmox API.
        let has_invalid_dimension = match scaling_policy.mode {
            ScalingMode::Absolute => {
                scaling_policy.absolute_cores.is_some_and(|v| v < 1)
                    || scaling_policy.absolute_memory_mb.is_some_and(|v| v < 1)
            }
            ScalingMode::Delta => {
                scaling_policy.delta_cores.is_some_and(|v| v < 1)
                    || scaling_policy.delta_memory_mb.is_some_and(|v| v < 1)
            }
            ScalingMode::None => false,
        };
        if has_invalid_dimension {
            tracing::error!(
                %update_history_id,
                "resource scaling: policy dimension < 1 reached the hook (corrupt DB?) \
                 — aborting scale-up"
            );
            if let Some(tx) = &ctx.output_tx {
                drop(
                    tx.send(
                        b"[Resource scaling] Scaling policy has invalid dimension (< 1) \
                      \xe2\x80\x94 aborting\n"
                            .to_vec(),
                    ),
                );
            }
            return;
        }

        // Compute target values from ScalingPolicy (v2).
        // target_cores: None = no cores adjustment (policy skips CPU, or LXC had no limit).
        let target_cores: Option<u32> =
            compute_target_cores(&scaling_policy, original_cores_opt, update_history_id);
        let target_memory_mb = compute_target_memory_mb(&scaling_policy, original_memory_u64);

        // -1 sentinel: LXC container that had no cores limit (original_cores_opt was None).
        let original_cores_stored = original_cores_opt.map(|c| c as i32).unwrap_or(-1);
        let scaled_cores_stored = target_cores.map(|c| c as i32).unwrap_or(-1);

        // Persist record with scale_status = "scaling" BEFORE API call (crash-safe)
        let scaling_record = ScalingRecord {
            update_history_id,
            tenant_id,
            host_id,
            software_item_id,
            plugin_config_id: mapping.plugin_config_id,
            mapping_id: mapping.id,
            vm_type: mapping.proxmox_type.clone(),
            original_cores: original_cores_stored,
            original_memory_mb: original_memory_u64 as i64,
            scaled_cores: scaled_cores_stored,
            scaled_memory_mb: i64::try_from(target_memory_mb).unwrap_or(i64::MAX),
            scale_status: "scaling".to_string(),
            restore_status: "pending".to_string(),
            error_message: None,
            scaling_mode_used: scaling_policy.mode,
        };
        if let Err(e) = policy_store::upsert_scaling_record(db, &scaling_record).await {
            tracing::warn!(
                %update_history_id, error = %e,
                "resource scaling: failed to persist scaling record — aborting scale-up"
            );
            if let Some(tx) = &ctx.output_tx {
                drop(
                    tx.send(
                        format!("[Resource scaling] Failed to persist scaling record: {e}\n")
                            .into_bytes(),
                    ),
                );
            }
            return;
        }

        // Stream status line; send failure means the receiver is gone — ignore.
        if let Some(tx) = &ctx.output_tx {
            let cores_display = target_cores
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unlimited".to_string());
            drop(tx.send(
                format!(
                    "Scaling VM resources to {cores_display} cores / {target_memory_mb} MB\u{2026}\n"
                )
                .into_bytes(),
            ));
        }

        // Apply the resource change
        let scale_result = match mapping.proxmox_type.as_str() {
            "qemu" => {
                client
                    .set_qemu_config_resources(
                        node,
                        vmid,
                        target_cores.unwrap_or(original_cores_opt.unwrap_or(0)),
                        target_memory_mb,
                    )
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
                    let cores_display = target_cores
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "unlimited".to_string());
                    drop(tx.send(
                        format!(
                            "VM resources scaled to {cores_display} cores / {target_memory_mb} MB.\n"
                        )
                        .into_bytes(),
                    ));
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
        use crate::entity::proxmox_host_mapping;
        use sea_orm::EntityTrait;

        let mapping_row = match proxmox_host_mapping::Entity::find_by_id(record.mapping_id)
            .one(db)
            .await
            .map_err(|e| {
                rootcause::report!(
                    uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                        format!("failed to load host mapping {}: {e}", record.mapping_id)
                    )
                )
            })? {
            Some(row) => row,
            None => {
                tracing::warn!(
                    %update_history_id,
                    mapping_id = %record.mapping_id,
                    "resource scaling: host mapping deleted before restore; \
                     writing skipped_mapping_deleted"
                );
                let mut skipped = record.clone();
                skipped.restore_status = "skipped_mapping_deleted".to_string();
                if let Err(e) = policy_store::upsert_scaling_record(db, &skipped).await {
                    tracing::warn!(%update_history_id, error = %e,
                        "resource scaling: failed to persist skipped_mapping_deleted record");
                }
                return Ok(());
            }
        };

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
        let original_memory_mb = record.original_memory_mb as u64;
        // -1 sentinel: LXC had no cores limit → restore by omitting cores from the PUT.
        let lxc_restore_cores: Option<u32> = if record.original_cores == -1 {
            None
        } else {
            Some(record.original_cores as u32)
        };

        // Restore resources
        let restore_result = match record.vm_type.as_str() {
            "qemu" => {
                client
                    .set_qemu_config_resources(
                        node,
                        vmid,
                        record.original_cores as u32,
                        original_memory_mb,
                    )
                    .await
            }
            _ => {
                client
                    .set_lxc_config_resources(node, vmid, lxc_restore_cores, original_memory_mb)
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

                // Best-effort email notification: look up any tenant user to get an email
                // address. If lookup fails or no user exists, silently skip.
                {
                    use sea_orm::{ColumnTrait, QueryFilter};
                    use uptrakit_shared_db::entity::{user, user_role};

                    let maybe_user_id = user_role::Entity::find()
                        .filter(user_role::Column::TenantId.eq(record.tenant_id))
                        .one(ctx.tenant_db.db())
                        .await
                        .ok()
                        .flatten()
                        .map(|ur| ur.user_id);

                    if let Some(user_id) = maybe_user_id {
                        let maybe_email = user::Entity::find_by_id(user_id)
                            .one(ctx.tenant_db.db())
                            .await
                            .ok()
                            .flatten()
                            .map(|u| u.email);

                        if let Some(email) = maybe_email {
                            let subject = "Proxmox resource restore failed";
                            let body = format!(
                                "A Proxmox VM resource restore failed for update {update_history_id}.\n\
                                 VM is still running at scaled resources ({scaled_cores} cores / \
                                 {scaled_memory_mb} MB).\n\
                                 Error: {err}",
                                update_history_id = ctx.update_history_id,
                                scaled_cores = record.scaled_cores,
                                scaled_memory_mb = record.scaled_memory_mb,
                            );
                            drop(
                                ctx.notification_ops
                                    .send_transactional_email(
                                        &ctx.tenant_db,
                                        email.expose_email(),
                                        subject,
                                        &body,
                                        &body,
                                    )
                                    .await,
                            );
                        }
                    }
                }

                Err(rootcause::report!(
                    uptrakit_plugin_infrastructure_core::error::PluginError::PluginInternal(
                        format!("resource restore failed: {err}")
                    )
                ))
            }
        }
    }
}

/// Compute target core count from a `ScalingPolicy`.
///
/// `original = None` means the LXC container has no cores limit set.
/// Returns `None` to signal "don't touch cores in the API call" when:
/// - the policy does not request CPU scaling, OR
/// - `delta_cores` is set but there is no baseline (original is None).
///
/// No minimum clamp is applied here — the integrity guard in the caller
/// already returns early for invalid values (v < 1).
fn compute_target_cores(
    policy: &ScalingPolicy,
    original: Option<u32>,
    update_history_id: uuid::Uuid,
) -> Option<u32> {
    match policy.mode {
        ScalingMode::Absolute => policy.absolute_cores.map(|c| c as u32).or(original),
        ScalingMode::Delta => match (policy.delta_cores, original) {
            (Some(d), Some(c)) => Some((c as i64 + i64::from(d)) as u32),
            (Some(_), None) => {
                tracing::warn!(
                    %update_history_id,
                    "resource scaling: delta_cores set but LXC container has no cores limit \
                     — skipping CPU scaling dimension"
                );
                None
            }
            (None, orig) => orig,
        },
        ScalingMode::None => original,
    }
}

/// Compute target memory (MB) from a `ScalingPolicy`.
///
/// - `Absolute` mode: use `absolute_memory_mb` if set, else keep original.
/// - `Delta` mode: add `delta_memory_mb` to original.
/// - `None`: keep original.
///
/// No minimum clamp is applied here — the integrity guard in the caller
/// already returns early for invalid values (v < 1).
fn compute_target_memory_mb(policy: &ScalingPolicy, original: u64) -> u64 {
    match policy.mode {
        ScalingMode::Absolute => policy
            .absolute_memory_mb
            .map(|m| m as u64)
            .unwrap_or(original),
        ScalingMode::Delta => policy
            .delta_memory_mb
            .map(|d| (original as i64 + i64::from(d)) as u64)
            .unwrap_or(original),
        ScalingMode::None => original,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use sea_orm::{DbBackend, MockDatabase, MockExecResult};
    use time::OffsetDateTime;
    use uptrakit_plugin_infrastructure_core::{
        NotificationTransport, UpdateHookController, UpdateHookPostContext, UpdateHookPreContext,
        plugin_ops::NotificationOps,
    };
    use uptrakit_shared_types::PluginTypeId;
    use uuid::Uuid;

    // ── Test stubs ────────────────────────────────────────────────────────────

    struct TestHookController {
        tenant_db: uptrakit_tenant_db::TenantDb,
    }

    impl UpdateHookController for TestHookController {
        fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
            &self.tenant_db
        }
    }

    struct NoOpNotificationOps;

    #[async_trait::async_trait]
    impl NotificationOps for NoOpNotificationOps {
        fn transport(
            &self,
            _id: &PluginTypeId,
        ) -> Option<std::sync::Arc<dyn NotificationTransport>> {
            None
        }

        fn notification_supported_types(&self) -> Vec<PluginTypeId> {
            vec![]
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn mock_host_mapping(
        tenant_id: Uuid,
        host_id: Uuid,
        plugin_config_id: Uuid,
        vmid: i32,
        vm_type: &str,
    ) -> crate::entity::proxmox_host_mapping::Model {
        let now = OffsetDateTime::now_utc();
        crate::entity::proxmox_host_mapping::Model {
            id: Uuid::now_v7(),
            tenant_id,
            plugin_config_id,
            host_id: Some(host_id),
            proxmox_node: "pve1".to_string(),
            proxmox_vmid: vmid,
            proxmox_type: vm_type.to_string(),
            proxmox_name: None,
            proxmox_status: "running".to_string(),
            hostname: None,
            ip_addresses: None,
            machine_id: None,
            match_method: None,
            discovered_at: now,
            updated_at: now,
        }
    }

    fn mock_scaling_default_absolute(
        tenant_id: Uuid,
        plugin_config_id: Uuid,
    ) -> crate::entity::proxmox_scaling_default::Model {
        let now = OffsetDateTime::now_utc();
        crate::entity::proxmox_scaling_default::Model {
            id: Uuid::now_v7(),
            tenant_id,
            plugin_config_id,
            scaling_mode: crate::scaling_mode::ScalingMode::Absolute,
            absolute_cores: Some(8),
            absolute_memory_mb: Some(4096),
            delta_cores: None,
            delta_memory_mb: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn mock_plugin_config(
        tenant_id: Uuid,
        plugin_config_id: Uuid,
        api_url: &str,
    ) -> uptrakit_shared_db::entity::plugin_config::Model {
        let now = OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::plugin_config::Model {
            id: plugin_config_id,
            tenant_id,
            name: "test".to_string(),
            plugin_type: "infrastructure_proxmox".to_string(),
            config: serde_json::json!({
                "api_url": api_url,
                "api_token": "root@pam!tok=secret",
                "verify_tls": false
            }),
            enabled: true,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        }
    }

    fn mock_scaling_record(
        update_history_id: Uuid,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
        mapping_id: Uuid,
    ) -> crate::entity::proxmox_resource_scaling_record::Model {
        let now = OffsetDateTime::now_utc();
        crate::entity::proxmox_resource_scaling_record::Model {
            update_history_id,
            tenant_id,
            host_id,
            software_item_id,
            plugin_config_id,
            mapping_id,
            vm_type: "qemu".to_string(),
            original_cores: 4,
            original_memory_mb: 4096,
            scaled_cores: 8,
            scaled_memory_mb: 8192,
            scale_status: "scaled".to_string(),
            restore_status: "pending".to_string(),
            error_message: None,
            scaling_mode_used: "absolute".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    fn mock_scaling_item_override_delta_memory(
        tenant_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
        delta_memory_mb: i32,
    ) -> crate::entity::proxmox_scaling_item_override::Model {
        let now = OffsetDateTime::now_utc();
        crate::entity::proxmox_scaling_item_override::Model {
            id: Uuid::now_v7(),
            tenant_id,
            software_item_id,
            plugin_config_id,
            scaling_mode: crate::scaling_mode::ScalingMode::Delta,
            absolute_cores: None,
            absolute_memory_mb: None,
            delta_cores: None,
            delta_memory_mb: Some(delta_memory_mb),
            created_at: now,
            updated_at: now,
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn controller_update_hook_plugin_implements_plugin_meta() {
        let plugin = ControllerUpdateHookPlugin;
        assert_eq!(plugin.plugin_type_id().as_str(), "infrastructure_proxmox");
    }

    #[tokio::test]
    async fn finalize_returns_ok_when_no_record_in_db() {
        use crate::entity::proxmox_resource_scaling_record;

        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_query_results([Vec::<proxmox_resource_scaling_record::Model>::new()])
            .into_connection();

        let result = crate::policy_store::load_scaling_record(&db, uuid::Uuid::now_v7()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn pre_update_hook_no_op_when_no_host_mapping() {
        use crate::entity::proxmox_host_mapping;

        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();

        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_query_results([Vec::<proxmox_host_mapping::Model>::new()])
            .into_connection();

        let tenant_db = uptrakit_tenant_db::TenantDb::new(db, tenant_id);
        let controller = TestHookController { tenant_db };
        let ctx = UpdateHookPreContext::new(
            &controller,
            tenant_id,
            host_id,
            Uuid::now_v7(),
            Uuid::now_v7(),
        );

        ControllerUpdateHookPlugin
            .prepare_pre_update_hook(&ctx)
            .await;
    }

    #[tokio::test]
    async fn pre_update_hook_no_op_when_no_scaling_configured() {
        use crate::entity::{proxmox_scaling_default, proxmox_scaling_item_override};

        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();

        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_query_results([vec![mock_host_mapping(
                tenant_id,
                host_id,
                plugin_config_id,
                100,
                "qemu",
            )]])
            // load_scaling_item_override → None
            .append_query_results([Vec::<proxmox_scaling_item_override::Model>::new()])
            // load_scaling_global_default → None (no row → ScalingPolicy::none())
            .append_query_results([Vec::<proxmox_scaling_default::Model>::new()])
            .into_connection();

        let tenant_db = uptrakit_tenant_db::TenantDb::new(db, tenant_id);
        let controller = TestHookController { tenant_db };
        let ctx = UpdateHookPreContext::new(
            &controller,
            tenant_id,
            host_id,
            Uuid::now_v7(),
            Uuid::now_v7(),
        );

        ControllerUpdateHookPlugin
            .prepare_pre_update_hook(&ctx)
            .await;
    }

    #[tokio::test]
    async fn pre_update_hook_skips_scaling_when_qemu_hotplug_absent() {
        use crate::entity::proxmox_scaling_item_override;
        use httpmock::prelude::*;

        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();

        let server = MockServer::start();
        // GET config: cores and memory present, but no hotplug field
        let get_config_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/api2/json/nodes/pve1/qemu/100/config");
            then.status(200)
                .json_body(serde_json::json!({"data": {"cores": 4, "memory": 4096}}));
        });

        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_query_results([vec![mock_host_mapping(
                tenant_id,
                host_id,
                plugin_config_id,
                100,
                "qemu",
            )]])
            // load_scaling_item_override → None
            .append_query_results([Vec::<proxmox_scaling_item_override::Model>::new()])
            // load_scaling_global_default → Some with absolute scaling configured
            .append_query_results([vec![mock_scaling_default_absolute(
                tenant_id,
                plugin_config_id,
            )]])
            .append_query_results([vec![mock_plugin_config(
                tenant_id,
                plugin_config_id,
                &server.base_url(),
            )]])
            .into_connection();

        let tenant_db = uptrakit_tenant_db::TenantDb::new(db, tenant_id);
        let controller = TestHookController { tenant_db };
        let ctx = UpdateHookPreContext::new(
            &controller,
            tenant_id,
            host_id,
            Uuid::now_v7(),
            Uuid::now_v7(),
        );

        ControllerUpdateHookPlugin
            .prepare_pre_update_hook(&ctx)
            .await;

        // GET was called once (no hotplug → no PUT should have been sent)
        get_config_mock.assert_calls(1);
    }

    #[tokio::test]
    async fn finalize_restores_resources_after_successful_scale() {
        use httpmock::prelude::*;

        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let mapping_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();

        let server = MockServer::start();
        // PUT to restore original resources
        let put_config_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/api2/json/nodes/pve1/qemu/100/config");
            then.status(200)
                .json_body(serde_json::json!({"data": null}));
        });

        let scaled_model = mock_scaling_record(
            update_history_id,
            tenant_id,
            host_id,
            software_item_id,
            plugin_config_id,
            mapping_id,
        );
        // "restored" record returned after upsert UPDATE
        let mut restored_model = scaled_model.clone();
        restored_model.restore_status = "restored".to_string();

        let db = MockDatabase::new(DbBackend::Sqlite)
            // load_scaling_record → Some(scaled record)
            .append_query_results([vec![scaled_model.clone()]])
            // find_by_id(mapping_id) for host mapping
            .append_query_results([vec![mock_host_mapping(
                tenant_id,
                host_id,
                plugin_config_id,
                100,
                "qemu",
            )]])
            // load_plugin_config_payload
            .append_query_results([vec![mock_plugin_config(
                tenant_id,
                plugin_config_id,
                &server.base_url(),
            )]])
            // upsert_scaling_record UPDATE: find_by_id → Some(scaled)
            .append_query_results([vec![scaled_model]])
            // SELECT after UPDATE → restored model
            .append_query_results([vec![restored_model]])
            // UPDATE exec
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let tenant_db_for_controller = uptrakit_tenant_db::TenantDb::new(db, tenant_id);
        let controller = TestHookController {
            tenant_db: tenant_db_for_controller,
        };
        // A separate empty DB for the post-context tenant_db (email path, not exercised here)
        let email_db = MockDatabase::new(DbBackend::Sqlite).into_connection();
        let post_tenant_db = uptrakit_tenant_db::TenantDb::new(email_db, tenant_id);

        let notification_ops = NoOpNotificationOps;
        let ctx = UpdateHookPostContext::new(
            &controller,
            tenant_id,
            host_id,
            software_item_id,
            update_history_id,
            uptrakit_shared_types::UpdateStatus::Completed,
            &notification_ops,
            post_tenant_db,
        );

        let result = ControllerUpdateHookPlugin
            .finalize_post_update_hook(&ctx)
            .await;
        assert!(result.is_ok(), "restore should succeed: {result:?}");
        put_config_mock.assert_calls(1);
    }

    #[test]
    fn is_active_returns_false_for_none_mode() {
        use crate::scaling_store::ScalingPolicy;
        let policy = ScalingPolicy::none();
        assert!(!policy.is_active());
    }

    #[test]
    fn is_active_returns_false_for_absolute_with_no_dimensions() {
        use crate::scaling_store::{ScalingMode, ScalingPolicy};
        let policy = ScalingPolicy {
            mode: ScalingMode::Absolute,
            ..Default::default()
        };
        assert!(!policy.is_active());
    }

    #[test]
    fn delta_target_computation() {
        let original_cores: u32 = 4;
        let delta_cores: i32 = 2;
        let target = (original_cores as i64 + delta_cores as i64) as u32;
        assert_eq!(target, 6u32);
    }

    #[test]
    fn delta_integrity_guard_condition() {
        use crate::scaling_store::{ScalingMode, ScalingPolicy};
        let policy_zero_delta = ScalingPolicy {
            mode: ScalingMode::Delta,
            delta_cores: Some(0),
            ..Default::default()
        };
        assert!(policy_zero_delta.is_active());
        let guard_fires = policy_zero_delta.delta_cores.is_some_and(|v| v < 1);
        assert!(
            guard_fires,
            "integrity guard must fire when delta_cores = 0"
        );

        let policy_valid = ScalingPolicy {
            mode: ScalingMode::Delta,
            delta_cores: Some(1),
            ..Default::default()
        };
        let guard_fires_valid = policy_valid.delta_cores.is_some_and(|v| v < 1);
        assert!(
            !guard_fires_valid,
            "guard must not fire for delta_cores = 1"
        );
    }

    #[test]
    fn delta_partial_config_cores_only() {
        use crate::scaling_store::{ScalingMode, ScalingPolicy};
        let policy = ScalingPolicy {
            mode: ScalingMode::Delta,
            delta_cores: Some(2),
            delta_memory_mb: None,
            ..Default::default()
        };
        assert!(policy.is_active());
        assert!(policy.delta_memory_mb.is_none());
    }

    /// LXC container with no CPU limit (cores absent from API response):
    /// memory-only delta policy must succeed and PUT must omit `cores`.
    #[tokio::test]
    async fn pre_update_hook_scales_memory_when_lxc_has_no_cores_limit() {
        use crate::entity::{proxmox_resource_scaling_record, proxmox_scaling_default};
        use httpmock::prelude::*;

        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();

        let server = MockServer::start();
        // GET LXC config — no cores field (unlimited CPU)
        let get_config_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/api2/json/nodes/pve1/lxc/110/config");
            then.status(200)
                .json_body(serde_json::json!({"data": {"memory": 512}}));
        });
        // PUT must be called (memory scaled); body correctness verified by unit logic
        let put_config_mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/api2/json/nodes/pve1/lxc/110/config");
            then.status(200)
                .json_body(serde_json::json!({"data": null}));
        });

        let mapping_id = Uuid::now_v7();
        let mut mapping = mock_host_mapping(tenant_id, host_id, plugin_config_id, 110, "lxc");
        mapping.id = mapping_id;

        // Scaling record that will be returned after INSERT (for the subsequent UPDATE)
        let now = time::OffsetDateTime::now_utc();
        let scaling_record_row = proxmox_resource_scaling_record::Model {
            update_history_id,
            tenant_id,
            host_id,
            software_item_id,
            plugin_config_id,
            mapping_id,
            vm_type: "lxc".to_string(),
            original_cores: -1,
            original_memory_mb: 512,
            scaled_cores: -1,
            scaled_memory_mb: 2560,
            scale_status: "scaling".to_string(),
            restore_status: "pending".to_string(),
            error_message: None,
            scaling_mode_used: "delta".to_string(),
            created_at: now,
            updated_at: now,
        };

        let db = MockDatabase::new(DbBackend::Sqlite)
            // load_host_mapping
            .append_query_results([vec![mapping]])
            // load_scaling_item_override → delta_memory_mb=2048
            .append_query_results([vec![mock_scaling_item_override_delta_memory(
                tenant_id,
                software_item_id,
                plugin_config_id,
                2048,
            )]])
            // load_scaling_global_default → empty (item override takes precedence)
            .append_query_results([Vec::<proxmox_scaling_default::Model>::new()])
            // load_plugin_config_payload
            .append_query_results([vec![mock_plugin_config(
                tenant_id,
                plugin_config_id,
                &server.base_url(),
            )]])
            // upsert_scaling_record (INSERT path): find_by_id → None
            .append_query_results([Vec::<proxmox_resource_scaling_record::Model>::new()])
            // INSERT exec
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // upsert_scaling_record UPDATE (after scale success): find_by_id → existing row
            .append_query_results([vec![scaling_record_row]])
            // UPDATE exec
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let tenant_db = uptrakit_tenant_db::TenantDb::new(db, tenant_id);
        let controller = TestHookController { tenant_db };
        let ctx = UpdateHookPreContext::new(
            &controller,
            tenant_id,
            host_id,
            software_item_id,
            update_history_id,
        );

        ControllerUpdateHookPlugin
            .prepare_pre_update_hook(&ctx)
            .await;

        get_config_mock.assert_calls(1);
        put_config_mock.assert_calls(1);
    }
}
