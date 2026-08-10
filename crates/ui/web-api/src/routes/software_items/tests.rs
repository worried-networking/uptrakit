//! Tests for the software_items route handlers.
#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]
#![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

use super::audit::{
    SOFTWARE_ITEM_APPROVE_AUDIT_ACTION, SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION,
    SOFTWARE_ITEM_BATCH_AUDIT_ACTION, SOFTWARE_ITEM_CREATE_AUDIT_ACTION,
    SOFTWARE_ITEM_DELETE_AUDIT_ACTION, SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION,
    SOFTWARE_ITEM_MERGE_AUDIT_ACTION, SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION,
    SOFTWARE_ITEM_UPDATE_AUDIT_ACTION, SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION,
};
use super::*;
use crate::AppState;
use crate::app_state::AuditEmitterState;
use crate::auth::AuthMethod;
use crate::extract::{Unvalidated, Validated};
use crate::middleware::action::{
    CanCreateSoftware, CanDeleteSoftware, CanTriggerChecks, CanTriggerUpdates, CanUpdateSoftware,
};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
use crate::queries::update_types::ActorType;
use crate::tenant_db::TenantDb;
use crate::test_harness::{
    build_test_state_with_plugin_ops, insert_default_tenant, setup_migrated_db,
};
use async_trait::async_trait;
use axum::{
    Extension,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::{
    CatalogConfig, ControllerPostUpdateContext, ControllerProtectionContext,
    ControllerProtectionDecision, ControllerUpdateHookOps, ControllerUpdateProtection,
    ControllerUpdateProtectionOps, NotificationOps, NotificationTransport, PluginConfigOps,
    PluginMetadataOps, PluginOps, PluginSurfaceActionOps, PluginSurfaceOps, PostUpdateOutcome,
    SoftwareItemCreatedEvent, SoftwareItemLifecycle, SoftwareItemLifecycleContext,
    SoftwareItemLifecycleOps, SoftwareItemPatch, SurfaceActionContext, SurfaceActionError,
    build_catalog,
};
use uptrakit_shared_db::entity::{
    audit_log, host_software_item, host_software_item_plugin, plugin_config, service,
    software_item, update_history,
};
use uptrakit_web_api_types::PluginRole;
use uuid::Uuid;

struct SkipProtection;

impl uptrakit_plugin_infrastructure_registry::PluginMeta for SkipProtection {
    fn plugin_type_id(&self) -> uptrakit_shared_types::PluginTypeId {
        uptrakit_shared_types::PluginTypeId::from_static("test_skip_protection")
    }
}

#[async_trait]
impl ControllerUpdateProtection for SkipProtection {
    async fn prepare_pre_update_protection(
        &self,
        _ctx: &ControllerProtectionContext<'_>,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<ControllerProtectionDecision> {
        Ok(ControllerProtectionDecision::skipped(Some(
            "test skipped protection".to_string(),
        )))
    }

    async fn finalize_post_update(
        &self,
        _ctx: &ControllerPostUpdateContext<'_>,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<PostUpdateOutcome> {
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

async fn build_test_state_without_real_protection(
    db: DatabaseConnection,
    tenant_id: Uuid,
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
        protection: Arc::new(SkipProtection),
    });
    let (state, _jwt) = build_test_state_with_plugin_ops(db, tenant_id, Some(plugin_ops)).await;
    state
}

async fn setup_state() -> (DatabaseConnection, Uuid, Arc<AppState>, TenantDb) {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
    (db, tenant_id, state, tenant_db)
}

async fn insert_software_item_row(db: &DatabaseConnection, tenant_id: Uuid, item_id: Uuid) {
    let now = OffsetDateTime::now_utc();
    software_item::ActiveModel {
        id: Set(item_id),
        tenant_id: Set(tenant_id),
        name: Set("Audit App".to_string()),
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
    .expect("insert software item");
}

async fn insert_software_item_row_with_flags(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    item_id: Uuid,
    name: &str,
    featured: bool,
) {
    let now = OffsetDateTime::now_utc();
    software_item::ActiveModel {
        id: Set(item_id),
        tenant_id: Set(tenant_id),
        name: Set(name.to_string()),
        featured: Set(featured),
        icon_url: Set(None),
        last_checked_at: Set(None),
        awaiting_restart_timeout: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert software item with flags");
}

async fn insert_host_assignment(db: &DatabaseConnection, host_id: Uuid, item_id: Uuid) -> Uuid {
    let now = OffsetDateTime::now_utc();
    let host_software_item_id = Uuid::now_v7();
    host_software_item::ActiveModel {
        id: Set(host_software_item_id),
        host_id: Set(host_id),
        software_item_id: Set(item_id),
        qualifier: Set(None),
        plugin_config_id: Set(None),
        package_identifier: Set(None),
        installed_version: Set(Some("1.0.0".to_string())),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(Some("1.1.0".to_string())),
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
    .expect("insert host assignment");
    host_software_item_id
}

async fn insert_execute_update_plugin(
    db: &DatabaseConnection,
    host_id: Uuid,
    item_id: Uuid,
    host_software_item_id: Uuid,
) {
    let now = OffsetDateTime::now_utc();
    host_software_item_plugin::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(item_id),
        host_software_item_id: Set(host_software_item_id),
        plugin_config_id: Set(None),
        plugin_type: Set("package-manager.apt".to_string()),
        role: Set("execute_update".to_string()),
        ordinal: Set(0),
        package_identifier: Set("pkg".to_string()),
        config: Set(None),
        execution_site: Set("auto".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert execute_update plugin");
}

async fn insert_detect_version_plugin(
    db: &DatabaseConnection,
    host_id: Uuid,
    item_id: Uuid,
    host_software_item_id: Uuid,
    execution_site: &str,
) {
    let now = OffsetDateTime::now_utc();
    host_software_item_plugin::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(item_id),
        host_software_item_id: Set(host_software_item_id),
        plugin_config_id: Set(None),
        plugin_type: Set("package-manager.apt".to_string()),
        role: Set("detect_version".to_string()),
        ordinal: Set(0),
        package_identifier: Set("pkg".to_string()),
        config: Set(None),
        execution_site: Set(execution_site.to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert detect_version plugin");
}

async fn tenant_audit_row_for_action(
    db: &DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("expected tenant audit row");
}

async fn tenant_audit_row_for_action_and_outcome(
    db: &DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    outcome: &'static str,
) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .filter(audit_log::Column::Outcome.eq(outcome))
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("expected tenant audit row with outcome");
}

fn test_auth_user() -> AuthenticatedUser {
    AuthenticatedUser::new(Uuid::now_v7(), AuthMethod::Password, None)
}

#[tokio::test]
async fn create_software_item_writes_success_audit_event() {
    let (db, _tenant_id, state, tenant_db) = setup_state().await;

    let response = create_software_item(
        State(Arc::clone(&state)),
        tenant_db,
        CanCreateSoftware::new(test_auth_user()),
        None,
        Validated(CreateSoftwareItemRequest {
            name: "Create Audit App".to_string(),
            featured: true,
            icon_url: None,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_CREATE_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    let details = row.details_json.expect("details");
    assert_eq!(details["featured"], serde_json::json!(true));
}

#[tokio::test]
async fn create_software_item_duplicate_writes_validation_failed_audit_event() {
    let (db, tenant_id, state, tenant_db) = setup_state().await;
    let req = CreateSoftwareItemRequest {
        name: "Duplicate Create App".to_string(),
        featured: false,
        icon_url: None,
    };

    let first = create_software_item(
        State(Arc::clone(&state)),
        tenant_db,
        CanCreateSoftware::new(test_auth_user()),
        None,
        Validated(CreateSoftwareItemRequest {
            name: req.name.clone(),
            featured: req.featured,
            icon_url: req.icon_url.clone(),
        }),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);

    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let err = create_software_item(
        State(Arc::clone(&state)),
        tenant_db,
        CanCreateSoftware::new(test_auth_user()),
        None,
        Validated(req),
    )
    .await;
    assert_eq!(err.status(), StatusCode::CONFLICT);

    let row = tenant_audit_row_for_action_and_outcome(
        &db,
        SOFTWARE_ITEM_CREATE_AUDIT_ACTION,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str(),
    )
    .await;
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("software_item.duplicate_item")
    );
}

#[tokio::test]
async fn update_software_item_missing_item_writes_denied_audit_event() {
    let (db, _tenant_id, state, tenant_db) = setup_state().await;
    let missing_item_id = Uuid::now_v7();

    let response = update_software_item(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        None,
        Path(missing_item_id),
        Unvalidated::new_for_test(UpdateSoftwareItemRequest {
            name: Some("Nope".to_string()),
            featured: None,
            icon_url: Default::default(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_UPDATE_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(
        row.target_id.as_deref(),
        Some(missing_item_id.to_string().as_str())
    );
}

#[tokio::test]
async fn delete_software_item_missing_item_writes_denied_audit_event() {
    let (db, _tenant_id, state, tenant_db) = setup_state().await;
    let missing_item_id = Uuid::now_v7();

    let response = delete_software_item(
        State(Arc::clone(&state)),
        tenant_db,
        CanDeleteSoftware::new(test_auth_user()),
        None,
        Path(missing_item_id),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_DELETE_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("software_item.not_found")
    );
}

#[tokio::test]
async fn approve_software_item_already_featured_writes_denied_audit_event() {
    let (db, tenant_id, state, tenant_db) = setup_state().await;
    let item_id = Uuid::now_v7();
    insert_software_item_row_with_flags(&db, tenant_id, item_id, "Featured App", true).await;

    let response = approve_software_item(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        None,
        Path(item_id),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_APPROVE_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("software_item.already_featured")
    );
}

#[tokio::test]
async fn assign_hosts_empty_payload_writes_validation_failed_audit_event() {
    let (db, tenant_id, state, tenant_db) = setup_state().await;
    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;

    let response = assign_hosts(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        None,
        Path(item_id),
        Unvalidated::new_for_test(AssignHostsRequest {
            host_assignments: vec![],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
}

#[tokio::test]
async fn unassign_host_missing_assignment_writes_denied_audit_event() {
    let (db, tenant_id, state, tenant_db) = setup_state().await;
    let item_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;

    let response = unassign_host(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        None,
        Path((item_id, host_id)),
        Query(DeleteHostAssignmentParams {
            ignore: Some(false),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("software_item.assignment_not_found")
    );
}

#[tokio::test]
async fn update_host_assignment_missing_item_writes_denied_audit_event() {
    let (db, _tenant_id, state, tenant_db) = setup_state().await;
    let item_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();

    let response = update_host_assignment(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        None,
        Path((item_id, host_id)),
        Unvalidated::new_for_test(UpdateHostAssignmentRequest {
            role: PluginRole::DetectVersion,
            ordinal: 0,
            plugin_config_id: Some(Uuid::now_v7()),
            plugin_config: None,
            plugin_type: None,
            package_identifier: Some("pkg".to_string()),
            config_override: Default::default(),
            execution_site: None,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let row =
        tenant_audit_row_for_action(&db, SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
}

#[tokio::test]
async fn zero_source_patch_keeps_config_backed_plugin_source() {
    let (db, tenant_id, state, tenant_db) = setup_state().await;
    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;
    let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;

    let plugin_config_id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    plugin_config::ActiveModel {
        id: Set(plugin_config_id),
        tenant_id: Set(tenant_id),
        name: Set("Test Apt Config".to_string()),
        plugin_type: Set("package-manager.apt".to_string()),
        config: Set(serde_json::json!({})),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert test plugin_config");

    host_software_item_plugin::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host.id),
        software_item_id: Set(item_id),
        host_software_item_id: Set(host_software_item_id),
        plugin_config_id: Set(Some(plugin_config_id)),
        plugin_type: Set("package-manager.apt".to_string()),
        role: Set("detect_version".to_string()),
        ordinal: Set(0),
        package_identifier: Set("nginx".to_string()),
        config: Set(None),
        execution_site: Set("auto".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .expect("insert config-backed detect_version plugin");

    let response = update_host_assignment(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        None,
        Path((item_id, host.id)),
        Unvalidated::new_for_test(UpdateHostAssignmentRequest {
            role: PluginRole::DetectVersion,
            ordinal: 0,
            plugin_config_id: None,
            plugin_config: None,
            plugin_type: None,
            package_identifier: Some("renamed-pkg".to_string()),
            config_override: Default::default(),
            execution_site: None,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("deserialize body");
    let host_summary = body["hosts"]
        .as_array()
        .expect("hosts array")
        .iter()
        .find(|h| h["host_id"] == serde_json::json!(host.id))
        .expect("host summary present");
    let plugin_entry = host_summary["plugins"]
        .as_array()
        .expect("plugins array")
        .iter()
        .find(|p| p["role"] == serde_json::json!("detect_version"))
        .expect("detect_version plugin entry present");
    assert_eq!(
        plugin_entry["plugin_config_id"],
        serde_json::json!(plugin_config_id)
    );
    assert_eq!(
        plugin_entry["package_identifier"],
        serde_json::json!("renamed-pkg")
    );
}

#[tokio::test]
async fn zero_source_patch_keeps_type_only_plugin_source() {
    let (db, tenant_id, state, tenant_db) = setup_state().await;
    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;
    let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;
    insert_detect_version_plugin(&db, host.id, item_id, host_software_item_id, "auto").await;

    let response = update_host_assignment(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        None,
        Path((item_id, host.id)),
        Unvalidated::new_for_test(UpdateHostAssignmentRequest {
            role: PluginRole::DetectVersion,
            ordinal: 0,
            plugin_config_id: None,
            plugin_config: None,
            plugin_type: None,
            package_identifier: Some("renamed-pkg".to_string()),
            config_override: Default::default(),
            execution_site: None,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("deserialize body");
    let host_summary = body["hosts"]
        .as_array()
        .expect("hosts array")
        .iter()
        .find(|h| h["host_id"] == serde_json::json!(host.id))
        .expect("host summary present");
    let plugin_entry = host_summary["plugins"]
        .as_array()
        .expect("plugins array")
        .iter()
        .find(|p| p["role"] == serde_json::json!("detect_version"))
        .expect("detect_version plugin entry present");
    assert_eq!(plugin_entry["plugin_config_id"], serde_json::Value::Null);
    assert_eq!(
        plugin_entry["plugin_type"],
        serde_json::json!("package-manager.apt")
    );
    assert_eq!(
        plugin_entry["package_identifier"],
        serde_json::json!("renamed-pkg")
    );
}

#[tokio::test]
async fn zero_source_patch_with_no_existing_row_is_rejected() {
    let (db, tenant_id, state, tenant_db) = setup_state().await;
    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;
    insert_host_assignment(&db, host.id, item_id).await;

    let response = update_host_assignment(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        None,
        Path((item_id, host.id)),
        Unvalidated::new_for_test(UpdateHostAssignmentRequest {
            role: PluginRole::FetchReleases,
            ordinal: 0,
            plugin_config_id: None,
            plugin_config: None,
            plugin_type: None,
            package_identifier: None,
            config_override: Default::default(),
            execution_site: None,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body: uptrakit_web_api_types::error::ErrorResponse =
        serde_json::from_slice(&bytes).expect("deserialize error response");
    assert!(
        body.error.contains("no plugin source"),
        "expected 'no plugin source' in error message, got: {}",
        body.error
    );

    let row = tenant_audit_row_for_action_and_outcome(
        &db,
        SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str(),
    )
    .await;
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("software_item.missing_plugin_source")
    );
}

#[tokio::test]
async fn delete_plugin_assignment_invalid_role_writes_validation_failed_audit_event() {
    let (db, _tenant_id, state, tenant_db) = setup_state().await;
    let item_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();

    let response = delete_plugin_assignment(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        None,
        Path((item_id, host_id, "invalid_role".to_string(), 0)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let row =
        tenant_audit_row_for_action(&db, SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("software_item.invalid_role")
    );
}

#[tokio::test]
async fn execute_merge_invalid_request_writes_validation_failed_audit_event() {
    let (db, _tenant_id, state, tenant_db) = setup_state().await;
    let only_id = Uuid::now_v7();

    let err = match execute_software_item_merge(
        State(AuditEmitterState(state.audit_emitter.clone())),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        CanDeleteSoftware::new(test_auth_user()),
        None,
        Unvalidated::new_for_test(MergeSoftwareItemsExecuteRequest {
            candidate_ids: vec![only_id],
            survivor_id: only_id,
        }),
    )
    .await
    {
        Ok(response) => panic!(
            "invalid merge request should fail, got status {}",
            response.into_response().status()
        ),
        Err(err) => err,
    };
    assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_MERGE_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("software_item.invalid_merge_request")
    );
}

#[tokio::test]
async fn execute_merge_empty_candidates_writes_validation_failed_audit_event() {
    let (db, _tenant_id, state, tenant_db) = setup_state().await;

    let err = match execute_software_item_merge(
        State(AuditEmitterState(state.audit_emitter.clone())),
        tenant_db,
        CanUpdateSoftware::new(test_auth_user()),
        CanDeleteSoftware::new(test_auth_user()),
        None,
        Unvalidated::new_for_test(MergeSoftwareItemsExecuteRequest {
            candidate_ids: vec![],
            survivor_id: Uuid::now_v7(),
        }),
    )
    .await
    {
        Ok(response) => panic!(
            "empty candidate_ids should fail require_valid(), got status {}",
            response.into_response().status()
        ),
        Err(err) => err,
    };
    assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_MERGE_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
}

#[tokio::test]
async fn batch_software_items_partial_result_writes_partial_audit_event() {
    let (db, tenant_id, state, tenant_db) = setup_state().await;
    let existing_item_id = Uuid::now_v7();
    let missing_item_id = Uuid::now_v7();
    insert_software_item_row_with_flags(
        &db,
        tenant_id,
        existing_item_id,
        "Batch Partial App",
        false,
    )
    .await;

    let response = batch_software_items(
        State(Arc::clone(&state)),
        tenant_db,
        CanDeleteSoftware::new(test_auth_user()),
        None,
        Validated(BatchActionRequest {
            action: "approve".to_string(),
            ids: vec![existing_item_id, missing_item_id],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_BATCH_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Partial.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["requested_count"], serde_json::json!(2));
    assert_eq!(details["succeeded_count"], serde_json::json!(1));
    assert_eq!(details["failed_count"], serde_json::json!(1));
}

#[tokio::test]
async fn batch_software_items_unknown_action_writes_validation_failed_audit_event() {
    let (db, tenant_id, state, tenant_db) = setup_state().await;
    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;

    let response = batch_software_items(
        State(Arc::clone(&state)),
        tenant_db,
        CanDeleteSoftware::new(test_auth_user()),
        None,
        Validated(BatchActionRequest {
            action: "invalid".to_string(),
            ids: vec![item_id],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_BATCH_AUDIT_ACTION).await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("software_item.batch_unknown_action")
    );
}

#[tokio::test]
async fn trigger_update_writes_software_update_triggered_audit_event() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let service = crate::test_harness::fixtures::insert_service(
        &db,
        tenant_id,
        service::ServiceStatus::Approved,
    )
    .await;
    crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;
    let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;
    insert_execute_update_plugin(&db, host.id, item_id, host_software_item_id).await;

    let auth_user = AuthenticatedUser::new(Uuid::now_v7(), AuthMethod::Password, None);
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = trigger_update(
        State(Arc::clone(&state)),
        tenant_db,
        CanTriggerUpdates::new(auth_user),
        None,
        Path((item_id, host.id)),
        Unvalidated::new_for_test(TriggerUpdateRequest {
            to_version: "1.1.0".to_string(),
            release_info: None,
            interactive: false,
        }),
    )
    .await;

    let response = match response {
        Ok(response) => response,
        Err(err) => panic!(
            "trigger_update should succeed, got status {}",
            err.into_response().status()
        ),
    };
    assert_eq!(response.into_response().status(), StatusCode::OK);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    assert_eq!(row.target_id.as_deref(), Some(item_id.to_string().as_str()));
    let details = row.details_json.expect("details");
    assert_eq!(details["host_id"], serde_json::json!(host.id));
    assert_eq!(details["to_version"], serde_json::json!("1.1.0"));
    assert_eq!(details["interactive"], serde_json::json!(false));
    assert_eq!(details["dispatch_status"], serde_json::json!("pending"));

    let update_row = update_history::Entity::find()
        .filter(update_history::Column::SoftwareItemId.eq(item_id))
        .filter(update_history::Column::HostId.eq(host.id))
        .one(&db)
        .await
        .expect("query update history")
        .expect("update history row");
    assert_eq!(update_row.actor_type, ActorType::User.as_str());
}

#[tokio::test]
async fn trigger_update_host_not_assigned_writes_validation_failed_audit_event() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let service = crate::test_harness::fixtures::insert_service(
        &db,
        tenant_id,
        service::ServiceStatus::Approved,
    )
    .await;
    crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;

    let auth_user = AuthenticatedUser::new(Uuid::now_v7(), AuthMethod::Password, None);
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = trigger_update(
        State(Arc::clone(&state)),
        tenant_db,
        CanTriggerUpdates::new(auth_user),
        None,
        Path((item_id, host.id)),
        Unvalidated::new_for_test(TriggerUpdateRequest {
            to_version: "1.1.0".to_string(),
            release_info: None,
            interactive: false,
        }),
    )
    .await;

    let error = match response {
        Ok(response) => panic!(
            "trigger_update should fail with host-not-assigned, got status {}",
            response.into_response().status()
        ),
        Err(err) => err,
    };
    assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    assert_eq!(row.target_id.as_deref(), Some(item_id.to_string().as_str()));
    let details = row.details_json.expect("details");
    assert_eq!(details["host_id"], serde_json::json!(host.id));
    assert_eq!(details["to_version"], serde_json::json!("1.1.0"));
    assert_eq!(details["interactive"], serde_json::json!(false));
    assert_eq!(
        details["reason_code"],
        serde_json::json!("trigger_update.host_not_assigned")
    );
}

#[tokio::test]
async fn trigger_update_missing_item_writes_denied_audit_event() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let service = crate::test_harness::fixtures::insert_service(
        &db,
        tenant_id,
        service::ServiceStatus::Approved,
    )
    .await;
    crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

    let missing_item_id = Uuid::now_v7();

    let auth_user = AuthenticatedUser::new(Uuid::now_v7(), AuthMethod::Password, None);
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = trigger_update(
        State(Arc::clone(&state)),
        tenant_db,
        CanTriggerUpdates::new(auth_user),
        None,
        Path((missing_item_id, host.id)),
        Unvalidated::new_for_test(TriggerUpdateRequest {
            to_version: "1.1.0".to_string(),
            release_info: None,
            interactive: true,
        }),
    )
    .await;

    let error = match response {
        Ok(response) => panic!(
            "trigger_update should fail with software-item-not-found, got status {}",
            response.into_response().status()
        ),
        Err(err) => err,
    };
    assert_eq!(error.into_response().status(), StatusCode::NOT_FOUND);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    assert_eq!(
        row.target_id.as_deref(),
        Some(missing_item_id.to_string().as_str())
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["host_id"], serde_json::json!(host.id));
    assert_eq!(details["to_version"], serde_json::json!("1.1.0"));
    assert_eq!(details["interactive"], serde_json::json!(true));
    assert_eq!(
        details["reason_code"],
        serde_json::json!("trigger_update.software_item_not_found")
    );
}

#[tokio::test]
async fn trigger_update_with_api_token_actor_writes_api_token_actor_id() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let service = crate::test_harness::fixtures::insert_service(
        &db,
        tenant_id,
        service::ServiceStatus::Approved,
    )
    .await;
    crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;
    let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;
    insert_execute_update_plugin(&db, host.id, item_id, host_software_item_id).await;

    let token_id = Uuid::now_v7();
    let auth_user = AuthenticatedUser::new(Uuid::now_v7(), AuthMethod::ApiToken, None);
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = trigger_update(
        State(Arc::clone(&state)),
        tenant_db,
        CanTriggerUpdates::new(auth_user),
        Some(Extension(AuthenticatedApiTokenId(token_id))),
        Path((item_id, host.id)),
        Unvalidated::new_for_test(TriggerUpdateRequest {
            to_version: "1.1.0".to_string(),
            release_info: None,
            interactive: false,
        }),
    )
    .await;

    let response = match response {
        Ok(response) => response,
        Err(err) => panic!(
            "trigger_update should succeed, got status {}",
            err.into_response().status()
        ),
    };
    assert_eq!(response.into_response().status(), StatusCode::OK);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::ApiToken.as_str()
    );
    assert_eq!(row.actor_id, Some(token_id));

    let update_row = update_history::Entity::find()
        .filter(update_history::Column::SoftwareItemId.eq(item_id))
        .filter(update_history::Column::HostId.eq(host.id))
        .one(&db)
        .await
        .expect("query update history")
        .expect("update history row");
    assert_eq!(update_row.actor_type, ActorType::ApiToken.as_str());
    assert_eq!(update_row.actor_id, token_id.to_string());
}

#[tokio::test]
async fn check_versions_writes_software_version_check_triggered_audit_event() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let service = crate::test_harness::fixtures::insert_service(
        &db,
        tenant_id,
        service::ServiceStatus::Approved,
    )
    .await;
    crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;
    let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;
    insert_detect_version_plugin(&db, host.id, item_id, host_software_item_id, "agent").await;

    let auth_user = AuthenticatedUser::new(Uuid::now_v7(), AuthMethod::Password, None);
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = check_versions(
        State(Arc::clone(&state)),
        tenant_db,
        CanTriggerChecks::new(auth_user),
        None,
        Path(item_id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    assert_eq!(row.target_id.as_deref(), Some(item_id.to_string().as_str()));
    let details = row.details_json.expect("details");
    assert_eq!(details["dispatch_scope"], serde_json::json!("all_hosts"));
    assert_eq!(details["agents_notified"], serde_json::json!(1));
    assert_eq!(details["controller_checks_run"], serde_json::json!(0));
}

#[tokio::test]
async fn check_versions_host_writes_software_version_check_triggered_audit_event() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let service = crate::test_harness::fixtures::insert_service(
        &db,
        tenant_id,
        service::ServiceStatus::Approved,
    )
    .await;
    crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;
    let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;
    insert_detect_version_plugin(&db, host.id, item_id, host_software_item_id, "agent").await;

    let auth_user = AuthenticatedUser::new(Uuid::now_v7(), AuthMethod::Password, None);
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = check_versions_host(
        State(Arc::clone(&state)),
        tenant_db,
        CanTriggerChecks::new(auth_user),
        None,
        Path((item_id, host.id)),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    assert_eq!(row.target_id.as_deref(), Some(item_id.to_string().as_str()));
    let details = row.details_json.expect("details");
    assert_eq!(details["dispatch_scope"], serde_json::json!("single_host"));
    assert_eq!(details["host_id"], serde_json::json!(host.id));
    assert_eq!(details["agents_notified"], serde_json::json!(1));
    assert_eq!(details["controller_checks_run"], serde_json::json!(0));
}

#[tokio::test]
async fn check_versions_host_missing_assignment_writes_validation_failed_audit_event() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

    let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
    let item_id = Uuid::now_v7();
    insert_software_item_row(&db, tenant_id, item_id).await;

    let auth_user = AuthenticatedUser::new(Uuid::now_v7(), AuthMethod::Password, None);
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = check_versions_host(
        State(Arc::clone(&state)),
        tenant_db,
        CanTriggerChecks::new(auth_user),
        None,
        Path((item_id, host.id)),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let row = tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["dispatch_scope"], serde_json::json!("single_host"));
    assert_eq!(details["host_id"], serde_json::json!(host.id));
    assert_eq!(
        details["reason_code"],
        serde_json::json!("version_check.host_not_assigned")
    );
}
