//! Shared update-trigger logic used by the REST handler and service WS handlers.
//!
//! The trigger pipeline is split into three composable layers defined in
//! [`super::update_dispatch`]:
//!
//! 1. [`validate_update_preconditions`] — verifies all preconditions and loads
//!    the data needed for record creation and dispatch.
//! 2. [`create_update_history_record`] — inserts a Pending `update_history` row.
//! 3. [`dispatch_update_to_agent`] — builds the `ExecuteUpdate` payload and
//!    sends it to the agent via `NotificationService`.
//!
//! [`trigger_update_for_host`] is a convenience wrapper that calls all three
//! sequentially. The batch update code path calls them independently (bulk
//! validation, bulk insert, selective dispatch).

use uptrakit_internal_wire::ReleaseInfo;
use uptrakit_shared_db::entity::update_history;
use uuid::Uuid;

use super::update_dispatch::{
    CreateUpdateRecordParams, DispatchContext, DispatchUpdateParams, PreUpdateProtectionOutcome,
    TriggerUpdateError, build_plugin_assignment, config_prefers_interactive,
    create_update_history_record, dispatch_update_to_agent, has_active_update_for_host,
    prepare_pre_update_protection, validate_update_preconditions,
};

// Re-export for tests that exercise the enrichment logic.
#[cfg(test)]
use super::update_dispatch::enrich_release_info_with_attestation;

// ---------------------------------------------------------------------------
// Public structs
// ---------------------------------------------------------------------------

/// Result returned by a successful [`trigger_update_for_host`] call.
pub struct TriggerUpdateResult {
    /// The newly-created `update_history` record ID.
    pub update_history_id: Uuid,
    /// The initial status of the record. `Queued` when the host already had
    /// an active update at dispatch time; `Pending` otherwise.
    pub initial_status: update_history::UpdateStatus,
}

/// Parameters for [`trigger_update_for_host`].
pub struct TriggerUpdateParams<'a> {
    pub tenant_id: Uuid,
    pub item_id: Uuid,
    pub host_id: Uuid,
    pub to_version: String,
    /// Who initiated the update.
    pub actor_type: &'a str,
    /// User UUID string, API token UUID string, service UUID string, or empty string.
    pub actor_id: &'a str,
    /// Optional release metadata supplied by the REST caller.
    /// `None` when triggered from a service or a scheduler.
    pub release_info: Option<ReleaseInfo>,
    /// When true, the agent allocates a PTY and keeps stdin open for forwarding.
    pub interactive: bool,
}

// ---------------------------------------------------------------------------
// Convenience wrapper
// ---------------------------------------------------------------------------

/// Returns `true` when the database error looks like a unique-constraint
/// violation on `uix_update_history_host_active`.
fn is_unique_constraint_violation(e: &rootcause::Report<TriggerUpdateError>) -> bool {
    if let TriggerUpdateError::Database(db_err) = e.current_context() {
        let msg = db_err.to_string();
        // SQLite: "UNIQUE constraint failed: ..."
        // PostgreSQL: "duplicate key value violates unique constraint ..."
        return msg.contains("UNIQUE constraint failed")
            || msg.contains("duplicate key value violates unique constraint");
    }
    false
}

/// Core update-trigger logic shared by the REST handler and service WS handlers.
///
/// Validates preconditions, then either inserts a `Pending` record and
/// dispatches immediately (when the host is free), or inserts a `Queued`
/// record (when the host already has an active update). The caller can inspect
/// `TriggerUpdateResult::initial_status` to distinguish the two cases.
///
/// For batch operations, call the three layers independently instead.
///
/// # Errors
///
/// Returns a [`TriggerUpdateError`] describing the first validation failure or
/// database error encountered.
#[tracing::instrument(skip_all)]
pub async fn trigger_update_for_host(
    db: &sea_orm::DatabaseConnection,
    dispatch: DispatchContext<'_>,
    params: TriggerUpdateParams<'_>,
) -> super::update_dispatch::Result<TriggerUpdateResult> {
    let target =
        validate_update_preconditions(db, params.tenant_id, params.host_id, params.item_id).await?;

    // Resolve the interactive flag before creating the history record so the
    // persisted column accurately reflects whether the agent will open a PTY,
    // including when the plugin config opts in via `prefer_interactive: true`.
    let execute_update_plugin = build_plugin_assignment(
        &target.execute_update_data.0,
        target.execute_update_data.1.as_ref(),
    )?;
    let resolved_interactive = params.interactive
        || config_prefers_interactive(
            &execute_update_plugin.plugin_type,
            &execute_update_plugin.config,
        );

    // Build a reusable record params template — only `initial_status` differs
    // between the Queued, Pending, and race-condition-Queued insert sites.
    let build_record = |initial_status| CreateUpdateRecordParams {
        tenant_id: params.tenant_id,
        host_id: params.host_id,
        item_id: params.item_id,
        host_software_item_id: Some(target.hsi_link.id),
        to_version: &params.to_version,
        from_version: target.hsi_link.installed_version.clone(),
        actor_type: params.actor_type,
        actor_id: params.actor_id,
        update_category: &target.hsi_link.update_category,
        batch_id: None,
        initial_status,
        interactive: resolved_interactive,
    };

    // Check if the host already has an active (Pending/InProgress) update.
    let host_busy = has_active_update_for_host(db, params.host_id).await?;

    if host_busy {
        // Insert as Queued — do not dispatch until the active update completes.
        let update_history_id =
            create_update_history_record(db, &build_record(update_history::UpdateStatus::Queued))
                .await?;
        tracing::info!(
            update_id = %update_history_id,
            host_id = %params.host_id,
            "host has an active update — new update queued"
        );
        return Ok(TriggerUpdateResult {
            update_history_id,
            initial_status: update_history::UpdateStatus::Queued,
        });
    }

    // Attempt to insert as Pending.
    let pending_insert =
        create_update_history_record(db, &build_record(update_history::UpdateStatus::Pending))
            .await;

    let (update_history_id, initial_status) = match pending_insert {
        Ok(id) => (id, update_history::UpdateStatus::Pending),
        Err(e) if is_unique_constraint_violation(&e) => {
            // Concurrent Pending INSERT from another controller won the race.
            // Re-insert as Queued so this update is not lost.
            tracing::debug!(
                host_id = %params.host_id,
                "concurrent Pending INSERT detected (unique constraint); re-inserting as Queued"
            );
            let id = create_update_history_record(
                db,
                &build_record(update_history::UpdateStatus::Queued),
            )
            .await?;
            (id, update_history::UpdateStatus::Queued)
        }
        Err(e) => return Err(e),
    };

    if matches!(initial_status, update_history::UpdateStatus::Pending) {
        let pre_update_outcome = prepare_pre_update_protection(
            db,
            dispatch.protection.clone(),
            &target,
            update_history_id,
            None,
        )
        .await?;

        if matches!(pre_update_outcome, PreUpdateProtectionOutcome::Failed) {
            return Err(rootcause::report!(TriggerUpdateError::PreUpdateProtection(
                "controller-side pre-update protection failed".to_string()
            )));
        }

        dispatch_update_to_agent(
            dispatch.notifier,
            &target,
            DispatchUpdateParams {
                update_history_id,
                to_version: params.to_version,
                release_info: params.release_info,
                interactive: params.interactive,
            },
        )
        .await?;
    }

    Ok(TriggerUpdateResult {
        update_history_id,
        initial_status,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::queries::update_types::ActorType;
    use async_trait::async_trait;
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, EntityTrait, ModelTrait,
        QueryFilter, QueryOrder, Set,
    };
    use time::OffsetDateTime;
    use uptrakit_internal_wire::ControllerMessage;
    use uptrakit_plugin_infrastructure_registry::{
        ControllerPostUpdateContext, ControllerProtectionContext, ControllerProtectionDecision,
        ControllerUpdateProtection, PluginError, PluginResult, PostUpdateOutcome,
    };
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, plugin_config, prelude::*, service,
        service_host, software_item, tenant, update_history,
    };
    use uptrakit_shared_types::PluginTypeId;
    use uptrakit_shared_types::ServiceStatus;
    use uuid::Uuid;

    /// A no-op notifier for tests — always returns `true` (agent locally connected).
    struct NoopNotifier;

    #[async_trait::async_trait]
    impl crate::notifier::ServiceNotifier for NoopNotifier {
        async fn send_to_service(&self, _service_id: &Uuid, _msg: ControllerMessage) -> bool {
            true
        }
    }

    struct AlwaysFailProtection;

    impl uptrakit_plugin_infrastructure_registry::PluginMeta for AlwaysFailProtection {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::new("infra_test_controller_update_protection")
        }
    }

    #[async_trait]
    impl ControllerUpdateProtection for AlwaysFailProtection {
        async fn prepare_pre_update_protection(
            &self,
            _ctx: &ControllerProtectionContext<'_>,
        ) -> PluginResult<ControllerProtectionDecision> {
            Err(rootcause::report!(PluginError::PluginInternal(
                "controller protection failed".to_string()
            )))
        }

        async fn finalize_post_update(
            &self,
            _ctx: &ControllerPostUpdateContext<'_>,
        ) -> PluginResult<PostUpdateOutcome> {
            Ok(PostUpdateOutcome::default())
        }
    }

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    struct Fixture {
        tenant_id: Uuid,
        item_id: Uuid,
        host_id: Uuid,
        service_id: Uuid,
        plugin_config_id: Uuid,
    }

    async fn insert_base_fixture(db: &DatabaseConnection) -> Fixture {
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let service_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();

        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set(format!("test-{tenant_id}")),
            is_default: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("test-app".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set("machine-001".to_string()),
            hostname: Set("host-001".to_string()),
            friendly_name: Set("Host 001".to_string()),
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

        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("agent-host".to_string()),
            friendly_name: Set("Agent 001".to_string()),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("hash-{service_id}")),
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

        service_host::ActiveModel {
            service_id: Set(service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        plugin_config::ActiveModel {
            id: Set(plugin_config_id),
            tenant_id: Set(tenant_id),
            name: Set("test-plugin".to_string()),
            plugin_type: Set("releases_github".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        let hsi_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(hsi_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(plugin_config_id)),
            package_identifier: Set(Some("test-pkg".to_string())),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(Some("1.1.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("feature".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            host_software_item_id: Set(hsi_id),
            plugin_config_id: Set(Some(plugin_config_id)),
            plugin_type: Set("releases_github".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("org/repo".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        Fixture {
            tenant_id,
            item_id,
            host_id,
            service_id,
            plugin_config_id,
        }
    }

    // ── validate_update_preconditions ───────────────────────────────────

    #[tokio::test]
    async fn validate_preconditions_success() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        let target = result.unwrap();
        assert_eq!(target.item.id, f.item_id);
        assert_eq!(target.host.id, f.host_id);
        assert_eq!(target.agent.id, f.service_id);
    }

    #[tokio::test]
    async fn validate_preconditions_item_not_found() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let missing_id = Uuid::now_v7();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, missing_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::SoftwareItemNotFound
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_item_deactivated() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let item = SoftwareItem::find_by_id(f.item_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: software_item::ActiveModel = item.into();
        active.deactivated_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::SoftwareItemNotFound
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_host_wrong_tenant() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();
        // Create a second tenant and a host belonging to it.
        let other_tenant_id = Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(other_tenant_id),
            name: Set("other-tenant".to_string()),
            slug: Set(format!("other-{other_tenant_id}")),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        let other_host_id = Uuid::now_v7();
        host::ActiveModel {
            id: Set(other_host_id),
            tenant_id: Set(other_tenant_id),
            machine_id: Set("machine-other".to_string()),
            hostname: Set("host-other".to_string()),
            friendly_name: Set("Other Host".to_string()),
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
        .insert(&db)
        .await
        .unwrap();
        // Query with f.tenant_id — the other host belongs to other_tenant_id, so it
        // passes the software item check but fails the host tenant check.
        let result =
            validate_update_preconditions(&db, f.tenant_id, other_host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::HostNotFound
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_host_not_assigned() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(f.host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(f.item_id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        hsi.delete(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::HostNotAssigned
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_no_service_host() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let sh = ServiceHost::find()
            .filter(service_host::Column::HostId.eq(f.host_id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        sh.delete(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::NoAgent
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_prefers_active_service_when_stale_link_exists() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();
        let current_service_id = Uuid::now_v7();

        let stale_service = Service::find_by_id(f.service_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut stale_active: service::ActiveModel = stale_service.into();
        stale_active.deactivated_at = Set(Some(now));
        stale_active.updated_at = Set(now);
        stale_active.service_app_name = Set(Some("uptrakit-agent-ssh".to_string()));
        stale_active.update(&db).await.unwrap();

        service::ActiveModel {
            id: Set(current_service_id),
            tenant_id: Set(f.tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("current-agent-host".to_string()),
            friendly_name: Set("Current Agent".to_string()),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("hash-{current_service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(Some("uptrakit-agent-ssh".to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        service_host::ActiveModel {
            service_id: Set(current_service_id),
            host_id: Set(f.host_id),
            linked_at: Set(now),
        }
        .insert(&db)
        .await
        .unwrap();

        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        let validated = result.expect("active linked service should still be selected");
        assert_eq!(validated.agent.id, current_service_id);
    }

    #[tokio::test]
    async fn validate_preconditions_agent_not_approved() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let svc = Service::find_by_id(f.service_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: service::ActiveModel = svc.into();
        active.status = Set(ServiceStatus::Pending);
        active.update(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::AgentNotApproved
        ));
    }

    #[tokio::test]
    async fn trigger_update_queued_when_host_busy() {
        // When a Pending update already exists for the host, trigger_update_for_host
        // must insert a Queued record and return initial_status: Queued.
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();

        // Insert a Pending update for the host (simulates an active update).
        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(f.item_id),
            host_software_item_id: Set(None),
            from_version: Set(None),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(update_history::UpdateStatus::Pending),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            created_at: Set(now),
            update_category: Set("feature".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        let result = trigger_update_for_host(
            &db,
            DispatchContext {
                notifier: &NoopNotifier,
                protection: None,
            },
            TriggerUpdateParams {
                tenant_id: f.tenant_id,
                item_id: f.item_id,
                host_id: f.host_id,
                to_version: "1.2.0".to_string(),
                actor_type: ActorType::User.as_str(),
                actor_id: "user-1",
                release_info: None,
                interactive: false,
            },
        )
        .await
        .unwrap();

        assert!(
            matches!(result.initial_status, update_history::UpdateStatus::Queued),
            "expected Queued, got {:?}",
            result.initial_status
        );

        // Verify the record is in the DB with status Queued.
        let record = UpdateHistory::find_by_id(result.update_history_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.status, update_history::UpdateStatus::Queued);
        assert!(record.batch_id.is_none());
    }

    #[tokio::test]
    async fn trigger_update_pending_when_host_free() {
        // When no active update exists, trigger_update_for_host inserts Pending.
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;

        let result = trigger_update_for_host(
            &db,
            DispatchContext {
                notifier: &NoopNotifier,
                protection: None,
            },
            TriggerUpdateParams {
                tenant_id: f.tenant_id,
                item_id: f.item_id,
                host_id: f.host_id,
                to_version: "1.1.0".to_string(),
                actor_type: ActorType::User.as_str(),
                actor_id: "user-1",
                release_info: None,
                interactive: false,
            },
        )
        .await
        .unwrap();

        assert!(
            matches!(result.initial_status, update_history::UpdateStatus::Pending),
            "expected Pending, got {:?}",
            result.initial_status
        );
    }

    #[tokio::test]
    async fn trigger_update_protection_failure_marks_failed_and_returns_err() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let protection = Arc::new(AlwaysFailProtection);

        let result = trigger_update_for_host(
            &db,
            DispatchContext {
                notifier: &NoopNotifier,
                protection: Some(protection),
            },
            TriggerUpdateParams {
                tenant_id: f.tenant_id,
                item_id: f.item_id,
                host_id: f.host_id,
                to_version: "1.1.0".to_string(),
                actor_type: ActorType::User.as_str(),
                actor_id: "user-1",
                release_info: None,
                interactive: false,
            },
        )
        .await;

        assert!(
            result.is_err(),
            "controller-side protection failure must bubble up"
        );

        let row = UpdateHistory::find()
            .filter(update_history::Column::HostId.eq(f.host_id))
            .filter(update_history::Column::SoftwareItemId.eq(f.item_id))
            .order_by_desc(update_history::Column::CreatedAt)
            .one(&db)
            .await
            .unwrap()
            .expect("update_history row should exist");

        assert_eq!(row.status, update_history::UpdateStatus::Failed);
        assert!(row.completed_at.is_some(), "failed row must be completed");
        assert_eq!(row.pre_update_protection_status.as_deref(), Some("failed"));
        assert!(row.pre_update_protection_summary.is_some());
    }

    #[tokio::test]
    async fn has_active_update_returns_true_for_pending() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();

        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(f.item_id),
            host_software_item_id: Set(None),
            from_version: Set(None),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(update_history::UpdateStatus::Pending),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            created_at: Set(now),
            update_category: Set("feature".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        assert!(has_active_update_for_host(&db, f.host_id).await.unwrap());
    }

    #[tokio::test]
    async fn has_active_update_returns_false_when_only_queued() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();

        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(f.item_id),
            host_software_item_id: Set(None),
            from_version: Set(None),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(update_history::UpdateStatus::Queued),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            created_at: Set(now),
            update_category: Set("feature".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        assert!(!has_active_update_for_host(&db, f.host_id).await.unwrap());
    }

    #[tokio::test]
    async fn validate_preconditions_host_update_in_progress_different_item_placeholder() {
        // Regression: validate_update_preconditions no longer rejects when
        // a Pending update exists for a different item on the same host.
        // That case is now handled by trigger_update_for_host (queued instead of 409).
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();

        // Insert a second software item (not assigned to the host).
        let other_item_id = Uuid::now_v7();
        uptrakit_shared_db::entity::software_item::ActiveModel {
            id: Set(other_item_id),
            tenant_id: Set(f.tenant_id),
            name: Set("other-app".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        // Insert a Pending update_history row for the OTHER item on the same host.
        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(other_item_id),
            host_software_item_id: Set(None),
            from_version: Set(None),
            to_version: Set(Some("2.0.0".to_string())),
            status: Set(update_history::UpdateStatus::Pending),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            created_at: Set(now),
            update_category: Set("feature".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        // validate_update_preconditions no longer checks the host lock —
        // it should now succeed (the lock check was removed).
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(
            result.is_ok(),
            "expected Ok after removing host lock check; got: {result:?}"
        );
    }

    #[tokio::test]
    async fn validate_preconditions_no_execute_plugin() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        HostSoftwareItemPlugin::delete_many()
            .filter(host_software_item_plugin::Column::HostId.eq(f.host_id))
            .filter(host_software_item_plugin::Column::Role.eq("execute_update"))
            .exec(&db)
            .await
            .unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::NoExecuteUpdatePlugin
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_plugin_config_deactivated() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let pc = PluginConfig::find_by_id(f.plugin_config_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: plugin_config::ActiveModel = pc.into();
        active.deactivated_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::PluginConfigNotFound
        ));
    }

    // ── enrich_release_info_with_attestation ────────────────────────────

    fn make_release_info() -> ReleaseInfo {
        ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://github.com/owner/repo/releases/tag/v1.0.0".to_string(),
            assets: vec![uptrakit_internal_wire::ReleaseAsset {
                name: "app-amd64.tar.gz".to_string(),
                download_url:
                    "https://github.com/owner/repo/releases/download/v1.0.0/app-amd64.tar.gz"
                        .to_string(),
                size: Some(1024),
                content_type: None,
                sha256_digest: None,
            }],
            attestation_status: None,
            require_attestation: false,
        }
    }

    #[test]
    fn enrich_release_info_none_no_metadata_returns_none() {
        let result = enrich_release_info_with_attestation(None, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn enrich_release_info_none_no_tag_in_metadata_returns_none() {
        // Metadata without a tag cannot produce a valid ReleaseInfo.
        let meta = serde_json::json!({ "release_url": "https://example.com/release" });
        let result = enrich_release_info_with_attestation(None, Some(&meta), None);
        assert!(result.is_none());
    }

    #[test]
    fn enrich_release_info_reconstructed_from_metadata_when_none() {
        // Primary bug-fix path: frontend sends release_info: None; controller
        // reconstructs from latest_release_metadata stored after fetch_releases.
        let meta = serde_json::json!({
            "tag": "v2.4.0",
            "release_url": "https://github.com/owner/repo/releases/tag/v2.4.0",
            "assets": [
                {
                    "name": "app-amd64.tar.gz",
                    "download_url": "https://github.com/owner/repo/releases/download/v2.4.0/app-amd64.tar.gz",
                    "size": 2048,
                    "sha256_digest": "c".repeat(64)
                }
            ],
            "attestation_status": "Verified"
        });
        let result = enrich_release_info_with_attestation(None, Some(&meta), None).unwrap();
        assert_eq!(result.tag, "v2.4.0");
        assert_eq!(
            result.release_url,
            "https://github.com/owner/repo/releases/tag/v2.4.0"
        );
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].name, "app-amd64.tar.gz");
        assert_eq!(
            result.assets[0].download_url,
            "https://github.com/owner/repo/releases/download/v2.4.0/app-amd64.tar.gz"
        );
        assert_eq!(result.assets[0].size, Some(2048));
        assert_eq!(result.assets[0].sha256_digest, Some("c".repeat(64)));
        assert_eq!(
            result.attestation_status,
            Some(uptrakit_internal_wire::AttestationStatus::Verified)
        );
        assert!(!result.require_attestation);
    }

    #[test]
    fn enrich_release_info_reconstructed_assets_without_download_url_skipped() {
        // Assets missing download_url (empty string from default) are skipped.
        let meta = serde_json::json!({
            "tag": "v1.0.0",
            "release_url": "https://example.com",
            "assets": [
                { "name": "no-url.tar.gz" }
            ]
        });
        let result = enrich_release_info_with_attestation(None, Some(&meta), None).unwrap();
        assert!(result.assets.is_empty());
    }

    #[test]
    fn enrich_release_info_reconstructed_respects_require_attestation_config() {
        let meta = serde_json::json!({
            "tag": "v1.0.0",
            "release_url": "https://example.com",
            "assets": []
        });
        let config = serde_json::json!({ "require_attestation": true });
        let result =
            enrich_release_info_with_attestation(None, Some(&meta), Some(&config)).unwrap();
        assert!(result.require_attestation);
    }

    #[test]
    fn enrich_release_info_no_metadata_leaves_unchanged() {
        let ri = make_release_info();
        let result = enrich_release_info_with_attestation(Some(ri), None, None).unwrap();
        assert!(result.attestation_status.is_none());
        assert!(!result.require_attestation);
        assert!(result.assets[0].sha256_digest.is_none());
    }

    #[test]
    fn enrich_release_info_sets_attestation_status_and_digest() {
        let ri = make_release_info();
        let meta = serde_json::json!({
            "attestation_status": "Verified",
            "assets": [
                { "name": "app-amd64.tar.gz", "sha256_digest": "a".repeat(64) }
            ]
        });
        let result = enrich_release_info_with_attestation(Some(ri), Some(&meta), None).unwrap();
        assert_eq!(
            result.attestation_status,
            Some(uptrakit_internal_wire::AttestationStatus::Verified)
        );
        assert_eq!(result.assets[0].sha256_digest, Some("a".repeat(64)));
    }

    #[test]
    fn enrich_release_info_sets_require_attestation_from_config() {
        let ri = make_release_info();
        let config = serde_json::json!({ "require_attestation": true });
        let result = enrich_release_info_with_attestation(Some(ri), None, Some(&config)).unwrap();
        assert!(result.require_attestation);
    }

    #[test]
    fn enrich_release_info_asset_name_mismatch_leaves_digest_none() {
        let ri = make_release_info();
        let meta = serde_json::json!({
            "attestation_status": "NotFound",
            "assets": [
                { "name": "other-asset.tar.gz", "sha256_digest": "b".repeat(64) }
            ]
        });
        let result = enrich_release_info_with_attestation(Some(ri), Some(&meta), None).unwrap();
        assert_eq!(
            result.attestation_status,
            Some(uptrakit_internal_wire::AttestationStatus::NotFound)
        );
        assert!(result.assets[0].sha256_digest.is_none());
    }
}
