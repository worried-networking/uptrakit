#![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]
#![expect(
    clippy::unwrap_used,
    reason = "test code: panics on failure are acceptable"
)]
#![expect(
    clippy::expect_used,
    reason = "expect used for infallible operations in test code; message documents the invariant"
)]

use super::*;
use crate::AppState;
use std::collections::HashSet;

use super::super::shared_types::MAX_UPDATE_OUTPUT_BYTES;
use super::result::select_best_output;
use super::started::{UpdateStartedInfo, broadcast_update_started_events};
use async_trait::async_trait;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::{
    CatalogConfig, ControllerPostUpdateContext, ControllerProtectionContext,
    ControllerProtectionDecision, ControllerUpdateHookOps, ControllerUpdateProtection,
    ControllerUpdateProtectionOps, NotificationOps, NotificationTransport, PluginConfigOps,
    PluginError, PluginMetadataOps, PluginOps, PluginResult, PluginSurfaceActionOps,
    PluginSurfaceOps, PostUpdateOutcome, SoftwareItemCreatedEvent, SoftwareItemLifecycle,
    SoftwareItemLifecycleContext, SoftwareItemLifecycleOps, SoftwareItemPatch,
    SurfaceActionContext, SurfaceActionError, build_catalog,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, service, service_host, software_item,
    update_batch, update_history,
};
use uptrakit_shared_types::{PluginTypeId, ServiceStatus};
use uptrakit_wire::{BatchUpdateResultPayload, UpdateFinalStatus, UpdateResultPayload};
use uuid::Uuid;

struct ReplayFailProtection;

impl uptrakit_plugin_infrastructure_registry::PluginMeta for ReplayFailProtection {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::new("infra_test_replay_fail_protection")
    }
}

#[async_trait]
impl ControllerUpdateProtection for ReplayFailProtection {
    async fn prepare_pre_update_protection(
        &self,
        _ctx: &ControllerProtectionContext<'_>,
    ) -> PluginResult<ControllerProtectionDecision> {
        Err(rootcause::report!(PluginError::PluginInternal(
            "replay protection failure".to_string()
        )))
    }

    async fn finalize_post_update(
        &self,
        _ctx: &ControllerPostUpdateContext<'_>,
    ) -> PluginResult<PostUpdateOutcome> {
        Ok(PostUpdateOutcome::default())
    }
}

struct FinalizeErrorProtection;

impl uptrakit_plugin_infrastructure_registry::PluginMeta for FinalizeErrorProtection {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::new("infra_test_finalize_error_protection")
    }
}

#[async_trait]
impl ControllerUpdateProtection for FinalizeErrorProtection {
    async fn prepare_pre_update_protection(
        &self,
        _ctx: &ControllerProtectionContext<'_>,
    ) -> PluginResult<ControllerProtectionDecision> {
        Ok(ControllerProtectionDecision::skipped(None))
    }

    async fn finalize_post_update(
        &self,
        _ctx: &ControllerPostUpdateContext<'_>,
    ) -> PluginResult<PostUpdateOutcome> {
        Err(rootcause::report!(PluginError::PluginInternal(
            "finalize failure".to_string()
        )))
    }
}

/// No-op update protection: skips pre-update protection and succeeds on
/// post-update finalization. Used in tests that need to bypass the
/// Proxmox controller update protection registered by the default catalog.
struct NoopUpdateProtection;

impl uptrakit_plugin_infrastructure_registry::PluginMeta for NoopUpdateProtection {
    fn plugin_type_id(&self) -> PluginTypeId {
        PluginTypeId::new("infra_test_noop_protection")
    }
}

#[async_trait]
impl ControllerUpdateProtection for NoopUpdateProtection {
    async fn prepare_pre_update_protection(
        &self,
        _ctx: &ControllerProtectionContext<'_>,
    ) -> PluginResult<ControllerProtectionDecision> {
        Ok(ControllerProtectionDecision::skipped(None))
    }

    async fn finalize_post_update(
        &self,
        _ctx: &ControllerPostUpdateContext<'_>,
    ) -> PluginResult<PostUpdateOutcome> {
        Ok(PostUpdateOutcome::default())
    }
}

struct ProtectionOverridePluginOps {
    inner: Arc<dyn PluginOps>,
    protection: Arc<dyn ControllerUpdateProtection>,
}

impl PluginMetadataOps for ProtectionOverridePluginOps {
    fn get(
        &self,
        id: &uptrakit_shared_types::PluginTypeId,
    ) -> Option<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
        self.inner.get(id)
    }

    fn all(&self) -> Vec<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
        self.inner.all()
    }

    fn instance_enabled(&self, id: &uptrakit_shared_types::PluginTypeId) -> bool {
        self.inner.instance_enabled(id)
    }
}

impl PluginConfigOps for ProtectionOverridePluginOps {}

#[async_trait]
impl PluginSurfaceActionOps for ProtectionOverridePluginOps {
    async fn handle_surface_action(
        &self,
        ctx: &SurfaceActionContext<'_>,
        surface_id: &str,
        action_id: &str,
        method: uptrakit_wire::surfaces::InteractionHttpMethod,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, SurfaceActionError> {
        self.inner
            .handle_surface_action(ctx, surface_id, action_id, method, params)
            .await
    }
}

impl PluginSurfaceOps for ProtectionOverridePluginOps {
    fn surface_registrations(&self) -> Vec<uptrakit_wire::surfaces::SurfaceRegistration> {
        self.inner.surface_registrations()
    }
}

#[async_trait]
impl NotificationOps for ProtectionOverridePluginOps {
    fn transport(
        &self,
        id: &uptrakit_shared_types::PluginTypeId,
    ) -> Option<Arc<dyn NotificationTransport>> {
        self.inner.transport(id)
    }

    fn notification_supported_types(&self) -> Vec<uptrakit_shared_types::PluginTypeId> {
        self.inner.notification_supported_types()
    }
}

#[async_trait]
impl SoftwareItemLifecycleOps for ProtectionOverridePluginOps {
    async fn on_software_item_created(
        &self,
        event: &SoftwareItemCreatedEvent,
        ctx: &SoftwareItemLifecycleContext,
    ) -> Option<SoftwareItemPatch> {
        self.inner.on_software_item_created(event, ctx).await
    }

    fn software_item_lifecycle_plugins(&self) -> &[Arc<dyn SoftwareItemLifecycle>] {
        self.inner.software_item_lifecycle_plugins()
    }
}

impl ControllerUpdateProtectionOps for ProtectionOverridePluginOps {
    fn controller_update_protection(&self) -> Option<Arc<dyn ControllerUpdateProtection>> {
        Some(self.protection.clone())
    }
}

impl ControllerUpdateHookOps for ProtectionOverridePluginOps {}

async fn build_test_state_with_protection(
    db: sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    protection: Arc<dyn ControllerUpdateProtection>,
) -> Arc<AppState> {
    let base_plugin_ops: Arc<dyn PluginOps> = Arc::new(
        build_catalog(
            &CatalogConfig::default(),
            uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
        )
        .expect("catalog should build in tests"),
    );
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(ProtectionOverridePluginOps {
        inner: base_plugin_ops,
        protection,
    });
    let (state, _jwt) =
        crate::test_harness::build_test_state_with_plugin_ops(db, tenant_id, Some(plugin_ops))
            .await;
    state
}

async fn insert_service_row(db: &sea_orm::DatabaseConnection, tenant_id: Uuid, service_id: Uuid) {
    let now = OffsetDateTime::now_utc();
    service::ActiveModel {
        id: Set(service_id),
        tenant_id: Set(tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set(format!("svc-{service_id}")),
        friendly_name: Set(format!("Service {service_id}")),
        ip_address: Set(None),
        status: Set(ServiceStatus::Approved),
        enrollment_secret_hash: Set(format!("secret-{service_id}")),
        client_version: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        enrollment_token_id: Set(None),
        cert_lifetime_hours: Set(None),
        service_app_name: Set(Some("uptrakit-agent".to_string())),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
}

async fn insert_linked_host(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    service_id: Uuid,
) -> Uuid {
    let now = OffsetDateTime::now_utc();
    let host_id = Uuid::now_v7();

    host::ActiveModel {
        id: Set(host_id),
        tenant_id: Set(tenant_id),
        machine_id: Set(format!("machine-{service_id}-{host_id}")),
        hostname: Set(format!("host-{service_id}-{host_id}")),
        friendly_name: Set(format!("Host {service_id} {host_id}")),
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

    service_host::ActiveModel {
        service_id: Set(service_id),
        host_id: Set(host_id),
        linked_at: Set(now),
    }
    .insert(db)
    .await
    .unwrap();

    host_id
}

async fn insert_software_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    name: &str,
) -> Uuid {
    let now = OffsetDateTime::now_utc();
    software_item::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        name: Set(name.to_string()),
        featured: Set(false),
        icon_url: Set(None),
        last_checked_at: Set(None),
        awaiting_restart_timeout: Set(None),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .unwrap()
    .id
}

async fn tenant_audit_row_for_action(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> uptrakit_shared_db::entity::audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = uptrakit_shared_db::entity::audit_log::Entity::find()
            .filter(uptrakit_shared_db::entity::audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(uptrakit_shared_db::entity::audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("expected tenant audit row for action {action_type}");
}

#[tokio::test]
async fn broadcast_update_started_emits_semantic_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_service_row(&db, tenant_id, service_id).await;
    let host_id = insert_linked_host(&db, tenant_id, service_id).await;
    let software_item_id = insert_software_item(&db, tenant_id, "nginx").await;
    let payload = uptrakit_wire::UpdateStartedPayload {
        update_history_id: Uuid::now_v7(),
        from_version: Some("1.0.0".to_string()),
        interactive: false,
    };
    let info = UpdateStartedInfo {
        batch_id: None,
        host_id,
        software_item_id,
        tenant_id,
    };

    broadcast_update_started_events(&state, service_id, &payload, &info).await;

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STARTED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(row.target_type.as_deref(), Some("update_history"));
    assert_eq!(
        row.target_id.as_deref(),
        Some(payload.update_history_id.to_string().as_str())
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["host_id"], serde_json::json!(host_id));
    assert_eq!(
        details["software_item_id"],
        serde_json::json!(software_item_id)
    );
    assert_eq!(details["interactive"], serde_json::json!(false));
}

#[tokio::test]
async fn broadcast_batch_update_started_emits_batch_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    insert_service_row(&db, tenant_id, service_id).await;
    let host_id = insert_linked_host(&db, tenant_id, service_id).await;
    let software_item_id = insert_software_item(&db, tenant_id, "nginx").await;
    let batch_id = Uuid::now_v7();
    let payload = uptrakit_wire::UpdateStartedPayload {
        update_history_id: Uuid::now_v7(),
        from_version: Some("1.0.0".to_string()),
        interactive: true,
    };
    let info = UpdateStartedInfo {
        batch_id: Some(batch_id),
        host_id,
        software_item_id,
        tenant_id,
    };

    broadcast_update_started_events(&state, service_id, &payload, &info).await;

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_BATCH_UPDATE_STARTED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(row.target_type.as_deref(), Some("batch_update"));
    assert_eq!(
        row.target_id.as_deref(),
        Some(batch_id.to_string().as_str())
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["host_id"], serde_json::json!(host_id));
    assert_eq!(
        details["software_item_id"],
        serde_json::json!(software_item_id)
    );
    assert_eq!(details["interactive"], serde_json::json!(true));
    assert_eq!(
        details["update_history_id"],
        serde_json::json!(payload.update_history_id)
    );
}

async fn insert_pending_update_without_assignment(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
) -> Uuid {
    let now = OffsetDateTime::now_utc();
    let update_history_id = Uuid::now_v7();
    update_history::ActiveModel {
        id: Set(update_history_id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(None),
        from_version: Set(Some("1.0.0".to_string())),
        to_version: Set(Some("1.1.0".to_string())),
        status: Set(update_history::UpdateStatus::Pending),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set("user".to_string()),
        actor_id: Set(String::new()),
        execution_owner_service_id: Set(None),
        execution_owner_instance_id: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        awaiting_restart_since: Set(None),
        created_at: Set(now),
        update_category: Set("security".to_string()),
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
    update_history_id
}

async fn insert_replayable_queued_update(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
) -> Uuid {
    let now = OffsetDateTime::now_utc();
    let host_software_item_id = host_software_item::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        qualifier: Set(None),
        plugin_config_id: Set(None),
        package_identifier: Set(Some("demo".to_string())),
        installed_version: Set(Some("1.0.0".to_string())),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("security".to_string()),
        deactivated_at: Set(None),
        last_discovered_at: Set(None),
        discovery_source: Set(None),
        missing_since: Set(None),
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
        plugin_type: Set("generic.shell".to_string()),
        role: Set("execute_update".to_string()),
        ordinal: Set(0),
        package_identifier: Set("demo".to_string()),
        config: Set(None),
        execution_site: Set("agent".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .unwrap();

    let update_history_id = Uuid::now_v7();
    update_history::ActiveModel {
        id: Set(update_history_id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(Some(host_software_item_id)),
        from_version: Set(Some("1.0.0".to_string())),
        to_version: Set(Some("1.1.0".to_string())),
        status: Set(update_history::UpdateStatus::Queued),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set("user".to_string()),
        actor_id: Set(String::new()),
        execution_owner_service_id: Set(None),
        execution_owner_instance_id: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        awaiting_restart_since: Set(None),
        created_at: Set(now),
        update_category: Set("security".to_string()),
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

    update_history_id
}

struct InProgressUpdateFlags {
    host_software_item_id: Option<Uuid>,
    batch_id: Option<Uuid>,
    interactive: bool,
}

async fn insert_owned_in_progress_update(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    service_id: Uuid,
    runtime_instance_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    flags: InProgressUpdateFlags,
) -> Uuid {
    let InProgressUpdateFlags {
        host_software_item_id,
        batch_id,
        interactive,
    } = flags;
    let now = OffsetDateTime::now_utc();
    let update_history_id = Uuid::now_v7();
    update_history::ActiveModel {
        id: Set(update_history_id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(host_software_item_id),
        from_version: Set(Some("1.0.0".to_string())),
        to_version: Set(Some("1.1.0".to_string())),
        status: Set(update_history::UpdateStatus::InProgress),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set("user".to_string()),
        actor_id: Set(String::new()),
        execution_owner_service_id: Set(Some(service_id)),
        execution_owner_instance_id: Set(Some(runtime_instance_id)),
        started_at: Set(Some(now)),
        completed_at: Set(None),
        awaiting_restart_since: Set(None),
        created_at: Set(now),
        update_category: Set("security".to_string()),
        batch_id: Set(batch_id),
        interactive: Set(interactive),
        output_truncated: Set(false),
        pre_update_protection_status: Set(None),
        pre_update_protection_summary: Set(None),
        recovery_hint: Set(None),
    }
    .insert(db)
    .await
    .expect("insert owned in-progress update");
    update_history_id
}

#[tokio::test]
async fn prepare_pending_replay_messages_fails_unreplayable_rows_and_unblocks_successors() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state =
        build_test_state_with_protection(db, tenant_id, Arc::new(NoopUpdateProtection)).await;
    let service_id = Uuid::now_v7();

    insert_service_row(state.db(), tenant_id, service_id).await;
    let host_id = insert_linked_host(state.db(), tenant_id, service_id).await;
    let broken_item_id = insert_software_item(state.db(), tenant_id, "broken").await;
    let queued_item_id = insert_software_item(state.db(), tenant_id, "queued").await;

    let broken_update_id =
        insert_pending_update_without_assignment(state.db(), tenant_id, host_id, broken_item_id)
            .await;
    let queued_update_id =
        insert_replayable_queued_update(state.db(), tenant_id, host_id, queued_item_id).await;

    let messages = prepare_pending_replay_messages(&state, service_id)
        .await
        .unwrap();

    // Unprotected Pending records are handed off to the orchestrator rather
    // than replayed inline, so the returned messages array is empty.
    assert!(
        messages.is_empty(),
        "unprotected Pending records are dispatched via orchestrator, not replay messages"
    );

    let broken_row = update_history::Entity::find_by_id(broken_update_id)
        .one(state.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(broken_row.status, update_history::UpdateStatus::Failed);
    assert!(
        broken_row.output.contains("replay failed"),
        "failed pending row should record why reconnect replay could not continue"
    );

    // The queued successor was promoted to Pending by dispatch_next_queued_update_for_replay
    // so the orchestrator can pick it up on the next cycle.
    let queued_row = update_history::Entity::find_by_id(queued_update_id)
        .one(state.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued_row.status, update_history::UpdateStatus::Pending);
}

#[tokio::test]
async fn prepare_pending_replay_messages_skips_replay_dispatch_when_successor_protection_fails() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state =
        build_test_state_with_protection(db, tenant_id, Arc::new(ReplayFailProtection)).await;
    let service_id = Uuid::now_v7();

    insert_service_row(state.db(), tenant_id, service_id).await;
    let host_id = insert_linked_host(state.db(), tenant_id, service_id).await;
    let broken_item_id = insert_software_item(state.db(), tenant_id, "broken").await;
    let queued_item_id = insert_software_item(state.db(), tenant_id, "queued").await;

    insert_pending_update_without_assignment(state.db(), tenant_id, host_id, broken_item_id).await;
    let queued_update_id =
        insert_replayable_queued_update(state.db(), tenant_id, host_id, queued_item_id).await;

    let messages = prepare_pending_replay_messages(&state, service_id)
        .await
        .unwrap();
    assert!(
        messages.is_empty(),
        "unprotected Pending records are dispatched via orchestrator, not replay messages"
    );

    // broken_item had no plugin assignment so load_target_for_dispatch fails, which calls
    // fail_unreplayable_pending_update synchronously. That promotes the queued successor to
    // Pending before prepare_pending_replay_messages returns — no orchestrator is ever spawned.
    let queued_row = update_history::Entity::find_by_id(queued_update_id)
        .one(state.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        queued_row.status,
        update_history::UpdateStatus::Pending,
        "queued successor must be Pending after synchronous fail_unreplayable_pending_update"
    );
}

#[tokio::test]
async fn handle_update_result_unowned_failure_finalization_error_still_promotes_successor() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let state =
        build_test_state_with_protection(db, tenant_id, Arc::new(FinalizeErrorProtection)).await;
    let service_id = Uuid::now_v7();

    insert_service_row(state.db(), tenant_id, service_id).await;
    let host_id = insert_linked_host(state.db(), tenant_id, service_id).await;
    let failed_item_id = insert_software_item(state.db(), tenant_id, "failed-item").await;
    let queued_item_id = insert_software_item(state.db(), tenant_id, "queued-item").await;

    let pending_unowned_id =
        insert_pending_update_without_assignment(state.db(), tenant_id, host_id, failed_item_id)
            .await;
    let queued_update_id =
        insert_replayable_queued_update(state.db(), tenant_id, host_id, queued_item_id).await;

    let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::from([host_id])));
    let _ = handle_update_result(
        &state,
        service_id,
        UpdateResultPayload {
            update_history_id: pending_unowned_id,
            status: UpdateFinalStatus::Failed,
            error: Some("ssh pre-start failure".to_string()),
            output: String::new(),
            from_version: None,
            to_version: None,
            resumable: None,
        },
        &linked_host_ids,
        Some(Uuid::now_v7()),
    )
    .await;

    let failed_row = update_history::Entity::find_by_id(pending_unowned_id)
        .one(state.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed_row.status, update_history::UpdateStatus::Failed);

    let queued_row = update_history::Entity::find_by_id(queued_update_id)
        .one(state.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        queued_row.status,
        update_history::UpdateStatus::Pending,
        "finalization errors for pre-start failures must not block queue progression"
    );
}

#[tokio::test]
async fn handle_update_result_emits_update_finalized_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    let runtime_instance_id = Uuid::now_v7();

    insert_service_row(&db, tenant_id, service_id).await;
    let host_id = insert_linked_host(&db, tenant_id, service_id).await;
    let software_item_id = insert_software_item(&db, tenant_id, "nginx").await;
    let host_software_item_id = host_software_item::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        qualifier: Set(None),
        plugin_config_id: Set(None),
        package_identifier: Set(Some("nginx".to_string())),
        installed_version: Set(Some("1.0.0".to_string())),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(OffsetDateTime::now_utc()),
        update_category: Set("security".to_string()),
        deactivated_at: Set(None),
        last_discovered_at: Set(None),
        discovery_source: Set(None),
        missing_since: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert host_software_item")
    .id;
    let update_history_id = insert_owned_in_progress_update(
        &db,
        tenant_id,
        service_id,
        runtime_instance_id,
        host_id,
        software_item_id,
        InProgressUpdateFlags {
            host_software_item_id: Some(host_software_item_id),
            batch_id: None,
            interactive: false,
        },
    )
    .await;

    let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::from([host_id])));
    handle_update_result(
        &state,
        service_id,
        UpdateResultPayload {
            update_history_id,
            status: UpdateFinalStatus::Failed,
            error: Some("permission denied".to_string()),
            output: "stderr omitted".to_string(),
            from_version: Some("1.0.0".to_string()),
            to_version: Some("1.1.0".to_string()),
            resumable: None,
        },
        &linked_host_ids,
        Some(runtime_instance_id),
    )
    .await;

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_FINALIZED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Failed.as_str()
    );
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(service_id));
    assert_eq!(row.target_type.as_deref(), Some("update_history"));
    assert_eq!(
        row.target_id.as_deref(),
        Some(update_history_id.to_string().as_str())
    );
    let details = row.details_json.expect("software.update.finalized details");
    assert_eq!(details["status"], serde_json::json!("failed"));
    assert_eq!(details["dispatch_mode"], serde_json::json!("queued"));
    assert_eq!(details["host_id"], serde_json::json!(host_id));
    assert_eq!(
        details["software_item_id"],
        serde_json::json!(software_item_id)
    );
    assert_eq!(details["output_truncated"], serde_json::json!(false));
    assert_eq!(
        details["reason_code"],
        serde_json::json!("agent_reported_failure")
    );
    assert!(
        !details.to_string().contains("stderr omitted"),
        "semantic audit details must not store raw update output"
    );
}

#[tokio::test]
async fn handle_batch_update_result_emits_batch_update_finalized_audit_summary() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    let runtime_instance_id = Uuid::now_v7();

    insert_service_row(&db, tenant_id, service_id).await;
    let host_id = insert_linked_host(&db, tenant_id, service_id).await;
    let host_id_b = insert_linked_host(&db, tenant_id, service_id).await;
    let batch_id = Uuid::now_v7();
    update_batch::ActiveModel {
        id: Set(batch_id),
        tenant_id: Set(tenant_id),
        batch_type: Set("manual".to_string()),
        status: Set(uptrakit_shared_types::BatchStatus::InProgress),
        total_count: Set(2),
        actor_type: Set("user".to_string()),
        actor_id: Set(Uuid::now_v7().to_string()),
        output: Set(String::new()),
        output_bytes: Set(0),
        created_at: Set(OffsetDateTime::now_utc()),
        completed_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert update_batch");

    let software_item_a = insert_software_item(&db, tenant_id, "nginx").await;
    let hsi_a = host_software_item::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(software_item_a),
        qualifier: Set(None),
        plugin_config_id: Set(None),
        package_identifier: Set(Some("nginx".to_string())),
        installed_version: Set(Some("1.0.0".to_string())),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(OffsetDateTime::now_utc()),
        update_category: Set("security".to_string()),
        deactivated_at: Set(None),
        last_discovered_at: Set(None),
        discovery_source: Set(None),
        missing_since: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert first host_software_item")
    .id;
    let update_a = insert_owned_in_progress_update(
        &db,
        tenant_id,
        service_id,
        runtime_instance_id,
        host_id,
        software_item_a,
        InProgressUpdateFlags {
            host_software_item_id: Some(hsi_a),
            batch_id: Some(batch_id),
            interactive: false,
        },
    )
    .await;

    let software_item_b = insert_software_item(&db, tenant_id, "postgres").await;
    let hsi_b = host_software_item::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id_b),
        software_item_id: Set(software_item_b),
        qualifier: Set(None),
        plugin_config_id: Set(None),
        package_identifier: Set(Some("postgres".to_string())),
        installed_version: Set(Some("1.0.0".to_string())),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(OffsetDateTime::now_utc()),
        update_category: Set("security".to_string()),
        deactivated_at: Set(None),
        last_discovered_at: Set(None),
        discovery_source: Set(None),
        missing_since: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert second host_software_item")
    .id;
    let update_b = insert_owned_in_progress_update(
        &db,
        tenant_id,
        service_id,
        runtime_instance_id,
        host_id_b,
        software_item_b,
        InProgressUpdateFlags {
            host_software_item_id: Some(hsi_b),
            batch_id: Some(batch_id),
            interactive: false,
        },
    )
    .await;

    let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::from([host_id, host_id_b])));
    handle_batch_update_result(
        &state,
        service_id,
        BatchUpdateResultPayload {
            batch_id,
            results: vec![
                uptrakit_wire::BatchUpdateItemResult {
                    update_history_id: update_a,
                    host_software_item_id: hsi_a,
                    status: UpdateFinalStatus::Completed,
                    installed_version: Some("1.1.0".to_string()),
                    error: None,
                    output: String::new(),
                },
                uptrakit_wire::BatchUpdateItemResult {
                    update_history_id: update_b,
                    host_software_item_id: hsi_b,
                    status: UpdateFinalStatus::Failed,
                    installed_version: None,
                    error: Some("package lock held".to_string()),
                    output: String::new(),
                },
            ],
        },
        &linked_host_ids,
        Some(runtime_instance_id),
    )
    .await;

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_BATCH_UPDATE_FINALIZED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Partial.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("batch_update"));
    assert_eq!(
        row.target_id.as_deref(),
        Some(batch_id.to_string().as_str())
    );
    let details = row
        .details_json
        .expect("software.batch_update.finalized details");
    assert_eq!(details["result_count"], serde_json::json!(2));
    assert_eq!(details["completed_count"], serde_json::json!(1));
    assert_eq!(details["failed_count"], serde_json::json!(1));
    assert_eq!(details["dispatch_mode"], serde_json::json!("batch"));
}

#[tokio::test]
async fn handle_stdin_attention_emits_update_stdin_attention_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let service_id = Uuid::now_v7();
    let runtime_instance_id = Uuid::now_v7();

    insert_service_row(&db, tenant_id, service_id).await;
    let host_id = insert_linked_host(&db, tenant_id, service_id).await;
    let software_item_id = insert_software_item(&db, tenant_id, "nginx").await;
    let update_history_id = insert_owned_in_progress_update(
        &db,
        tenant_id,
        service_id,
        runtime_instance_id,
        host_id,
        software_item_id,
        InProgressUpdateFlags {
            host_software_item_id: None,
            batch_id: None,
            interactive: true,
        },
    )
    .await;

    let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::from([host_id])));
    handle_stdin_attention(
        &state,
        service_id,
        &serde_json::from_value(serde_json::json!({
            "update_history_id": update_history_id,
            "hint": "Enter password",
        }))
        .expect("stdin attention payload"),
        &linked_host_ids,
        Some(runtime_instance_id),
    )
    .await;

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STDIN_ATTENTION,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("update_history"));
    assert_eq!(
        row.target_id.as_deref(),
        Some(update_history_id.to_string().as_str())
    );
    let details = row
        .details_json
        .expect("software.update.stdin_attention details");
    assert_eq!(details["hint_present"], serde_json::json!(true));
    assert_eq!(details["hint_length"], serde_json::json!(14));
    assert_eq!(details["interactive"], serde_json::json!(true));
    assert!(
        !details.to_string().contains("Enter password"),
        "semantic audit details must not store raw stdin hints"
    );
}

#[tokio::test]
async fn load_pending_update_records_skips_deactivated_hosts() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
    let service_id = Uuid::now_v7();

    insert_service_row(state.db(), tenant_id, service_id).await;
    let host_id = insert_linked_host(state.db(), tenant_id, service_id).await;
    let software_item_id = insert_software_item(state.db(), tenant_id, "deactivated").await;
    insert_replayable_queued_update(state.db(), tenant_id, host_id, software_item_id).await;

    host::ActiveModel {
        id: Set(host_id),
        deactivated_at: Set(Some(OffsetDateTime::now_utc())),
        ..Default::default()
    }
    .update(state.db())
    .await
    .unwrap();

    let records = load_pending_update_records(&state, service_id)
        .await
        .unwrap();

    assert!(
        records.is_none(),
        "deactivated hosts must not produce pending replay work"
    );
}

#[tokio::test]
async fn select_best_output_truncates_agent_output_on_utf8_boundary() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

    let agent_output = format!("{}étail", "a".repeat(MAX_UPDATE_OUTPUT_BYTES - 1));

    let (output, truncated) = select_best_output(&state, Uuid::now_v7(), agent_output).await;

    assert!(truncated);
    assert_eq!(output, "a".repeat(MAX_UPDATE_OUTPUT_BYTES - 1));
}
