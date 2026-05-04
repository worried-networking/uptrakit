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

        // Stream status line; send failure means the receiver is gone — ignore.
        if let Some(tx) = &ctx.output_tx {
            drop(tx.send(
                format!(
                    "Scaling VM resources to {target_cores} cores / {target_memory_mb} MB\u{2026}\n"
                )
                .into_bytes(),
            ));
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
                    drop(tx.send(
                        format!(
                            "VM resources scaled to {target_cores} cores / {target_memory_mb} MB.\n"
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

    fn mock_protection_default_with_scaling(
        tenant_id: Uuid,
        plugin_config_id: Uuid,
    ) -> crate::entity::proxmox_protection_default::Model {
        let now = OffsetDateTime::now_utc();
        crate::entity::proxmox_protection_default::Model {
            tenant_id,
            plugin_config_id,
            mode: "do_nothing".to_string(),
            backup_target_key: None,
            snapshot_timeout_seconds: None,
            backup_timeout_seconds: None,
            update_cores: Some(8),
            update_memory_mb: Some(4096),
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
        use crate::entity::{proxmox_protection_default, proxmox_protection_item_override};

        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();

        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_query_results([vec![mock_host_mapping(
                tenant_id,
                host_id,
                plugin_config_id,
                100,
                "qemu",
            )]])
            // load_item_override → None
            .append_query_results([Vec::<proxmox_protection_item_override::Model>::new()])
            // load_global_default → Some, but both scaling fields are None
            .append_query_results([vec![proxmox_protection_default::Model {
                tenant_id,
                plugin_config_id,
                mode: "do_nothing".to_string(),
                backup_target_key: None,
                snapshot_timeout_seconds: None,
                backup_timeout_seconds: None,
                update_cores: None,
                update_memory_mb: None,
                created_at: now,
                updated_at: now,
            }]])
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
        use crate::entity::proxmox_protection_item_override;
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
            .append_query_results([Vec::<proxmox_protection_item_override::Model>::new()])
            .append_query_results([vec![mock_protection_default_with_scaling(
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
}
