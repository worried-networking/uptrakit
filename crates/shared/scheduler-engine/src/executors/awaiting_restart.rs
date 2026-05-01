use std::sync::Arc;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uptrakit_shared_db::entity::{software_item, update_history};
use uptrakit_shared_types::UpdateStatus;
use uptrakit_wire::{
    CheckVersionsPayload, ControllerMessage, PluginAssignment, VersionCheckAssignment,
};
use uuid::Uuid;

use crate::error::SchedulerError;
use crate::notifier::SchedulerNotifier;
use crate::tick_executor::TickExecutor;

/// Tick executor for `AwaitingRestart` update records.
///
/// Two responsibilities per poll cycle:
///
/// 1. **Timeout enforcement** — scan all `AwaitingRestart` records; those whose
///    `awaiting_restart_since` is older than the `software_item.awaiting_restart_timeout`
///    (default: 600 s) are transitioned to `Failed` and
///    [`SchedulerNotifier::signal_host_progression`] is called so the queue can
///    advance.
///
/// 2. **Verification dispatch** — for each `AwaitingRestart` record that has a
///    non-NULL `execution_owner_service_id`, look up the `detect_version` plugin
///    assignment for the host/software-item pair and send a [`CheckVersions`]
///    message to the owning service. The reply is processed by
///    `apply_awaiting_restart_version_check` in the version-check handler, which
///    terminates the `AwaitingRestart` state when the new version is confirmed.
pub struct AwaitingRestartExecutor {
    notifier: Arc<dyn SchedulerNotifier>,
}

impl AwaitingRestartExecutor {
    pub fn new(notifier: Arc<dyn SchedulerNotifier>) -> Self {
        Self { notifier }
    }
}

#[async_trait::async_trait]
impl TickExecutor for AwaitingRestartExecutor {
    async fn execute_tick(&self, db: &DatabaseConnection) -> crate::error::Result<()> {
        self.enforce_timeouts(db).await?;
        self.dispatch_verification(db).await?;
        Ok(())
    }
}

impl AwaitingRestartExecutor {
    /// Transition any `AwaitingRestart` records past their timeout to `Failed`.
    async fn enforce_timeouts(&self, db: &DatabaseConnection) -> crate::error::Result<()> {
        use sea_orm::prelude::*;

        let now = time::OffsetDateTime::now_utc();

        let records: Vec<(update_history::Model, Option<software_item::Model>)> =
            update_history::Entity::find()
                .filter(update_history::Column::Status.eq(UpdateStatus::AwaitingRestart))
                .filter(update_history::Column::AwaitingRestartSince.is_not_null())
                .find_also_related(software_item::Entity)
                .all(db)
                .await
                .context_to::<SchedulerError>()?;

        for (record, software) in records {
            let Some(since) = record.awaiting_restart_since else {
                continue;
            };
            let timeout_secs = software
                .and_then(|s| s.awaiting_restart_timeout)
                .unwrap_or(600) as i64;
            let deadline = since + time::Duration::seconds(timeout_secs);

            if now <= deadline {
                continue;
            }

            // CAS: only update if still AwaitingRestart (race-safe).
            let result = update_history::Entity::update_many()
                .filter(update_history::Column::Id.eq(record.id))
                .filter(update_history::Column::Status.eq(UpdateStatus::AwaitingRestart))
                .col_expr(
                    update_history::Column::Status,
                    Expr::value(UpdateStatus::Failed),
                )
                .col_expr(update_history::Column::CompletedAt, Expr::value(Some(now)))
                .exec(db)
                .await
                .context_to::<SchedulerError>()?;

            if result.rows_affected > 0 {
                tracing::info!(
                    update_history_id = %record.id,
                    timeout_secs,
                    host_id = %record.host_id,
                    tenant_id = %record.tenant_id,
                    "AwaitingRestart timed out — transitioned to Failed"
                );
                self.notifier
                    .signal_host_progression(record.host_id, record.tenant_id)
                    .await;
            }
        }
        Ok(())
    }

    /// Send `CheckVersions` messages for each `AwaitingRestart` record that
    /// has a `detect_version` plugin assignment.
    ///
    /// Uses the exact same query helper and payload construction as
    /// [`DetectVersionExecutor`](super::detect_version::DetectVersionExecutor):
    /// [`query_agent_assignment_rows`](super::queries::query_agent_assignment_rows)
    /// filtered to the specific `host_software_item_id` values from
    /// `AwaitingRestart` records.
    async fn dispatch_verification(&self, db: &DatabaseConnection) -> crate::error::Result<()> {
        use std::collections::HashMap;

        use sea_orm::{Condition, JoinType, QuerySelect, RelationTrait};
        use uptrakit_shared_db::entity::{
            host, host_software_item_plugin, plugin_config, service, service_host,
        };
        use uptrakit_shared_types::PluginTypeId;

        // 1. Load all AwaitingRestart records with execution_owner_service_id IS NOT NULL.
        let awaiting: Vec<update_history::Model> = update_history::Entity::find()
            .filter(update_history::Column::Status.eq(UpdateStatus::AwaitingRestart))
            .filter(update_history::Column::ExecutionOwnerServiceId.is_not_null())
            .all(db)
            .await
            .context_to::<SchedulerError>()?;

        if awaiting.is_empty() {
            tracing::debug!("no AwaitingRestart records with execution owner; skipping dispatch");
            return Ok(());
        }

        // Collect (host_software_item_id, execution_owner_service_id) pairs.
        // Records without host_software_item_id cannot be dispatched.
        let dispatch_targets: Vec<(Uuid, Uuid)> = awaiting
            .into_iter()
            .filter_map(|r| {
                let hsi_id = r.host_software_item_id?;
                let svc_id = r.execution_owner_service_id?;
                Some((hsi_id, svc_id))
            })
            .collect();

        if dispatch_targets.is_empty() {
            tracing::debug!(
                "no AwaitingRestart records with host_software_item_id; skipping dispatch"
            );
            return Ok(());
        }

        // Build owner_map with duplicate detection — multiple AwaitingRestart records
        // for the same host_software_item_id are collapsed; conflicts are warned.
        let mut owner_map: HashMap<Uuid, Uuid> = HashMap::new();
        for (hsi_id, svc_id) in dispatch_targets {
            if let Some(prev) = owner_map.insert(hsi_id, svc_id) {
                if prev != svc_id {
                    tracing::warn!(
                        host_software_item_id = %hsi_id,
                        prev_owner = %prev,
                        new_owner = %svc_id,
                        "multiple AwaitingRestart records share same host_software_item_id — using last owner"
                    );
                }
            }
        }
        let target_hsi_ids: Vec<Uuid> = owner_map.keys().copied().collect();

        // 2. Query detect_version plugin assignments for the targeted host_software_item_ids.
        //    Uses the same join pattern as query_agent_assignment_rows but filtered to
        //    specific host_software_item_ids instead of all items for a tenant.
        #[derive(Debug, sea_orm::FromQueryResult)]
        struct DispatchRow {
            service_id: Uuid,
            host_machine_id: String,
            software_item_id: Uuid,
            software_item_name: String,
            plugin_type: String,
            package_identifier: String,
            host_software_item_id: Uuid,
            profile_config: Option<serde_json::Value>,
            assignment_config: Option<serde_json::Value>,
        }

        let rows: Vec<DispatchRow> = host_software_item_plugin::Entity::find()
            .select_only()
            .column_as(service::Column::Id, "service_id")
            .column_as(host::Column::MachineId, "host_machine_id")
            .column_as(
                host_software_item_plugin::Column::SoftwareItemId,
                "software_item_id",
            )
            .column_as(software_item::Column::Name, "software_item_name")
            .column_as(host_software_item_plugin::Column::PluginType, "plugin_type")
            .column_as(
                host_software_item_plugin::Column::PackageIdentifier,
                "package_identifier",
            )
            .column_as(
                host_software_item_plugin::Column::HostSoftwareItemId,
                "host_software_item_id",
            )
            .column_as(plugin_config::Column::Config, "profile_config")
            .column_as(
                host_software_item_plugin::Column::Config,
                "assignment_config",
            )
            .join(
                JoinType::InnerJoin,
                host_software_item_plugin::Relation::SoftwareItem.def(),
            )
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
            .filter(
                host_software_item_plugin::Column::HostSoftwareItemId.is_in(target_hsi_ids.clone()),
            )
            .filter(host_software_item_plugin::Column::Role.eq("detect_version"))
            .filter(software_item::Column::DeactivatedAt.is_null())
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
            .into_model::<DispatchRow>()
            .all(db)
            .await
            .context_to::<SchedulerError>()?;

        if rows.is_empty() {
            tracing::debug!(
                targets = target_hsi_ids.len(),
                "no detect_version assignments found for AwaitingRestart records"
            );
            return Ok(());
        }

        // 3. Build VersionCheckAssignment per (service_id, host_machine_id),
        //    same grouping strategy as DetectVersionExecutor.
        let mut by_agent_host: HashMap<(Uuid, String), HashMap<Uuid, VersionCheckAssignment>> =
            HashMap::new();

        for row in rows {
            // Use the execution_owner_service_id if present, else fall back to
            // the service derived from the host/service_host join (the agent's
            // service). In practice both should coincide because the update was
            // dispatched to the same agent, but we guard for safety.
            let routing_service_id = owner_map
                .get(&row.host_software_item_id)
                .copied()
                .unwrap_or(row.service_id);

            let plugin_type = PluginTypeId::new(&row.plugin_type);
            let config = uptrakit_config_merge::resolve_effective_config(
                None,
                row.profile_config.as_ref(),
                row.assignment_config.as_ref(),
            );
            let assignment = PluginAssignment {
                plugin_type,
                package_identifier: row.package_identifier,
                config,
            };

            let agent_key = (routing_service_id, row.host_machine_id.clone());
            let items = by_agent_host.entry(agent_key).or_default();
            let item =
                items
                    .entry(row.software_item_id)
                    .or_insert_with(|| VersionCheckAssignment {
                        software_item_id: row.software_item_id,
                        name: row.software_item_name.clone(),
                        detect_version: None,
                        fetch_releases: None,
                        host_software_item_id: Some(row.host_software_item_id),
                    });
            item.detect_version = Some(assignment);
        }

        // 4. Flatten and send CheckVersions messages.
        let mut msg_count = 0usize;
        let mut item_count = 0usize;

        for ((service_id, host_machine_id), items) in by_agent_host {
            let assignments: Vec<VersionCheckAssignment> = items
                .into_values()
                .filter(|a| a.detect_version.is_some())
                .collect();
            if assignments.is_empty() {
                continue;
            }
            item_count += assignments.len();
            msg_count += 1;
            let msg = ControllerMessage::CheckVersions(CheckVersionsPayload {
                host_machine_id,
                assignments,
            });
            self.notifier.send_to_service(&service_id, msg).await;
        }

        tracing::info!(
            messages = msg_count,
            items = item_count,
            "sent CheckVersions requests for AwaitingRestart records"
        );
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, service, service_host, software_item,
        tenant, update_history,
    };
    use uptrakit_shared_db::migration::run_migrations;
    use uptrakit_shared_types::UpdateStatus;
    use uptrakit_wire::ControllerMessage;
    use uuid::Uuid;

    use crate::notifier::SchedulerNotifier;

    // ── helpers ──────────────────────────────────────────────────────────────

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();
        db
    }

    async fn insert_tenant(db: &DatabaseConnection) -> Uuid {
        let tenant_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set(tenant_id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        tenant_id
    }

    async fn insert_host(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let host_id = Uuid::now_v7();
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
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        host_id
    }

    async fn insert_service(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let service_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("agent-{service_id}")),
            friendly_name: Set("Agent".to_string()),
            ip_address: Set(None),
            status: Set(uptrakit_shared_types::ServiceStatus::Approved),
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
        service_id
    }

    async fn insert_software_item(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        awaiting_restart_timeout: Option<i32>,
    ) -> Uuid {
        let sw_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(sw_id),
            tenant_id: Set(tenant_id),
            name: Set("test-software".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            awaiting_restart_timeout: Set(awaiting_restart_timeout),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        sw_id
    }

    /// Returns `host_software_item_id`.
    async fn insert_host_software_item(
        db: &DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        host_software_item::ActiveModel {
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
        .id
    }

    async fn insert_detect_version_plugin(
        db: &DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
        host_software_item_id: Uuid,
    ) {
        let now = OffsetDateTime::now_utc();
        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(None),
            plugin_type: Set("package_manager_apt".to_string()),
            role: Set("detect_version".to_string()),
            ordinal: Set(0),
            package_identifier: Set("test-pkg".to_string()),
            config: Set(None),
            execution_site: Set("agent".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_service_host(db: &DatabaseConnection, service_id: Uuid, host_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        service_host::ActiveModel {
            service_id: Set(service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_update_history(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        host_software_item_id: Option<Uuid>,
        status: UpdateStatus,
        awaiting_restart_since: Option<OffsetDateTime>,
        execution_owner_service_id: Option<Uuid>,
    ) -> Uuid {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        update_history::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            from_version: Set(None),
            to_version: Set(None),
            status: Set(status),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set("test".to_string()),
            execution_owner_service_id: Set(execution_owner_service_id),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            awaiting_restart_since: Set(awaiting_restart_since),
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
        .unwrap();
        id
    }

    // ── spy notifier ─────────────────────────────────────────────────────────

    struct SpyNotifier {
        send_to_service_calls: parking_lot::Mutex<Vec<Uuid>>,
        signal_host_progression_calls: parking_lot::Mutex<Vec<(Uuid, Uuid)>>,
    }

    impl SpyNotifier {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                send_to_service_calls: parking_lot::Mutex::new(Vec::new()),
                signal_host_progression_calls: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn send_count(&self) -> usize {
            self.send_to_service_calls.lock().len()
        }

        fn progression_count(&self) -> usize {
            self.signal_host_progression_calls.lock().len()
        }
    }

    #[async_trait::async_trait]
    impl SchedulerNotifier for SpyNotifier {
        async fn send_to_service(&self, service_id: &Uuid, _msg: ControllerMessage) {
            self.send_to_service_calls.lock().push(*service_id);
        }
        async fn broadcast(&self, _msg: ControllerMessage) {}
        async fn send_by_capability(&self, _cap: &str, _msg: ControllerMessage) {}
        async fn signal_ca_rotation(&self, _reason: &str) {}
        async fn signal_software_states_changed(&self, _tenant_id: Uuid) {}
        async fn signal_crl_renewal(&self) {}
        async fn signal_host_progression(&self, host_id: Uuid, tenant_id: Uuid) {
            self.signal_host_progression_calls
                .lock()
                .push((host_id, tenant_id));
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_awaiting_restart_executor_times_out_record() {
        use super::AwaitingRestartExecutor;
        use crate::TickExecutor;
        use sea_orm::EntityTrait;

        let db = setup_test_db().await;
        let tenant_id = insert_tenant(&db).await;
        let host_id = insert_host(&db, tenant_id).await;
        // software_item has awaiting_restart_timeout = 120 seconds
        let sw_id = insert_software_item(&db, tenant_id, Some(120)).await;
        let _hsi_id = insert_host_software_item(&db, host_id, sw_id).await;

        // awaiting_restart_since = now - 130s  (past the 120s timeout)
        let past = OffsetDateTime::now_utc() - time::Duration::seconds(130);
        let record_id = insert_update_history(
            &db,
            tenant_id,
            host_id,
            sw_id,
            None,
            UpdateStatus::AwaitingRestart,
            Some(past),
            None, // no execution owner
        )
        .await;

        let notifier = SpyNotifier::new();
        let executor = AwaitingRestartExecutor::new(notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor.execute_tick(&db).await.unwrap();

        // Record must now be Failed.
        let record = update_history::Entity::find_by_id(record_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.status,
            UpdateStatus::Failed,
            "record past timeout must be transitioned to Failed"
        );
        assert!(
            record.completed_at.is_some(),
            "completed_at must be set on timeout"
        );

        // signal_host_progression must be called once for the timed-out record.
        assert_eq!(
            notifier.progression_count(),
            1,
            "signal_host_progression must be called once for the timed-out record"
        );
    }

    #[tokio::test]
    async fn test_awaiting_restart_executor_skips_null_execution_owner() {
        use super::AwaitingRestartExecutor;
        use crate::TickExecutor;

        let db = setup_test_db().await;
        let tenant_id = insert_tenant(&db).await;
        let host_id = insert_host(&db, tenant_id).await;
        let sw_id = insert_software_item(&db, tenant_id, Some(600)).await;
        let _hsi_id = insert_host_software_item(&db, host_id, sw_id).await;

        // awaiting_restart_since is recent — NOT past timeout
        let recent = OffsetDateTime::now_utc() - time::Duration::seconds(10);
        // execution_owner_service_id = NULL
        insert_update_history(
            &db,
            tenant_id,
            host_id,
            sw_id,
            None,
            UpdateStatus::AwaitingRestart,
            Some(recent),
            None, // NULL execution owner
        )
        .await;

        let notifier = SpyNotifier::new();
        let executor = AwaitingRestartExecutor::new(notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor.execute_tick(&db).await.unwrap();

        // send_to_service must NOT be called when execution_owner_service_id is NULL
        assert_eq!(
            notifier.send_count(),
            0,
            "send_to_service must not be called when execution_owner_service_id is NULL"
        );
    }

    #[tokio::test]
    async fn test_awaiting_restart_executor_dispatches_to_execution_owner() {
        use super::AwaitingRestartExecutor;
        use crate::TickExecutor;

        let db = setup_test_db().await;
        let tenant_id = insert_tenant(&db).await;
        let host_id = insert_host(&db, tenant_id).await;
        let service_id = insert_service(&db, tenant_id).await;
        insert_service_host(&db, service_id, host_id).await;
        let sw_id = insert_software_item(&db, tenant_id, Some(600)).await;
        let hsi_id = insert_host_software_item(&db, host_id, sw_id).await;
        insert_detect_version_plugin(&db, host_id, sw_id, hsi_id).await;

        // Recent AwaitingRestart record WITH execution_owner_service_id
        let recent = OffsetDateTime::now_utc() - time::Duration::seconds(10);
        insert_update_history(
            &db,
            tenant_id,
            host_id,
            sw_id,
            Some(hsi_id),
            UpdateStatus::AwaitingRestart,
            Some(recent),
            Some(service_id),
        )
        .await;

        let notifier = SpyNotifier::new();
        let executor = AwaitingRestartExecutor::new(notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor.execute_tick(&db).await.unwrap();

        // send_to_service MUST be called exactly once to dispatch verification.
        assert_eq!(
            notifier.send_count(),
            1,
            "send_to_service must be called once for the AwaitingRestart record with execution owner"
        );
        assert_eq!(
            notifier.send_to_service_calls.lock()[0],
            service_id,
            "must send to the execution owner service"
        );
    }

    #[tokio::test]
    async fn test_awaiting_restart_executor_empty_db_returns_ok() {
        use super::AwaitingRestartExecutor;
        use crate::TickExecutor;

        let db = setup_test_db().await;
        let notifier = SpyNotifier::new();
        let executor = AwaitingRestartExecutor::new(notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor.execute_tick(&db).await.unwrap();
        assert_eq!(notifier.send_count(), 0);
        assert_eq!(notifier.progression_count(), 0);
    }

    #[tokio::test]
    async fn test_awaiting_restart_executor_within_timeout_not_failed() {
        use super::AwaitingRestartExecutor;
        use crate::TickExecutor;
        use sea_orm::EntityTrait;

        let db = setup_test_db().await;
        let tenant_id = insert_tenant(&db).await;
        let host_id = insert_host(&db, tenant_id).await;
        // timeout = 120s, but only 60s have elapsed
        let sw_id = insert_software_item(&db, tenant_id, Some(120)).await;
        let _hsi_id = insert_host_software_item(&db, host_id, sw_id).await;

        let recent = OffsetDateTime::now_utc() - time::Duration::seconds(60);
        let record_id = insert_update_history(
            &db,
            tenant_id,
            host_id,
            sw_id,
            None,
            UpdateStatus::AwaitingRestart,
            Some(recent),
            None,
        )
        .await;

        let notifier = SpyNotifier::new();
        let executor = AwaitingRestartExecutor::new(notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor.execute_tick(&db).await.unwrap();

        let record = update_history::Entity::find_by_id(record_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.status,
            UpdateStatus::AwaitingRestart,
            "record within timeout must remain AwaitingRestart"
        );
        assert_eq!(
            notifier.progression_count(),
            0,
            "no progression signal for record within timeout"
        );
    }

    #[tokio::test]
    async fn test_awaiting_restart_executor_times_out_default_600s() {
        use super::AwaitingRestartExecutor;
        use crate::TickExecutor;
        use sea_orm::EntityTrait;

        let db = setup_test_db().await;
        let tenant_id = insert_tenant(&db).await;
        // Two separate hosts — the partial unique index allows only one active
        // AwaitingRestart record per host_id at a time.
        let host_past = insert_host(&db, tenant_id).await;
        let host_within = insert_host(&db, tenant_id).await;
        // software_item with awaiting_restart_timeout = NULL → defaults to 600 s
        let sw_id = insert_software_item(&db, tenant_id, None).await;
        let _hsi_past = insert_host_software_item(&db, host_past, sw_id).await;
        let _hsi_within = insert_host_software_item(&db, host_within, sw_id).await;

        // Record past the 600 s default timeout → must be transitioned to Failed.
        let past_deadline = OffsetDateTime::now_utc() - time::Duration::seconds(601);
        let timed_out_id = insert_update_history(
            &db,
            tenant_id,
            host_past,
            sw_id,
            None,
            UpdateStatus::AwaitingRestart,
            Some(past_deadline),
            None,
        )
        .await;

        // Record still within the 600 s default timeout → must remain AwaitingRestart.
        let within_deadline = OffsetDateTime::now_utc() - time::Duration::seconds(599);
        let still_waiting_id = insert_update_history(
            &db,
            tenant_id,
            host_within,
            sw_id,
            None,
            UpdateStatus::AwaitingRestart,
            Some(within_deadline),
            None,
        )
        .await;

        let notifier = SpyNotifier::new();
        let executor = AwaitingRestartExecutor::new(notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor.execute_tick(&db).await.unwrap();

        // The 601 s record must have timed out.
        let timed_out = update_history::Entity::find_by_id(timed_out_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            timed_out.status,
            UpdateStatus::Failed,
            "record at 601 s must be transitioned to Failed with NULL timeout (default 600 s)"
        );
        assert!(
            timed_out.completed_at.is_some(),
            "completed_at must be set when NULL-timeout record expires"
        );

        // The 599 s record must still be waiting.
        let still_waiting = update_history::Entity::find_by_id(still_waiting_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            still_waiting.status,
            UpdateStatus::AwaitingRestart,
            "record at 599 s must remain AwaitingRestart with NULL timeout (default 600 s)"
        );

        // Only the timed-out record triggers a progression signal.
        assert_eq!(
            notifier.progression_count(),
            1,
            "signal_host_progression must be called exactly once for the timed-out record"
        );
    }
}
