use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, JoinType, QueryFilter, QuerySelect,
    RelationTrait,
};
use uptrakit_shared_db::entity::{
    host, host_software_item_plugin, plugin_config, service, service_host, software_item,
};
use uuid::Uuid;

use crate::error::SchedulerError;

// ── Shared result row types ───────────────────────────────────────────────────

/// Row returned from agent-side assignment queries.
#[derive(Debug, sea_orm::FromQueryResult)]
pub(crate) struct AgentAssignmentRow {
    pub(crate) service_id: Uuid,
    pub(crate) host_machine_id: String,
    pub(crate) software_item_id: Uuid,
    pub(crate) software_item_name: String,
    pub(crate) plugin_type: String,
    pub(crate) package_identifier: String,
    /// FK pointing to the specific `host_software_items.id` row this plugin
    /// assignment belongs to. Used to populate
    /// `VersionCheckAssignment::host_software_item_id` so that the controller
    /// can route results back to the correct row when a service manages
    /// multiple hosts (e.g. SSH agent: one service → N remote hosts).
    pub(crate) host_software_item_id: Uuid,
    /// Profile config from `plugin_configs.config`. NULL when `plugin_config_id`
    /// is NULL (package manager assignments after type settings migration).
    pub(crate) profile_config: Option<serde_json::Value>,
    pub(crate) assignment_config: Option<serde_json::Value>,
    pub(crate) execution_site: String,
}

// ── Shared query helpers ──────────────────────────────────────────────────────

/// Query agent-side plugin assignments for the given roles,
/// joined through `host → service_host → service` for routing.
///
/// `roles` is a non-empty slice of role strings (e.g. `&["detect_version"]`
/// or `&["fetch_releases"]` or `&["detect_version", "fetch_releases"]`).
///
/// Uses LEFT JOIN on `plugin_configs` to handle assignments with nullable
/// `plugin_config_id` (package manager assignments after the type settings
/// migration). The `plugin_type` is read from `host_software_item_plugins`
/// (denormalized column) rather than from `plugin_configs`.
pub(crate) async fn query_agent_assignment_rows(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    roles: &[&str],
) -> crate::error::Result<Vec<AgentAssignmentRow>> {
    let role_strings: Vec<String> = roles.iter().map(|s| s.to_string()).collect();

    let rows: Vec<AgentAssignmentRow> = host_software_item_plugin::Entity::find()
        .select_only()
        .column_as(service::Column::Id, "service_id")
        .column_as(host::Column::MachineId, "host_machine_id")
        .column_as(
            host_software_item_plugin::Column::SoftwareItemId,
            "software_item_id",
        )
        .column_as(software_item::Column::Name, "software_item_name")
        // Read plugin_type from the denormalized HSIP column (not plugin_configs).
        .column_as(host_software_item_plugin::Column::PluginType, "plugin_type")
        .column_as(
            host_software_item_plugin::Column::PackageIdentifier,
            "package_identifier",
        )
        .column_as(plugin_config::Column::Config, "profile_config")
        .column_as(
            host_software_item_plugin::Column::Config,
            "assignment_config",
        )
        .column_as(
            host_software_item_plugin::Column::HostSoftwareItemId,
            "host_software_item_id",
        )
        .column_as(
            host_software_item_plugin::Column::ExecutionSite,
            "execution_site",
        )
        .join(
            JoinType::InnerJoin,
            host_software_item_plugin::Relation::SoftwareItem.def(),
        )
        // LEFT JOIN: plugin_config_id may be NULL for package manager assignments.
        .join(
            JoinType::LeftJoin,
            host_software_item_plugin::Relation::PluginConfig.def(),
        )
        .join(
            JoinType::InnerJoin,
            host_software_item_plugin::Relation::Host.def(),
        )
        .join(
            JoinType::InnerJoin,
            service_host::Relation::Host.def().rev(),
        )
        .join(JoinType::InnerJoin, service_host::Relation::Service.def())
        .filter(host_software_item_plugin::Column::Role.is_in(role_strings))
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        // Accept rows where plugin_config_id is NULL (package managers) OR
        // where the linked config is enabled and not deactivated.
        .filter(
            Condition::any()
                .add(host_software_item_plugin::Column::PluginConfigId.is_null())
                .add(
                    Condition::all()
                        .add(plugin_config::Column::Enabled.eq(true))
                        .add(plugin_config::Column::DeactivatedAt.is_null()),
                ),
        )
        .filter(host::Column::DeactivatedAt.is_null())
        .filter(service::Column::DeactivatedAt.is_null())
        .into_model::<AgentAssignmentRow>()
        .all(db)
        .await
        .context_to::<SchedulerError>()?;

    Ok(rows)
}

// ── Config merge ──────────────────────────────────────────────────────────────

/// Merge a base plugin config with per-item overrides.
///
/// Performs a shallow (top-level key) merge: override keys replace base keys.
/// If either value is not a JSON object the base is returned unchanged.
///
/// Superseded by `uptrakit_config_merge::resolve_effective_config` for the
/// three-layer config merge (type_settings + profile + assignment). Retained
/// for its unit tests and potential future use.
#[cfg(test)]
pub(crate) fn merge_config(
    base: &serde_json::Value,
    overrides: &serde_json::Value,
) -> serde_json::Value {
    match (base, overrides) {
        (serde_json::Value::Object(b), serde_json::Value::Object(o)) => {
            let mut merged = b.clone();
            for (k, v) in o {
                merged.insert(k.clone(), v.clone());
            }
            serde_json::Value::Object(merged)
        }
        _ => base.clone(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, service, service_host, software_item,
        tenant,
    };
    use uptrakit_shared_db::migration::run_migrations;

    #[test]
    fn merge_config_objects() {
        let base = serde_json::json!({"key1": "val1", "key2": "val2"});
        let overrides = serde_json::json!({"key2": "override2", "key3": "val3"});
        let merged = merge_config(&base, &overrides);
        assert_eq!(
            merged,
            serde_json::json!({"key1": "val1", "key2": "override2", "key3": "val3"})
        );
    }

    #[test]
    fn merge_config_non_object_override_returns_base() {
        let base = serde_json::json!({"key": "val"});
        let overrides = serde_json::json!("just a string");
        let merged = merge_config(&base, &overrides);
        assert_eq!(merged, base);
    }

    #[test]
    fn merge_config_non_object_base_returns_base() {
        let base = serde_json::json!(42);
        let overrides = serde_json::json!({"key": "val"});
        let merged = merge_config(&base, &overrides);
        assert_eq!(merged, serde_json::json!(42));
    }

    #[test]
    fn merge_config_empty_override() {
        let base = serde_json::json!({"key": "val"});
        let overrides = serde_json::json!({});
        let merged = merge_config(&base, &overrides);
        assert_eq!(merged, base);
    }

    #[test]
    fn merge_config_nested_objects_replaced_not_deep_merged() {
        let base = serde_json::json!({"nested": {"a": 1, "b": 2}});
        let overrides = serde_json::json!({"nested": {"c": 3}});
        let merged = merge_config(&base, &overrides);
        // Shallow merge: the entire "nested" value is replaced.
        assert_eq!(merged, serde_json::json!({"nested": {"c": 3}}));
    }

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();
        db
    }

    async fn insert_tenant(db: &DatabaseConnection, tenant_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test".to_string()),
            slug: Set(tenant_id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_service(db: &DatabaseConnection, tenant_id: Uuid, service_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("agent-host".to_string()),
            friendly_name: Set("Agent".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
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
        .unwrap();
    }

    async fn insert_host(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        host_id: Uuid,
        deactivated_at: Option<OffsetDateTime>,
    ) {
        let now = OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{host_id}")),
            hostname: Set(format!("host-{host_id}")),
            friendly_name: Set(format!("Host {host_id}")),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(deactivated_at),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_software_item(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        software_item_id: Uuid,
    ) {
        let now = OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(software_item_id),
            tenant_id: Set(tenant_id),
            name: Set("Actual Budget".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            awaiting_restart_timeout: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_plugin_assignment(
        db: &DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
    ) {
        let now = OffsetDateTime::now_utc();
        let host_software_item_id = host_software_item::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(None),
            plugin_config_id: Set(None),
            package_identifier: Set(None),
            installed_version: Set(None),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap()
        .id;
        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(None),
            plugin_type: Set("package_manager_apt".to_string()),
            role: Set("detect_version".to_string()),
            ordinal: Set(0),
            package_identifier: Set("actual".to_string()),
            config: Set(None),
            execution_site: Set("agent".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn query_agent_assignment_rows_excludes_deactivated_hosts() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let service_id = Uuid::now_v7();
        let active_host_id = Uuid::now_v7();
        let deactivated_host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();

        insert_service(&db, tenant_id, service_id).await;
        insert_host(&db, tenant_id, active_host_id, None).await;
        insert_host(
            &db,
            tenant_id,
            deactivated_host_id,
            Some(OffsetDateTime::now_utc()),
        )
        .await;
        insert_software_item(&db, tenant_id, software_item_id).await;
        insert_plugin_assignment(&db, active_host_id, software_item_id).await;
        insert_plugin_assignment(&db, deactivated_host_id, software_item_id).await;

        let now = OffsetDateTime::now_utc();
        for host_id in [active_host_id, deactivated_host_id] {
            service_host::ActiveModel {
                service_id: Set(service_id),
                host_id: Set(host_id),
                linked_at: Set(now),
            }
            .insert(&db)
            .await
            .unwrap();
        }

        let rows = query_agent_assignment_rows(&db, tenant_id, &["detect_version"])
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].host_machine_id, format!("machine-{active_host_id}"));
    }
}
