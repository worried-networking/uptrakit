use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use uptrakit_plugin_infrastructure_proxmox::testing as proxmox_testing;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, service, service_host,
    software_item, tenant, update_history,
};
use uptrakit_shared_types::ServiceStatus;
use uptrakit_web_api_queries::queries::update_dispatch::ValidatedUpdateTarget;
use uptrakit_web_api_queries::queries::update_triggers::PendingProtectionWork;

pub(crate) struct TestFixtures {
    pub(crate) tenant_id: Uuid,
    pub(crate) host_id: Uuid,
    pub(crate) service_id: Uuid,
    pub(crate) software_item_id: Uuid,
    pub(crate) host_software_item_id: Uuid,
    pub(crate) shell_config_id: Uuid,
    pub(crate) proxmox_config_id: Uuid,
    pub(crate) execute_update_plugin_id: Uuid,
    pub(crate) detect_version_plugin_id: Uuid,
    pub(crate) update_history_id: Uuid,
}

impl TestFixtures {
    pub(crate) async fn insert(db: &DatabaseConnection, proxmox_api_url: &str) -> Self {
        let now = OffsetDateTime::now_utc();

        let tenant_id = Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set("test-tenant".to_string()),
            is_default: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");

        let host_id = Uuid::now_v7();
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set("test-machine".to_string()),
            hostname: Set("test-host".to_string()),
            friendly_name: Set("test-host".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host");

        let service_id = Uuid::now_v7();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set(String::new()),
            hostname: Set("test-agent-host".to_string()),
            friendly_name: Set("test-agent".to_string()),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("test-hash-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert service");

        service_host::ActiveModel {
            service_id: Set(service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert service_host");

        let software_item_id = Uuid::now_v7();
        software_item::ActiveModel {
            id: Set(software_item_id),
            tenant_id: Set(tenant_id),
            name: Set("test-software".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            awaiting_restart_timeout: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software_item");

        let host_software_item_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(host_software_item_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(None),
            plugin_config_id: Set(None),
            package_identifier: Set(None),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(None),
            last_discovered_at: Set(None),
            discovery_source: Set(None),
            missing_since: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host_software_item");

        let shell_config_id = Uuid::now_v7();
        plugin_config::ActiveModel {
            id: Set(shell_config_id),
            tenant_id: Set(tenant_id),
            name: Set("test-shell-config".to_string()),
            plugin_type: Set("generic.shell".to_string()),
            config: Set(
                uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig::from_json(&json!({
                    "update_command": "echo ok",
                    "version_command": "echo 1.0.0"
                }))
                .expect("encrypt test config"),
            ),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            credential_updated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert shell plugin_config");

        let proxmox_config_id = Uuid::now_v7();
        plugin_config::ActiveModel {
            id: Set(proxmox_config_id),
            tenant_id: Set(tenant_id),
            name: Set("test-proxmox-config".to_string()),
            plugin_type: Set("infrastructure.proxmox".to_string()),
            config: Set(
                uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig::from_json(&json!({
                    "api_url": proxmox_api_url,
                    "api_token": "root@pam!tok=secret",
                    "verify_tls": false
                }))
                .expect("encrypt test config"),
            ),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            credential_updated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert proxmox plugin_config");

        let execute_update_plugin_id = Uuid::now_v7();
        host_software_item_plugin::ActiveModel {
            id: Set(execute_update_plugin_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(Some(shell_config_id)),
            plugin_type: Set("generic.shell".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("test-shell-pkg".to_string()),
            config: Set(None),
            execution_site: Set("agent".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert execute_update plugin");

        let detect_version_plugin_id = Uuid::now_v7();
        host_software_item_plugin::ActiveModel {
            id: Set(detect_version_plugin_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(Some(shell_config_id)),
            plugin_type: Set("generic.shell".to_string()),
            role: Set("detect_version".to_string()),
            ordinal: Set(0),
            package_identifier: Set("test-shell-pkg".to_string()),
            config: Set(None),
            execution_site: Set("agent".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert detect_version plugin");

        let update_history_id = Uuid::now_v7();
        update_history::ActiveModel {
            id: Set(update_history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(Some(host_software_item_id)),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("2.0.0".to_string())),
            status: Set(update_history::UpdateStatus::Pending),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("test".to_string()),
            actor_id: Set("functional-tests".to_string()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(None),
            completed_at: Set(None),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("unknown".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .expect("insert update_history");

        Self {
            tenant_id,
            host_id,
            service_id,
            software_item_id,
            host_software_item_id,
            shell_config_id,
            proxmox_config_id,
            execute_update_plugin_id,
            detect_version_plugin_id,
            update_history_id,
        }
    }

    pub(crate) async fn pending_work(
        &self,
        db: &DatabaseConnection,
        to_version: &str,
    ) -> PendingProtectionWork {
        let item = software_item::Entity::find_by_id(self.software_item_id)
            .one(db)
            .await
            .unwrap()
            .expect("software_item row");
        let host = host::Entity::find_by_id(self.host_id)
            .one(db)
            .await
            .unwrap()
            .expect("host row");
        let hsi_link = host_software_item::Entity::find_by_id(self.host_software_item_id)
            .one(db)
            .await
            .unwrap()
            .expect("host_software_item row");
        let agent = service::Entity::find_by_id(self.service_id)
            .one(db)
            .await
            .unwrap()
            .expect("service row");
        let shell_cfg = plugin_config::Entity::find_by_id(self.shell_config_id)
            .one(db)
            .await
            .unwrap()
            .expect("shell plugin_config row");
        let execute_plugin =
            host_software_item_plugin::Entity::find_by_id(self.execute_update_plugin_id)
                .one(db)
                .await
                .unwrap()
                .expect("execute_update plugin row");
        let detect_plugin =
            host_software_item_plugin::Entity::find_by_id(self.detect_version_plugin_id)
                .one(db)
                .await
                .unwrap()
                .expect("detect_version plugin row");

        let target = ValidatedUpdateTarget {
            item,
            host,
            hsi_link,
            agent,
            execute_update_data: (execute_plugin, Some(shell_cfg.clone())),
            detect_version_data: Some((detect_plugin, Some(shell_cfg))),
            fetch_releases_config: None,
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
        };

        PendingProtectionWork {
            target,
            update_history_id: self.update_history_id,
            to_version: to_version.to_string(),
            release_info: None,
            interactive: false,
        }
    }

    pub(crate) async fn insert_proxmox_mapping(
        &self,
        db: &DatabaseConnection,
        node: &str,
        vmid: i32,
        vm_type: &str,
    ) -> Uuid {
        proxmox_testing::insert_host_mapping(
            db,
            self.tenant_id,
            self.proxmox_config_id,
            self.host_id,
            node,
            vmid,
            vm_type,
        )
        .await
    }

    pub(crate) async fn insert_protection_default_snapshot(&self, db: &DatabaseConnection) {
        proxmox_testing::insert_protection_default(
            db,
            self.tenant_id,
            self.proxmox_config_id,
            "snapshot",
            None,
        )
        .await;
    }

    pub(crate) async fn insert_protection_default_backup(
        &self,
        db: &DatabaseConnection,
        backup_target_key: &str,
    ) {
        proxmox_testing::insert_protection_default(
            db,
            self.tenant_id,
            self.proxmox_config_id,
            "backup",
            Some(backup_target_key),
        )
        .await;
    }

    pub(crate) async fn insert_protection_default_do_nothing(&self, db: &DatabaseConnection) {
        proxmox_testing::insert_protection_default(
            db,
            self.tenant_id,
            self.proxmox_config_id,
            "do_nothing",
            None,
        )
        .await;
    }

    pub(crate) async fn insert_scaling_default_delta(
        &self,
        db: &DatabaseConnection,
        delta_cores: i32,
        delta_memory_mb: i32,
    ) {
        proxmox_testing::insert_scaling_default_delta(
            db,
            self.tenant_id,
            self.proxmox_config_id,
            delta_cores,
            delta_memory_mb,
        )
        .await;
    }

    pub(crate) async fn insert_scaling_default_absolute(
        &self,
        db: &DatabaseConnection,
        absolute_cores: i32,
        absolute_memory_mb: i32,
    ) {
        proxmox_testing::insert_scaling_default_absolute(
            db,
            self.tenant_id,
            self.proxmox_config_id,
            absolute_cores,
            absolute_memory_mb,
        )
        .await;
    }

    pub(crate) async fn insert_backup_target_cache(
        &self,
        db: &DatabaseConnection,
        node: &str,
        storage_id: &str,
        storage_type: &str,
        target_key: &str,
    ) {
        proxmox_testing::insert_backup_target_cache(
            db,
            self.tenant_id,
            self.proxmox_config_id,
            node,
            storage_id,
            storage_type,
            target_key,
        )
        .await;
    }
}
