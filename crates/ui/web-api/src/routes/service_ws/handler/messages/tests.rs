#![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]
#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]

use super::CertIdentity;
use super::discovery::enrich_discovered_items;
use super::version_check::{DisplayOverride, apply_version_update_to_db};
use super::*;
use crate::AppState;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use uptrakit_plugin_infrastructure_registry::{
    ControllerUpdateHookOps, ControllerUpdateProtection, ControllerUpdateProtectionOps,
    NotificationOps, NotificationTransport, PluginConfigOps, PluginDescriptor, PluginMetadataOps,
    PluginOps, PluginSurfaceActionOps, PluginSurfaceOps, PluginTypeId, SoftwareItemCreatedEvent,
    SoftwareItemLifecycle, SoftwareItemLifecycleContext, SoftwareItemLifecycleOps,
    SoftwareItemPatch, SurfaceActionError, plugin_ids,
};
use uptrakit_shared_db::entity::{
    audit_log, ca_certificate, host, host_software_item, plugin_config, service, service_host,
    software_item, system_audit_log, system_service,
};
use uptrakit_wire::report_tracker::ReportTracker;
use uptrakit_wire::{
    Capability, ControllerMessage, DiscoveredSoftware, DiscoveryPluginResult,
    DiscoveryResultsPayload, ErrorCode, HostInfo, RenewCertificatePayload, ReportHostsPayload,
    ReportPluginConfigPayload, ReportPluginConfigResponsePayload, UpdateCategory,
    VersionCheckResult, VersionCheckResultsPayload,
};
use uuid::Uuid;

use crate::embedded_support::EmbeddedServiceNotifier;
use crate::test_harness::{
    build_test_state, build_test_state_with_plugin_ops, insert_default_tenant, setup_migrated_db,
};

const TEST_CA_FINGERPRINT: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct TestPluginOps;
struct TestLifecyclePlugin;
struct TestSuccessfulCertSigner;
struct TestFailingCertSigner;
#[derive(Default)]
struct TestEmbeddedNotifier {
    machine_ids: parking_lot::Mutex<Vec<(Uuid, String)>>,
}

impl EmbeddedServiceNotifier for TestEmbeddedNotifier {
    fn on_external_connected(
        &self,
        _service_id: Uuid,
        _capabilities: &std::collections::BTreeSet<Capability>,
        _hostname: Option<&str>,
        _is_system: bool,
    ) {
    }

    fn on_external_disconnected(&self, _service_id: &Uuid) {}

    fn on_machine_id_reported(&self, service_id: &Uuid, machine_id: &str) {
        self.machine_ids
            .lock()
            .push((*service_id, machine_id.to_string()));
    }

    fn is_capability_yielded(&self, _capability: &Capability) -> bool {
        false
    }
}

#[derive(Debug, Deserialize)]
struct TestLifecycleTypeSettings {
    #[serde(default = "default_lifecycle_enabled")]
    enabled: bool,
}

const fn default_lifecycle_enabled() -> bool {
    true
}

static TEST_LIFECYCLE_PLUGINS: OnceLock<Vec<Arc<dyn SoftwareItemLifecycle>>> = OnceLock::new();

fn lifecycle_plugins() -> &'static [Arc<dyn SoftwareItemLifecycle>] {
    TEST_LIFECYCLE_PLUGINS
        .get_or_init(|| vec![Arc::new(TestLifecyclePlugin)])
        .as_slice()
}

async fn insert_embedded_service(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    service_id: Uuid,
) {
    let now = time::OffsetDateTime::now_utc();
    service::ActiveModel {
        id: Set(service_id),
        tenant_id: Set(tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set("embedded-agent-host".to_string()),
        friendly_name: Set("Embedded Agent".to_string()),
        ip_address: Set(None),
        status: Set(uptrakit_shared_types::ServiceStatus::Approved),
        enrollment_secret_hash: Set(format!("embedded-secret-{service_id}")),
        client_version: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        enrollment_token_id: Set(None),
        cert_lifetime_hours: Set(None),
        service_app_name: Set(Some("uptrakit-agent".to_string())),
        is_embedded: Set(true),
        embedded_owner_key: Set(None),
    }
    .insert(db)
    .await
    .expect("insert embedded service");
}

#[async_trait::async_trait]
impl SoftwareItemLifecycle for TestLifecyclePlugin {
    async fn on_software_item_created(
        &self,
        _event: &SoftwareItemCreatedEvent,
        _ctx: &SoftwareItemLifecycleContext,
    ) -> std::result::Result<
        Option<SoftwareItemPatch>,
        uptrakit_plugin_infrastructure_registry::PluginError,
    > {
        Ok(None)
    }
}

impl uptrakit_plugin_infrastructure_registry::PluginMeta for TestLifecyclePlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        plugin_ids::ENHANCEMENT_DASHBOARD_ICONS
    }
}

impl PluginMetadataOps for TestPluginOps {
    fn get(&self, _id: &PluginTypeId) -> Option<&PluginDescriptor> {
        None
    }
    fn all(&self) -> Vec<&PluginDescriptor> {
        vec![]
    }
    fn instance_enabled(&self, _id: &PluginTypeId) -> bool {
        true
    }
}

impl PluginConfigOps for TestPluginOps {}

#[async_trait::async_trait]
impl PluginSurfaceActionOps for TestPluginOps {
    async fn handle_surface_action(
        &self,
        _ctx: &uptrakit_plugin_infrastructure_registry::SurfaceActionContext<'_>,
        _surface_id: &str,
        _action_id: &str,
        _params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, SurfaceActionError> {
        Err(SurfaceActionError::PluginInternal(
            "not implemented".to_string(),
        ))
    }
}

impl PluginSurfaceOps for TestPluginOps {
    fn surface_registrations(&self) -> Vec<uptrakit_wire::surfaces::SurfaceRegistration> {
        Vec::new()
    }
}

#[async_trait::async_trait]
impl NotificationOps for TestPluginOps {
    fn transport(&self, _id: &PluginTypeId) -> Option<std::sync::Arc<dyn NotificationTransport>> {
        None
    }
    fn notification_supported_types(&self) -> Vec<PluginTypeId> {
        vec![]
    }
}

#[async_trait::async_trait]
impl SoftwareItemLifecycleOps for TestPluginOps {
    async fn on_software_item_created(
        &self,
        event: &SoftwareItemCreatedEvent,
        ctx: &SoftwareItemLifecycleContext,
    ) -> Option<SoftwareItemPatch> {
        let enabled = ctx
            .typed_type_setting::<TestLifecycleTypeSettings>(
                &plugin_ids::ENHANCEMENT_DASHBOARD_ICONS,
            )
            .map(|cfg| cfg.enabled)
            .unwrap_or(true);

        if !enabled {
            return None;
        }

        if event.name == "Actual Budget" {
            Some(
                SoftwareItemPatch::new()
                    .with_icon_url(Some("https://cdn.example.test/actual-budget.svg".into())),
            )
        } else {
            None
        }
    }

    fn software_item_lifecycle_plugins(&self) -> &[std::sync::Arc<dyn SoftwareItemLifecycle>] {
        lifecycle_plugins()
    }
}

impl ControllerUpdateProtectionOps for TestPluginOps {
    fn controller_update_protection(
        &self,
    ) -> Option<std::sync::Arc<dyn ControllerUpdateProtection>> {
        None
    }
}

impl ControllerUpdateHookOps for TestPluginOps {}

#[async_trait::async_trait]
impl crate::cert_signer::AgentCertSigner for TestSuccessfulCertSigner {
    async fn sign_agent_csr(
        &self,
        _csr_pem: &str,
        _agent_id: &Uuid,
        _lifetime: time::Duration,
    ) -> std::result::Result<
        crate::cert_signer::SignedCertBundle,
        rootcause::Report<crate::cert_signer::CertSignerError>,
    > {
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)
            .expect("key generation should succeed");
        let cert = rcgen::CertificateParams::new(vec!["localhost".to_string()])
            .expect("certificate params should be valid")
            .self_signed(&key_pair)
            .expect("certificate should self-sign");
        let not_after = time::UtcDateTime::from_unix_timestamp(
            (time::OffsetDateTime::now_utc() + time::Duration::days(30)).unix_timestamp(),
        )
        .expect("valid not_after timestamp");

        Ok(crate::cert_signer::SignedCertBundle {
            cert_pem: cert.pem(),
            not_after,
        })
    }

    fn active_ca_fingerprint(&self) -> String {
        TEST_CA_FINGERPRINT.to_string()
    }
}

#[async_trait::async_trait]
impl crate::cert_signer::AgentCertSigner for TestFailingCertSigner {
    async fn sign_agent_csr(
        &self,
        _csr_pem: &str,
        _agent_id: &Uuid,
        _lifetime: time::Duration,
    ) -> std::result::Result<
        crate::cert_signer::SignedCertBundle,
        rootcause::Report<crate::cert_signer::CertSignerError>,
    > {
        Err(rootcause::report!(
            crate::cert_signer::CertSignerError::Signing("forced renewal failure".to_string())
        ))
    }

    fn active_ca_fingerprint(&self) -> String {
        TEST_CA_FINGERPRINT.to_string()
    }
}
// ── Fixture helpers ───────────────────────────────────────────────────

fn state_with_successful_cert_signer(state: &Arc<AppState>) -> Arc<AppState> {
    Arc::new(AppState {
        cert_signer: Arc::new(TestSuccessfulCertSigner),
        ..(**state).clone()
    })
}

fn state_with_failing_cert_signer(state: &Arc<AppState>) -> Arc<AppState> {
    Arc::new(AppState {
        cert_signer: Arc::new(TestFailingCertSigner),
        ..(**state).clone()
    })
}

fn test_renewal_csr_pem() -> String {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)
        .expect("key generation should succeed");
    let mut params = rcgen::CertificateParams::default();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, Uuid::now_v7().to_string());
    let csr = params
        .serialize_request(&key_pair)
        .expect("csr serialization should succeed");
    csr.pem().expect("csr pem encoding should succeed")
}

async fn insert_ca_certificate(db: &sea_orm::DatabaseConnection) {
    let now = time::OffsetDateTime::now_utc();
    ca_certificate::ActiveModel {
        fingerprint: Set(TEST_CA_FINGERPRINT.to_string()),
        cert_pem: Set("-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n".to_string()),
        key_pem: Set(uptrakit_crypto::EncryptedString::new(
            "test-key".to_string(),
            "uptrakit:ca_certificates:key_pem",
        )
        .expect("encrypt test CA key")),
        not_before: Set(now - time::Duration::days(1)),
        not_after: Set(now + time::Duration::days(365)),
        activated_at: Set(now),
        deactivated_at: Set(None),
        created_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert ca certificate");
}

#[expect(clippy::string_slice, reason = "UUID hex is always ASCII")]
async fn insert_service(db: &sea_orm::DatabaseConnection, tenant_id: uuid::Uuid) -> service::Model {
    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    service::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set(format!("svc-{}", &id.to_string()[..8])),
        friendly_name: Set(format!("Service {}", &id.to_string()[..8])),
        ip_address: Set(None),
        status: Set(service::ServiceStatus::Approved),
        enrollment_secret_hash: Set(format!("secret-{id}")),
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
    .expect("insert service")
}

#[expect(clippy::string_slice, reason = "UUID hex is always ASCII")]
async fn insert_system_service(db: &sea_orm::DatabaseConnection) -> system_service::Model {
    let id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    system_service::ActiveModel {
        id: Set(id),
        capabilities: Set("[]".to_string()),
        hostname: Set(format!("sys-{}", &id.to_string()[..8])),
        friendly_name: Set(format!("System Service {}", &id.to_string()[..8])),
        ip_address: Set(None),
        status: Set(system_service::SystemServiceStatus::Approved),
        enrollment_secret_hash: Set(format!("secret-{id}")),
        client_version: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        cert_lifetime_hours: Set(None),
        system_enrollment_token_id: Set(None),
        service_app_name: Set(Some("uptrakit-scheduler".to_string())),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    }
    .insert(db)
    .await
    .expect("insert system service")
}

async fn wait_for_tenant_audit_row_for_action(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query tenant audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("expected tenant audit row for action {action_type}");
}

async fn wait_for_system_audit_row_for_action(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> system_audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = system_audit_log::Entity::find()
            .filter(system_audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(system_audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query system audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("expected system audit row for action {action_type}");
}

async fn tenant_audit_count_for_action(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> usize {
    audit_log::Entity::find()
        .filter(audit_log::Column::ActionType.eq(action_type))
        .all(db)
        .await
        .expect("query tenant audit count")
        .len()
}

fn assert_report_plugin_config_reply(
    response: &ProcessorResponse,
) -> &ReportPluginConfigResponsePayload {
    let Some(reply) = response.replies.first() else {
        panic!("expected ReportPluginConfigResponse reply");
    };

    match reply {
        ControllerMessage::ReportPluginConfigResponse(payload) => payload,
        other => panic!("unexpected reply variant: {other:?}"),
    }
}

fn report_plugin_config_payload(
    request_id: &str,
    plugin_type: &str,
    name: &str,
    config: serde_json::Value,
) -> ReportPluginConfigPayload {
    serde_json::from_value(serde_json::json!({
        "request_id": request_id,
        "plugin_type": plugin_type,
        "name": name,
        "config": config,
    }))
    .expect("ReportPluginConfigPayload JSON is always valid")
}

fn assert_certificate_reply(response: &ProcessorResponse) {
    let Some(reply) = response.replies.first() else {
        panic!("expected certificate reply");
    };

    match reply {
        ControllerMessage::Certificate(_) => {}
        ControllerMessage::Error(err) => {
            panic!(
                "renew response returned error: code={}, message={}",
                err.code, err.message
            );
        }
        _ => panic!("unexpected renew response variant"),
    }
}

fn assert_error_reply(
    response: &ProcessorResponse,
    expected_code: ErrorCode,
    expected_message: &str,
) {
    let Some(reply) = response.replies.first() else {
        panic!("expected error reply");
    };

    match reply {
        ControllerMessage::Error(err) => {
            assert_eq!(err.code, expected_code);
            assert_eq!(err.message, expected_message);
        }
        other => panic!("unexpected reply variant: {other:?}"),
    }
}

fn assert_error_reply_contains(
    response: &ProcessorResponse,
    expected_code: ErrorCode,
    expected_message_fragment: &str,
) {
    let Some(reply) = response.replies.first() else {
        panic!("expected error reply");
    };

    match reply {
        ControllerMessage::Error(err) => {
            assert_eq!(err.code, expected_code);
            assert!(
                err.message.contains(expected_message_fragment),
                "expected error message to contain {expected_message_fragment:?}, got {:?}",
                err.message
            );
        }
        other => panic!("unexpected reply variant: {other:?}"),
    }
}

async fn insert_host(db: &sea_orm::DatabaseConnection, tenant_id: uuid::Uuid) -> host::Model {
    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    host::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        machine_id: Set(format!("machine-{id}")),
        hostname: Set(format!("host-{id}")),
        friendly_name: Set(format!("Host {id}")),
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
    .expect("insert host")
}

async fn link_service_host(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    let now = time::OffsetDateTime::now_utc();
    service_host::ActiveModel {
        service_id: Set(service_id),
        host_id: Set(host_id),
        linked_at: Set(now),
    }
    .insert(db)
    .await
    .expect("link service_host");
}

#[expect(clippy::string_slice, reason = "UUID hex is always ASCII")]
async fn insert_software_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> software_item::Model {
    insert_named_software_item(
        db,
        tenant_id,
        &format!("App-{}", &uuid::Uuid::now_v7().to_string()[..8]),
        false,
    )
    .await
}

async fn insert_named_software_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    name: &str,
    featured: bool,
) -> software_item::Model {
    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    software_item::ActiveModel {
        id: Set(id),
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
    .expect("insert software_item")
}

async fn insert_host_software_item(
    db: &sea_orm::DatabaseConnection,
    host_id: uuid::Uuid,
    software_item_id: uuid::Uuid,
) -> host_software_item::Model {
    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    host_software_item::ActiveModel {
        id: Set(id),
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
        last_discovered_at: Set(None),
        discovery_source: Set(None),
        missing_since: Set(None),
    }
    .insert(db)
    .await
    .expect("insert host_software_item")
}

async fn insert_plugin_config(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    plugin_type: &str,
) -> plugin_config::Model {
    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    plugin_config::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        name: Set(format!("Config-{id}")),
        plugin_type: Set(plugin_type.to_string()),
        config: Set(serde_json::json!({})),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert plugin_config")
}

// ── Shared fixture builders ───────────────────────────────────────────

/// Returns the `CertIdentity` used by all `handle_renew_certificate` tests.
fn test_cert_identity() -> CertIdentity {
    CertIdentity {
        serial: "old-serial".to_string(),
        ca_fingerprint: TEST_CA_FINGERPRINT.to_string(),
    }
}

/// Builds one service with one linked host and one `host_software_item` row.
/// Used by version-check and enricher tests that operate on a single host.
async fn setup_single_host_sw_fixture() -> (
    sea_orm::DatabaseConnection,
    Arc<AppState>,
    service::Model,
    host::Model,
    software_item::Model,
    host_software_item::Model,
) {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let svc = insert_service(&db, tenant_id).await;
    let host = insert_host(&db, tenant_id).await;
    link_service_host(&db, svc.id, host.id).await;
    let sw = insert_software_item(&db, tenant_id).await;
    let hsi = insert_host_software_item(&db, host.id, sw.id).await;
    (db, state, svc, host, sw, hsi)
}

/// Builds a service with a sentinel hostname and an empty `linked_host_ids`
/// set. Used by the SSH/report-hosts regression tests.
async fn setup_ssh_report_hosts_fixture(
    sentinel: &str,
) -> (
    sea_orm::DatabaseConnection,
    Arc<AppState>,
    service::Model,
    Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
) {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let svc = insert_service_with_hostname(&db, tenant_id, sentinel).await;
    let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::new()));
    (db, state, svc, linked_host_ids)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn handle_renew_certificate_tenant_service_writes_tenant_semantic_audit_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let state = state_with_successful_cert_signer(&base_state);
    insert_ca_certificate(&db).await;
    let svc = insert_service(&db, tenant_id).await;

    let response = handle_renew_certificate(
        &state,
        svc.id,
        &test_cert_identity(),
        &RenewCertificatePayload {
            csr_pem: test_renewal_csr_pem(),
        },
        false,
    )
    .await;
    assert_certificate_reply(&response);

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(svc.id));
    assert_eq!(row.target_type.as_deref(), Some("service"));
    assert_eq!(row.target_id.as_deref(), Some(svc.id.to_string().as_str()));
}

#[tokio::test]
async fn handle_renew_certificate_tenant_service_not_approved_emits_denied_tenant_audit_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let state = state_with_successful_cert_signer(&base_state);
    let svc = insert_service(&db, tenant_id).await;

    service::ActiveModel {
        id: Set(svc.id),
        status: Set(service::ServiceStatus::Pending),
        ..Default::default()
    }
    .update(&db)
    .await
    .expect("downgrade tenant service approval");

    let response = handle_renew_certificate(
        &state,
        svc.id,
        &test_cert_identity(),
        &RenewCertificatePayload {
            csr_pem: test_renewal_csr_pem(),
        },
        false,
    )
    .await;
    assert_error_reply(&response, ErrorCode::Forbidden, "service is not approved");

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row.details_json.expect("service.certificate_renew details");
    assert_eq!(details["reason_code"], serde_json::json!("not_approved"));
}

#[tokio::test]
async fn handle_renew_certificate_tenant_signing_failure_emits_failed_tenant_audit_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let state = state_with_failing_cert_signer(&base_state);
    insert_ca_certificate(&db).await;
    let svc = insert_service(&db, tenant_id).await;

    let response = handle_renew_certificate(
        &state,
        svc.id,
        &test_cert_identity(),
        &RenewCertificatePayload {
            csr_pem: test_renewal_csr_pem(),
        },
        false,
    )
    .await;
    assert_error_reply_contains(
        &response,
        ErrorCode::CertificateError,
        "forced renewal failure",
    );

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Failed.as_str()
    );
    let details = row.details_json.expect("service.certificate_renew details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("certificate_signing_failed")
    );
}

#[tokio::test]
async fn handle_report_plugin_config_emits_success_tenant_audit_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let svc = insert_service(&db, tenant_id).await;

    let payload = report_plugin_config_payload(
        "req-plugin-config-success",
        "generic_shell",
        "Discovered Generic Shell",
        serde_json::json!({
            "version_command": "echo 1.2.3"
        }),
    );

    let response = handle_report_plugin_config(&state, svc.id, &payload).await;
    let reply = assert_report_plugin_config_reply(&response);
    assert!(reply.success);
    let config_id = reply.plugin_config_id.expect("plugin_config_id on success");

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
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
    assert_eq!(row.actor_id, Some(svc.id));
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    assert_eq!(row.target_id, Some(config_id.to_string()));
    let details = row.details_json.expect("plugin_config.create details");
    assert_eq!(details["plugin_type"], serde_json::json!("generic_shell"));
    assert_eq!(
        details["config_name"],
        serde_json::json!("Discovered Generic Shell")
    );
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("service_ws.report_plugin_config")
    );
    assert!(
        !details.to_string().contains("echo 1.2.3"),
        "semantic audit details must not store raw config content"
    );
}

#[tokio::test]
async fn handle_report_plugin_config_emits_validation_failed_tenant_audit_row_for_invalid_config() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let svc = insert_service(&db, tenant_id).await;

    let payload = report_plugin_config_payload(
        "req-plugin-config-invalid",
        "generic_shell",
        "Invalid Generic Shell",
        serde_json::json!({}),
    );

    let response = handle_report_plugin_config(&state, svc.id, &payload).await;
    let reply = assert_report_plugin_config_reply(&response);
    assert!(!reply.success);
    assert_eq!(reply.plugin_config_id, None);

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("plugin_config.create details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("invalid_plugin_config")
    );
}

#[tokio::test]
async fn handle_report_plugin_config_missing_service_emits_denied_system_audit_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

    let payload = report_plugin_config_payload(
        "req-plugin-config-missing-service",
        "generic_shell",
        "Missing Service Config",
        serde_json::json!({
            "version_command": "echo 1.2.3"
        }),
    );

    let response = handle_report_plugin_config(&state, Uuid::now_v7(), &payload).await;
    assert!(response.replies.is_empty());

    let row = wait_for_system_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row.details_json.expect("plugin_config.create details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("service_not_found")
    );
    let tenant_rows = tenant_audit_count_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(tenant_rows, 0);
}

#[tokio::test]
async fn handle_report_plugin_config_db_failure_emits_failed_tenant_audit_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let svc = insert_service(&db, tenant_id).await;

    db.execute_unprepared("DROP TABLE plugin_configs")
        .await
        .expect("drop plugin_configs table");

    let payload = report_plugin_config_payload(
        "req-plugin-config-db-failure",
        "generic_shell",
        "Broken Storage Config",
        serde_json::json!({
            "version_command": "echo 1.2.3"
        }),
    );

    let response = handle_report_plugin_config(&state, svc.id, &payload).await;
    let reply = assert_report_plugin_config_reply(&response);
    assert!(!reply.success);
    assert_eq!(reply.plugin_config_id, None);

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Failed.as_str()
    );
    let details = row.details_json.expect("plugin_config.create details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("create_or_find_failed")
    );
}

#[tokio::test]
async fn handle_renew_certificate_system_service_keeps_writing_system_audit_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let state = state_with_successful_cert_signer(&base_state);
    insert_ca_certificate(&db).await;
    let svc = insert_system_service(&db).await;

    let response = handle_renew_certificate(
        &state,
        svc.id,
        &test_cert_identity(),
        &RenewCertificatePayload {
            csr_pem: test_renewal_csr_pem(),
        },
        true,
    )
    .await;
    assert_certificate_reply(&response);

    let row = wait_for_system_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(svc.id));

    let tenant_rows = tenant_audit_count_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
    )
    .await;
    assert_eq!(tenant_rows, 0);
}

#[tokio::test]
async fn handle_renew_certificate_system_service_not_approved_emits_denied_system_audit_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let state = state_with_successful_cert_signer(&base_state);
    let svc = insert_system_service(&db).await;

    system_service::ActiveModel {
        id: Set(svc.id),
        status: Set(system_service::SystemServiceStatus::Pending),
        ..Default::default()
    }
    .update(&db)
    .await
    .expect("downgrade system service approval");

    let response = handle_renew_certificate(
        &state,
        svc.id,
        &test_cert_identity(),
        &RenewCertificatePayload {
            csr_pem: test_renewal_csr_pem(),
        },
        true,
    )
    .await;
    assert_error_reply(&response, ErrorCode::Forbidden, "service is not approved");

    let row = wait_for_system_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row.details_json.expect("service.certificate_renew details");
    assert_eq!(details["reason_code"], serde_json::json!("not_approved"));
}

#[tokio::test]
async fn handle_renew_certificate_system_signing_failure_emits_failed_system_audit_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let state = state_with_failing_cert_signer(&base_state);
    insert_ca_certificate(&db).await;
    let svc = insert_system_service(&db).await;

    let response = handle_renew_certificate(
        &state,
        svc.id,
        &test_cert_identity(),
        &RenewCertificatePayload {
            csr_pem: test_renewal_csr_pem(),
        },
        true,
    )
    .await;
    assert_error_reply_contains(
        &response,
        ErrorCode::CertificateError,
        "forced renewal failure",
    );

    let row = wait_for_system_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Failed.as_str()
    );
    let details = row.details_json.expect("service.certificate_renew details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("certificate_signing_failed")
    );
}

/// When `host_software_item_id` is set in the result, only the targeted row
/// is updated. The other host's row for the same software item is unchanged.
#[tokio::test]
async fn version_check_results_targeted_update_isolates_correct_row() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

    let svc = insert_service(&db, tenant_id).await;
    let host1 = insert_host(&db, tenant_id).await;
    let host2 = insert_host(&db, tenant_id).await;
    link_service_host(&db, svc.id, host1.id).await;
    link_service_host(&db, svc.id, host2.id).await;

    let sw = insert_software_item(&db, tenant_id).await;
    let hsi1 = insert_host_software_item(&db, host1.id, sw.id).await;
    let hsi2 = insert_host_software_item(&db, host2.id, sw.id).await;

    // Send VersionCheckResults targeting hsi1 only.
    let payload = VersionCheckResultsPayload {
        results: vec![VersionCheckResult {
            software_item_id: sw.id,
            installed_version: Some("2.0.0".to_string()),
            installed_display_version: None,
            latest_version: None,
            error: None,
            update_category: Default::default(),
            host_software_item_id: Some(hsi1.id),
            not_ready: None,
        }],
    };

    handle_version_check_results(&state, svc.id, &payload).await;

    // hsi1 must reflect the new version.
    let updated = host_software_item::Entity::find_by_id(hsi1.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(updated.installed_version, Some("2.0.0".to_string()));

    // hsi2 must be unchanged (no cross-host contamination).
    let unchanged = host_software_item::Entity::find_by_id(hsi2.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(unchanged.installed_version, None);
}

/// When `host_software_item_id` points to a row belonging to a *different*
/// service's host, the update must be rejected (security guard).
#[tokio::test]
async fn version_check_results_targeted_update_rejects_foreign_hsi_id() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

    // Service A owns host_a; service B owns host_b.
    let svc_a = insert_service(&db, tenant_id).await;
    let svc_b = insert_service(&db, tenant_id).await;
    let host_a = insert_host(&db, tenant_id).await;
    let host_b = insert_host(&db, tenant_id).await;
    link_service_host(&db, svc_a.id, host_a.id).await;
    link_service_host(&db, svc_b.id, host_b.id).await;

    let sw = insert_software_item(&db, tenant_id).await;
    let hsi_a = insert_host_software_item(&db, host_a.id, sw.id).await;
    let hsi_b = insert_host_software_item(&db, host_b.id, sw.id).await;

    // Service A sends a result pointing at hsi_b (belongs to service B).
    let payload = VersionCheckResultsPayload {
        results: vec![VersionCheckResult {
            software_item_id: sw.id,
            installed_version: Some("evil".to_string()),
            installed_display_version: None,
            latest_version: None,
            error: None,
            update_category: Default::default(),
            host_software_item_id: Some(hsi_b.id),
            not_ready: None,
        }],
    };

    handle_version_check_results(&state, svc_a.id, &payload).await;

    // hsi_b must not be modified — the host_ids guard filters it out.
    let untouched = host_software_item::Entity::find_by_id(hsi_b.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(untouched.installed_version, None);

    // hsi_a is also untouched (wrong hsi_id was provided).
    let untouched_a = host_software_item::Entity::find_by_id(hsi_a.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(untouched_a.installed_version, None);
}

#[tokio::test]
async fn version_check_results_error_result_preserves_targeted_row_while_success_updates_peer_row()
{
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

    let svc = insert_service(&db, tenant_id).await;
    let host_error = insert_host(&db, tenant_id).await;
    let host_success = insert_host(&db, tenant_id).await;
    link_service_host(&db, svc.id, host_error.id).await;
    link_service_host(&db, svc.id, host_success.id).await;

    let sw = insert_software_item(&db, tenant_id).await;
    let hsi_error = insert_host_software_item(&db, host_error.id, sw.id).await;
    let hsi_success = insert_host_software_item(&db, host_success.id, sw.id).await;

    let preserved_detected_at =
        time::OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("valid unix timestamp");
    let preserved_fetched_at =
        time::OffsetDateTime::from_unix_timestamp(1_700_000_100).expect("valid unix timestamp");
    let success_seed_detected_at =
        time::OffsetDateTime::from_unix_timestamp(1_700_000_200).expect("valid unix timestamp");
    let success_seed_fetched_at =
        time::OffsetDateTime::from_unix_timestamp(1_700_000_300).expect("valid unix timestamp");

    host_software_item::ActiveModel {
        id: Set(hsi_error.id),
        installed_version: Set(Some("1.0.0".to_string())),
        installed_version_detected_at: Set(Some(preserved_detected_at)),
        installed_display_version: Set(Some("1.0.0+baseline".to_string())),
        latest_version: Set(Some("1.0.1".to_string())),
        latest_version_fetched_at: Set(Some(preserved_fetched_at)),
        update_category: Set(UpdateCategory::Bugfix.to_string()),
        ..Default::default()
    }
    .update(&db)
    .await
    .expect("seed error-targeted host_software_item");

    let preserved_baseline = host_software_item::Entity::find_by_id(hsi_error.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");

    host_software_item::ActiveModel {
        id: Set(hsi_success.id),
        installed_version: Set(Some("0.9.0".to_string())),
        installed_version_detected_at: Set(Some(success_seed_detected_at)),
        installed_display_version: Set(Some("0.9.0+seed".to_string())),
        latest_version: Set(Some("0.9.9".to_string())),
        latest_version_fetched_at: Set(Some(success_seed_fetched_at)),
        update_category: Set(UpdateCategory::Unknown.to_string()),
        ..Default::default()
    }
    .update(&db)
    .await
    .expect("seed success-targeted host_software_item");

    let payload = VersionCheckResultsPayload {
        results: vec![
            VersionCheckResult {
                software_item_id: sw.id,
                installed_version: Some("9.9.9-should-not-apply".to_string()),
                installed_display_version: Some("should-not-apply-display".to_string()),
                latest_version: Some("10.0.0-should-not-apply".to_string()),
                error: Some("registry unavailable".to_string()),
                update_category: UpdateCategory::Feature,
                host_software_item_id: Some(hsi_error.id),
                not_ready: None,
            },
            VersionCheckResult {
                software_item_id: sw.id,
                installed_version: Some("2.0.0".to_string()),
                installed_display_version: Some("2.0.0+stable".to_string()),
                latest_version: Some("2.1.0".to_string()),
                error: None,
                update_category: UpdateCategory::Security,
                host_software_item_id: Some(hsi_success.id),
                not_ready: None,
            },
        ],
    };

    handle_version_check_results(&state, svc.id, &payload).await;

    let preserved_after = host_software_item::Entity::find_by_id(hsi_error.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    let success_after = host_software_item::Entity::find_by_id(hsi_success.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");

    assert_eq!(
        preserved_after.installed_version,
        preserved_baseline.installed_version
    );
    assert_eq!(
        preserved_after.installed_display_version,
        preserved_baseline.installed_display_version
    );
    assert_eq!(
        preserved_after.installed_version_detected_at,
        preserved_baseline.installed_version_detected_at
    );
    assert_eq!(
        preserved_after.latest_version,
        preserved_baseline.latest_version
    );
    assert_eq!(
        preserved_after.latest_version_fetched_at,
        preserved_baseline.latest_version_fetched_at
    );
    assert_eq!(
        preserved_after.update_category,
        preserved_baseline.update_category
    );

    assert_eq!(success_after.installed_version, Some("2.0.0".to_string()));
    assert_eq!(
        success_after.installed_display_version,
        Some("2.0.0+stable".to_string())
    );
    assert!(success_after.installed_version_detected_at.is_some());
    assert_ne!(
        success_after.installed_version_detected_at,
        Some(success_seed_detected_at)
    );
    assert_eq!(success_after.latest_version, Some("2.1.0".to_string()));
    assert!(success_after.latest_version_fetched_at.is_some());
    assert_ne!(
        success_after.latest_version_fetched_at,
        Some(success_seed_fetched_at)
    );
    assert_eq!(
        success_after.update_category,
        UpdateCategory::Security.to_string()
    );
}

/// Shared setup for `apply_version_update_to_db` direct-call tests.
///
/// Builds one service with one linked host and one `host_software_item`
/// row with no installed/display versions yet.
async fn setup_apply_version_update_to_db_fixture()
-> (Arc<crate::AppState>, software_item::Model, uuid::Uuid) {
    let (_, state, _, _, sw, hsi) = setup_single_host_sw_fixture().await;
    (state, sw, hsi.id)
}

#[tokio::test]
async fn apply_version_update_to_db_writes_override_into_installed_display_version() {
    let (state, sw, hsi_id) = setup_apply_version_update_to_db_fixture().await;
    let now = time::OffsetDateTime::now_utc();
    let result = VersionCheckResult {
        software_item_id: sw.id,
        installed_version: Some("sha_abc".to_string()),
        installed_display_version: None, // agent sent nothing
        latest_version: None,
        update_category: UpdateCategory::Unknown,
        error: None,
        host_software_item_id: Some(hsi_id),
        not_ready: None,
    };

    apply_version_update_to_db(
        state.db(),
        &result,
        vec![hsi_id],
        now,
        DisplayOverride::Override(Some("2026-06-11T01:15:00Z".to_string())),
    )
    .await;

    let row = host_software_item::Entity::find_by_id(hsi_id)
        .one(state.db())
        .await
        .expect("query")
        .expect("row");
    assert_eq!(row.installed_version.as_deref(), Some("sha_abc"));
    assert_eq!(
        row.installed_display_version.as_deref(),
        Some("2026-06-11T01:15:00Z")
    );
}

#[tokio::test]
async fn apply_version_update_to_db_override_clear_overwrites_prior_display() {
    let (state, sw, hsi_id) = setup_apply_version_update_to_db_fixture().await;

    // Seed a prior display value so we can prove Override(None) clears it.
    host_software_item::ActiveModel {
        id: Set(hsi_id),
        installed_display_version: Set(Some("old_display".to_string())),
        ..Default::default()
    }
    .update(state.db())
    .await
    .expect("seed prior display");

    let now = time::OffsetDateTime::now_utc();
    let result = VersionCheckResult {
        software_item_id: sw.id,
        installed_version: Some("sha_new".to_string()),
        installed_display_version: Some("legacy_agent_supplied".to_string()),
        latest_version: None,
        update_category: UpdateCategory::Unknown,
        error: None,
        host_software_item_id: Some(hsi_id),
        not_ready: None,
    };

    // Enricher ran but returned no display for this SHA → Override(None).
    apply_version_update_to_db(
        state.db(),
        &result,
        vec![hsi_id],
        now,
        DisplayOverride::Override(None),
    )
    .await;

    let row = host_software_item::Entity::find_by_id(hsi_id)
        .one(state.db())
        .await
        .expect("query")
        .expect("row");
    assert_eq!(row.installed_version.as_deref(), Some("sha_new"));
    assert_eq!(
        row.installed_display_version, None,
        "Override(None) must overwrite prior display"
    );
}

#[tokio::test]
async fn apply_version_update_to_db_use_agent_value_preserves_wire_value() {
    let (state, sw, hsi_id) = setup_apply_version_update_to_db_fixture().await;
    let now = time::OffsetDateTime::now_utc();
    let result = VersionCheckResult {
        software_item_id: sw.id,
        installed_version: Some("sha_zzz".to_string()),
        installed_display_version: Some("docker_supplied_date".to_string()),
        latest_version: None,
        update_category: UpdateCategory::Unknown,
        error: None,
        host_software_item_id: Some(hsi_id),
        not_ready: None,
    };

    // No enricher applies → UseAgentValue preserves the wire-supplied display.
    apply_version_update_to_db(
        state.db(),
        &result,
        vec![hsi_id],
        now,
        DisplayOverride::UseAgentValue,
    )
    .await;

    let row = host_software_item::Entity::find_by_id(hsi_id)
        .one(state.db())
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        row.installed_display_version.as_deref(),
        Some("docker_supplied_date")
    );
}

#[tokio::test]
async fn version_check_results_targeted_update_skips_deactivated_host() {
    let (db, state, svc, host, sw, hsi) = setup_single_host_sw_fixture().await;

    host::ActiveModel {
        id: Set(host.id),
        deactivated_at: Set(Some(time::OffsetDateTime::now_utc())),
        ..host.into()
    }
    .update(&db)
    .await
    .expect("deactivate host");

    let payload = VersionCheckResultsPayload {
        results: vec![VersionCheckResult {
            software_item_id: sw.id,
            installed_version: Some("2.0.0".to_string()),
            installed_display_version: None,
            latest_version: None,
            error: None,
            update_category: Default::default(),
            host_software_item_id: Some(hsi.id),
            not_ready: None,
        }],
    };

    handle_version_check_results(&state, svc.id, &payload).await;

    let unchanged = host_software_item::Entity::find_by_id(hsi.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(unchanged.installed_version, None);
}

#[tokio::test]
async fn enrich_discovered_items_defaults_to_enabled_when_type_setting_missing() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(TestPluginOps);
    let (state, _jwt) =
        build_test_state_with_plugin_ops(db.clone(), tenant_id, Some(plugin_ops)).await;
    let svc = insert_service(&db, tenant_id).await;

    let item = insert_named_software_item(&db, tenant_id, "Actual Budget", true).await;

    enrich_discovered_items(&state, &svc).await;

    let updated = software_item::Entity::find_by_id(item.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        updated.icon_url.as_deref(),
        Some("https://cdn.example.test/actual-budget.svg")
    );
}

#[tokio::test]
async fn enrich_discovered_items_respects_explicit_disabled_lifecycle_setting() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(TestPluginOps);
    let (state, _jwt) =
        build_test_state_with_plugin_ops(db.clone(), tenant_id, Some(plugin_ops)).await;
    let svc = insert_service(&db, tenant_id).await;

    crate::queries::plugin_type_settings::upsert_type_settings(
        &db,
        tenant_id,
        plugin_ids::ENHANCEMENT_DASHBOARD_ICONS.as_str(),
        serde_json::json!({ "enabled": false }),
    )
    .await
    .expect("save lifecycle type setting");

    let item = insert_named_software_item(&db, tenant_id, "Actual Budget", true).await;

    enrich_discovered_items(&state, &svc).await;

    let updated = software_item::Entity::find_by_id(item.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(updated.icon_url, None);
}

#[tokio::test]
async fn handle_version_check_results_emits_version_check_completed_audit_summary() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

    let svc = insert_service(&db, tenant_id).await;
    let host_error = insert_host(&db, tenant_id).await;
    let host_success = insert_host(&db, tenant_id).await;
    link_service_host(&db, svc.id, host_error.id).await;
    link_service_host(&db, svc.id, host_success.id).await;

    let sw = insert_software_item(&db, tenant_id).await;
    let hsi_error = insert_host_software_item(&db, host_error.id, sw.id).await;
    let hsi_success = insert_host_software_item(&db, host_success.id, sw.id).await;

    let payload = VersionCheckResultsPayload {
        results: vec![
            VersionCheckResult {
                software_item_id: sw.id,
                installed_version: Some("9.9.9-should-not-apply".to_string()),
                installed_display_version: None,
                latest_version: Some("10.0.0".to_string()),
                error: Some("registry unavailable".to_string()),
                update_category: Default::default(),
                host_software_item_id: Some(hsi_error.id),
                not_ready: None,
            },
            VersionCheckResult {
                software_item_id: sw.id,
                installed_version: Some("2.0.0".to_string()),
                installed_display_version: None,
                latest_version: Some("2.1.0".to_string()),
                error: None,
                update_category: Default::default(),
                host_software_item_id: Some(hsi_success.id),
                not_ready: None,
            },
        ],
    };

    handle_version_check_results(&state, svc.id, &payload).await;

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_COMPLETED,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Partial.as_str()
    );
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(svc.id));
    assert_eq!(row.target_type.as_deref(), Some("service"));
    assert_eq!(row.target_id.as_deref(), Some(svc.id.to_string().as_str()));
    let details = row
        .details_json
        .expect("software.version_check.completed details");
    assert_eq!(details["result_count"], serde_json::json!(2));
    assert_eq!(details["success_count"], serde_json::json!(1));
    assert_eq!(details["error_count"], serde_json::json!(1));
    assert_eq!(details["rows_mutated"], serde_json::json!(1));
}

#[tokio::test]
async fn enrich_discovered_items_emits_software_item_enrich_audit_summary() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(TestPluginOps);
    let (state, _jwt) =
        build_test_state_with_plugin_ops(db.clone(), tenant_id, Some(plugin_ops)).await;

    let svc = insert_service(&db, tenant_id).await;
    let item = insert_named_software_item(&db, tenant_id, "Actual Budget", true).await;

    enrich_discovered_items(&state, &svc).await;

    let updated = software_item::Entity::find_by_id(item.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        updated.icon_url.as_deref(),
        Some("https://cdn.example.test/actual-budget.svg")
    );

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_ENRICH,
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
    assert_eq!(row.actor_id, Some(svc.id));
    assert_eq!(row.target_type.as_deref(), Some("service"));
    assert_eq!(row.target_id.as_deref(), Some(svc.id.to_string().as_str()));
    let details = row.details_json.expect("software_item.enrich details");
    assert_eq!(details["patched_count"], serde_json::json!(1));
    assert_eq!(details["patch_failed_count"], serde_json::json!(0));
    assert_eq!(details["examined_count"], serde_json::json!(1));
}

#[tokio::test]
async fn handle_report_hosts_embedded_service_does_not_report_machine_id_to_notifier() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let notifier = Arc::new(TestEmbeddedNotifier::default());
    let state = Arc::new(AppState {
        embedded_service_notifier: Some(notifier.clone()),
        ..(*state).clone()
    });

    let service_id = Uuid::now_v7();
    insert_embedded_service(&db, tenant_id, service_id).await;

    let payload = ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "embedded-machine".to_string(),
            os_type: Some("macos".to_string()),
            os_version: Some("macOS 26.2".to_string()),
            architecture: Some("aarch64".to_string()),
            hostname: Some("MacBook-Pro---Andrey.local".to_string()),
            ip_address: None,
            agent_host_id: None,
            features: None,
        }],
        agent_version: "0.0.1".to_string(),
        capabilities: [Capability::SoftwareDiscovery, Capability::UpdateHooks]
            .into_iter()
            .collect(),
    };
    let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::new()));

    handle_report_hosts(&state, service_id, &payload, &linked_host_ids).await;

    assert!(notifier.machine_ids.lock().is_empty());
}

#[tokio::test]
async fn handle_report_hosts_emits_host_update_audit_summary_on_success() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let svc = insert_service(&db, tenant_id).await;

    let payload = ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "machine-success".to_string(),
            os_type: Some("linux".to_string()),
            os_version: Some("6.8".to_string()),
            architecture: Some("x86_64".to_string()),
            hostname: Some("host-success".to_string()),
            ip_address: Some("192.0.2.10".to_string()),
            agent_host_id: None,
            features: None,
        }],
        agent_version: "1.2.3".to_string(),
        capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
    };
    let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::new()));
    handle_report_hosts(&state, svc.id, &payload, &linked_host_ids).await;

    let row =
        wait_for_tenant_audit_row_for_action(&db, uptrakit_audit_log::AuditActionType::HOST_UPDATE)
            .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(svc.id));
    assert_eq!(row.target_type.as_deref(), Some("service"));
    assert_eq!(row.target_id.as_deref(), Some(svc.id.to_string().as_str()));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );

    let details = row.details_json.expect("host.update details");
    assert_eq!(details["created_hosts"].as_u64(), Some(1));
    assert_eq!(details["updated_hosts"].as_u64(), Some(0));
    assert_eq!(details["failed_hosts"].as_u64(), Some(0));
}

#[tokio::test]
async fn handle_report_hosts_emits_host_update_audit_summary_partial_when_some_hosts_fail() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let svc = insert_service(&db, tenant_id).await;

    let duplicate_host_id = Uuid::now_v7();
    let payload = ReportHostsPayload {
        hosts: vec![
            HostInfo {
                machine_id: "machine-partial-a".to_string(),
                os_type: Some("linux".to_string()),
                os_version: Some("6.8".to_string()),
                architecture: Some("x86_64".to_string()),
                hostname: Some("host-partial-a".to_string()),
                ip_address: Some("192.0.2.20".to_string()),
                agent_host_id: Some(duplicate_host_id),
                features: None,
            },
            HostInfo {
                machine_id: "machine-partial-b".to_string(),
                os_type: Some("linux".to_string()),
                os_version: Some("6.8".to_string()),
                architecture: Some("x86_64".to_string()),
                hostname: Some("host-partial-b".to_string()),
                ip_address: Some("192.0.2.21".to_string()),
                agent_host_id: Some(duplicate_host_id),
                features: None,
            },
        ],
        agent_version: "1.2.3".to_string(),
        capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
    };
    let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::new()));
    handle_report_hosts(&state, svc.id, &payload, &linked_host_ids).await;

    let row =
        wait_for_tenant_audit_row_for_action(&db, uptrakit_audit_log::AuditActionType::HOST_UPDATE)
            .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(svc.id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Partial.as_str()
    );

    let details = row.details_json.expect("host.update details");
    assert_eq!(details["created_hosts"].as_u64(), Some(1));
    assert_eq!(details["failed_hosts"].as_u64(), Some(1));
}

/// Insert a service with a recognisable sentinel `hostname` so that any
/// leak of `service.hostname` into a downstream `host` row is detectable.
#[expect(clippy::string_slice, reason = "UUID hex is always ASCII")]
async fn insert_service_with_hostname(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    hostname: &str,
) -> service::Model {
    let id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    service::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set(hostname.to_string()),
        friendly_name: Set(format!("Service {}", &id.to_string()[..8])),
        ip_address: Set(None),
        status: Set(service::ServiceStatus::Approved),
        enrollment_secret_hash: Set(format!("secret-{id}")),
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
    .expect("insert service with sentinel hostname")
}

/// Regression: when `host_info.ip_address` is present (SSH agent path),
/// both `hostname` and `friendly_name` on the new host row come from
/// the SSH target. The remote-reported hostname does NOT override the
/// operator-typed SSH target. Never from `service.hostname` either.
#[tokio::test]
async fn report_hosts_uses_ssh_target_over_remote_hostname() {
    const SERVICE_SENTINEL: &str = "controller-host-sentinel";
    let (db, state, svc, linked_host_ids) = setup_ssh_report_hosts_fixture(SERVICE_SENTINEL).await;

    // SSH agent reports both: ip_address = user-typed SSH target host
    // (bare hostname, parsed out of `user@host:port` by SshTarget at
    // bootstrap time — see `agent-ssh-runtime/src/ssh_target.rs`),
    // hostname = whatever the remote calls itself. SSH target wins.
    let payload = ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "machine-remote".to_string(),
            os_type: Some("linux".to_string()),
            os_version: None,
            architecture: None,
            hostname: Some("remote-self-name".to_string()),
            ip_address: Some("mikrotik.uk-home.example".to_string()),
            agent_host_id: None,
            features: None,
        }],
        agent_version: "1.0.0".to_string(),
        capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
    };
    handle_report_hosts(&state, svc.id, &payload, &linked_host_ids).await;

    let row = host::Entity::find()
        .filter(host::Column::MachineId.eq("machine-remote"))
        .one(&db)
        .await
        .expect("query")
        .expect("host row created");
    assert_eq!(row.hostname, "mikrotik.uk-home.example");
    assert_eq!(row.friendly_name, "mikrotik.uk-home.example");
    assert_ne!(row.hostname, SERVICE_SENTINEL);
}

/// Standalone-agent path: no SSH target, so the agent-reported
/// `hostname` is used.
#[tokio::test]
async fn report_hosts_uses_hostname_when_no_ssh_target() {
    const SERVICE_SENTINEL: &str = "controller-host-sentinel";
    let (db, state, svc, linked_host_ids) = setup_ssh_report_hosts_fixture(SERVICE_SENTINEL).await;

    let payload = ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "machine-standalone".to_string(),
            os_type: Some("linux".to_string()),
            os_version: None,
            architecture: None,
            hostname: Some("standalone.example".to_string()),
            ip_address: None,
            agent_host_id: None,
            features: None,
        }],
        agent_version: "1.0.0".to_string(),
        capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
    };
    handle_report_hosts(&state, svc.id, &payload, &linked_host_ids).await;

    let row = host::Entity::find()
        .filter(host::Column::MachineId.eq("machine-standalone"))
        .one(&db)
        .await
        .expect("query")
        .expect("host row created");
    assert_eq!(row.hostname, "standalone.example");
    assert_eq!(row.friendly_name, "standalone.example");
    assert_ne!(row.hostname, SERVICE_SENTINEL);
}

/// Regression for the reported bug: when the agent reports
/// `hostname: None` (e.g. RouterOS before the identity fix), the
/// controller falls back to `host_info.ip_address` — the user-typed
/// SSH target — and never to `service.hostname` (the controller's own
/// hostname for an embedded SSH agent).
#[tokio::test]
async fn report_hosts_falls_back_to_ip_address_not_service_hostname() {
    const SERVICE_SENTINEL: &str = "MacBook-Pro---Andrey.local";
    let (db, state, svc, linked_host_ids) = setup_ssh_report_hosts_fixture(SERVICE_SENTINEL).await;

    let payload = ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "machine-routeros".to_string(),
            os_type: Some("routeros".to_string()),
            os_version: None,
            architecture: None,
            hostname: None,
            ip_address: Some("mikrotik.uk-home.example".to_string()),
            agent_host_id: None,
            features: None,
        }],
        agent_version: "1.0.0".to_string(),
        capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
    };
    handle_report_hosts(&state, svc.id, &payload, &linked_host_ids).await;

    let row = host::Entity::find()
        .filter(host::Column::MachineId.eq("machine-routeros"))
        .one(&db)
        .await
        .expect("query")
        .expect("host row created");
    assert_eq!(row.hostname, "mikrotik.uk-home.example");
    assert_eq!(row.friendly_name, "mikrotik.uk-home.example");
    assert_ne!(
        row.hostname, SERVICE_SENTINEL,
        "must not leak controller's hostname into remote host record"
    );
}

/// Regression: when neither `hostname` nor `ip_address` is supplied, the
/// host is skipped (counted as failed) rather than getting the service
/// hostname or a synthetic name. This is unreachable for real agents.
#[tokio::test]
async fn report_hosts_skips_when_neither_hostname_nor_ip_provided() {
    const SERVICE_SENTINEL: &str = "controller-sentinel";
    let (db, state, svc, linked_host_ids) = setup_ssh_report_hosts_fixture(SERVICE_SENTINEL).await;

    let payload = ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "machine-no-name".to_string(),
            os_type: None,
            os_version: None,
            architecture: None,
            hostname: None,
            ip_address: None,
            agent_host_id: None,
            features: None,
        }],
        agent_version: "1.0.0".to_string(),
        capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
    };
    handle_report_hosts(&state, svc.id, &payload, &linked_host_ids).await;

    let row = host::Entity::find()
        .filter(host::Column::MachineId.eq("machine-no-name"))
        .one(&db)
        .await
        .expect("query");
    assert!(
        row.is_none(),
        "host with neither hostname nor ip_address must not be persisted"
    );
}

/// Fast-path stability: re-reporting a known SSH host on the fast path
/// (where `host_info.hostname` is None and only `ip_address` is carried)
/// resolves to the same name as the slow path, so the DB row's
/// `hostname` does not oscillate across reload ticks.
#[tokio::test]
async fn report_hosts_does_not_oscillate_hostname_on_fast_path_reload() {
    // Bare hostname — SshTarget strips `user@` and `:port` at bootstrap
    // time before the SSH agent stores it locally and reports it as
    // `ip_address`.
    const SSH_TARGET: &str = "mikrotik.uk-home.example";
    let (db, state, svc, linked_host_ids) =
        setup_ssh_report_hosts_fixture("controller-sentinel").await;

    // Slow path (initial bootstrap report): SSH agent set both fields.
    let initial = ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "machine-stable".to_string(),
            os_type: Some("routeros".to_string()),
            os_version: None,
            architecture: None,
            hostname: Some("remote-self-name".to_string()),
            ip_address: Some(SSH_TARGET.to_string()),
            agent_host_id: None,
            features: None,
        }],
        agent_version: "1.0.0".to_string(),
        capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
    };
    handle_report_hosts(&state, svc.id, &initial, &linked_host_ids).await;

    let row = host::Entity::find()
        .filter(host::Column::MachineId.eq("machine-stable"))
        .one(&db)
        .await
        .expect("query")
        .expect("host row created");
    assert_eq!(row.hostname, SSH_TARGET);
    let host_id = row.id;

    // Fast-path reload: hostname = None, only ip_address is carried.
    let reload = ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "machine-stable".to_string(),
            os_type: None,
            os_version: None,
            architecture: None,
            hostname: None,
            ip_address: Some(SSH_TARGET.to_string()),
            agent_host_id: None,
            features: None,
        }],
        agent_version: "1.0.0".to_string(),
        capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
    };
    handle_report_hosts(&state, svc.id, &reload, &linked_host_ids).await;

    let row_after = host::Entity::find_by_id(host_id)
        .one(&db)
        .await
        .expect("query")
        .expect("host row still exists");
    assert_eq!(
        row_after.hostname, SSH_TARGET,
        "fast-path reload must resolve to the same SSH target as the \
         initial slow-path report — no oscillation"
    );
}

#[tokio::test]
async fn handle_discovery_results_emits_host_discover_audit_summary_on_success() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let svc = insert_service(&db, tenant_id).await;
    let host = insert_host(&db, tenant_id).await;
    link_service_host(&db, svc.id, host.id).await;
    let plugin_config = insert_plugin_config(
        &db,
        tenant_id,
        uptrakit_shared_types::plugin_ids::PACKAGE_MANAGER_HOMEBREW.as_str(),
    )
    .await;
    let mut report_tracker = ReportTracker::new();

    let payload = DiscoveryResultsPayload {
        host_machine_id: host.machine_id.clone(),
        results: vec![DiscoveryPluginResult {
            plugin_config_id: Some(plugin_config.id),
            plugin_type: uptrakit_shared_types::plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
            discoveries: vec![DiscoveredSoftware {
                package_identifier: "wget".to_string(),
                name: "Wget".to_string(),
                installed_version: "1.0.0".to_string(),
                targets: vec![],
                extra: None,
                qualifier: None,
                plugin_package_identifier: None,
                featured: false,
                installed_display_version: None,
            }],
            error: None,
        }],
    };

    handle_discovery_results(&state, svc.id, payload, None, &mut report_tracker).await;

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(svc.id));
    assert_eq!(row.target_type.as_deref(), Some("host"));
    assert_eq!(row.target_id.as_deref(), Some(host.id.to_string().as_str()));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    let details = row.details_json.expect("host.discover details");
    assert_eq!(details["plugin_results"].as_u64(), Some(1));
}

#[tokio::test]
async fn handle_discovery_results_emits_host_discover_audit_summary_for_unknown_machine_id() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
    let svc = insert_service(&db, tenant_id).await;
    let mut report_tracker = ReportTracker::new();

    let payload = DiscoveryResultsPayload {
        host_machine_id: "missing-machine".to_string(),
        results: vec![],
    };

    handle_discovery_results(&state, svc.id, payload, None, &mut report_tracker).await;

    let row = wait_for_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
    )
    .await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::Service.as_str()
    );
    assert_eq!(row.actor_id, Some(svc.id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );

    let details = row.details_json.expect("host.discover details");
    assert_eq!(
        details["reason_code"].as_str(),
        Some("unknown_host_machine_id")
    );
}

// ── Enricher dispatch tests ───────────────────────────────────────────
//
// The 4 tests below exercise `build_enriched_display_overrides` (invoked
// from `handle_version_check_results`) against the test-only descriptors
// registered by `uptrakit_plugin_infrastructure_registry::test_support`:
//
//   * `__test_enricher_echo` — capable; returns
//     `display_version = Some(format!("date_for_{sha}"))`.
//   * `__test_enricher_miss` — capable; always returns
//     `display_version = None`.
//   * `__test_fetch_fail`   — NOT capable (no `EnrichInstalledVersion`).
//
// Per-test fixtures seed a `host_software_item_plugin` row with role
// `detect_version` so `plugin_types_for_role` resolves the plugin type.

use uptrakit_shared_db::entity::host_software_item_plugin;

/// Insert a `host_software_item_plugin` row with role `detect_version`
/// linking the given `host_software_item` to a plugin type.
async fn insert_detect_version_plugin_assignment(
    db: &sea_orm::DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
    host_software_item_id: Uuid,
    plugin_type: &str,
    package_identifier: &str,
) {
    let now = time::OffsetDateTime::now_utc();
    host_software_item_plugin::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(host_software_item_id),
        plugin_config_id: Set(None),
        plugin_type: Set(plugin_type.to_string()),
        role: Set("detect_version".to_string()),
        ordinal: Set(0),
        package_identifier: Set(package_identifier.to_string()),
        config: Set(None),
        execution_site: Set("auto".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert host_software_item_plugin");
}

#[tokio::test]
async fn handle_version_check_results_invokes_enricher_for_capable_plugin() {
    let (db, state, svc, host, sw, hsi) = setup_single_host_sw_fixture().await;
    insert_detect_version_plugin_assignment(
        &db,
        host.id,
        sw.id,
        hsi.id,
        "__test_enricher_echo",
        "pkg/foo",
    )
    .await;

    let payload = VersionCheckResultsPayload {
        results: vec![VersionCheckResult {
            software_item_id: sw.id,
            installed_version: Some("abc123".to_string()),
            installed_display_version: None,
            latest_version: None,
            error: None,
            update_category: UpdateCategory::Unknown,
            host_software_item_id: Some(hsi.id),
            not_ready: None,
        }],
    };

    handle_version_check_results(&state, svc.id, &payload).await;

    let row = host_software_item::Entity::find_by_id(hsi.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        row.installed_display_version.as_deref(),
        Some("date_for_abc123"),
        "enricher output must flow into the write"
    );
}

#[tokio::test]
async fn handle_version_check_results_does_not_invoke_enricher_without_capability() {
    let (db, state, svc, host, sw, hsi) = setup_single_host_sw_fixture().await;
    // `__test_fetch_fail` declares ReleaseFetching but NOT
    // EnrichInstalledVersion — dispatcher must skip the enricher path.
    insert_detect_version_plugin_assignment(
        &db,
        host.id,
        sw.id,
        hsi.id,
        "__test_fetch_fail",
        "pkg/foo",
    )
    .await;

    let payload = VersionCheckResultsPayload {
        results: vec![VersionCheckResult {
            software_item_id: sw.id,
            installed_version: Some("xyz".to_string()),
            installed_display_version: None,
            latest_version: None,
            error: None,
            update_category: UpdateCategory::Unknown,
            host_software_item_id: Some(hsi.id),
            not_ready: None,
        }],
    };

    handle_version_check_results(&state, svc.id, &payload).await;

    let row = host_software_item::Entity::find_by_id(hsi.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert!(
        row.installed_display_version.is_none(),
        "no enricher capability → no override"
    );
}

#[tokio::test]
async fn handle_version_check_results_writes_none_when_enricher_misses() {
    let (db, state, svc, host, sw, hsi) = setup_single_host_sw_fixture().await;
    // Seed a prior display value so the test proves Override(None) clears it.
    host_software_item::ActiveModel {
        id: Set(hsi.id),
        installed_display_version: Set(Some("old".to_string())),
        ..Default::default()
    }
    .update(&db)
    .await
    .expect("seed prior display");

    insert_detect_version_plugin_assignment(
        &db,
        host.id,
        sw.id,
        hsi.id,
        "__test_enricher_miss",
        "pkg/foo",
    )
    .await;

    let payload = VersionCheckResultsPayload {
        results: vec![VersionCheckResult {
            software_item_id: sw.id,
            installed_version: Some("new".to_string()),
            installed_display_version: None,
            latest_version: None,
            error: None,
            update_category: UpdateCategory::Unknown,
            host_software_item_id: Some(hsi.id),
            not_ready: None,
        }],
    };

    handle_version_check_results(&state, svc.id, &payload).await;

    let row = host_software_item::Entity::find_by_id(hsi.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert!(
        row.installed_display_version.is_none(),
        "miss must overwrite stale display with None"
    );
}

#[tokio::test]
async fn handle_version_check_results_keeps_distinct_display_per_host_for_same_skill() {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

    let svc = insert_service(&db, tenant_id).await;
    let host_a = insert_host(&db, tenant_id).await;
    let host_b = insert_host(&db, tenant_id).await;
    link_service_host(&db, svc.id, host_a.id).await;
    link_service_host(&db, svc.id, host_b.id).await;

    let sw = insert_software_item(&db, tenant_id).await;
    let hsi_a = insert_host_software_item(&db, host_a.id, sw.id).await;
    let hsi_b = insert_host_software_item(&db, host_b.id, sw.id).await;
    // Same package_identifier — only the per-result SHA differs.
    insert_detect_version_plugin_assignment(
        &db,
        host_a.id,
        sw.id,
        hsi_a.id,
        "__test_enricher_echo",
        "pkg/shared",
    )
    .await;
    insert_detect_version_plugin_assignment(
        &db,
        host_b.id,
        sw.id,
        hsi_b.id,
        "__test_enricher_echo",
        "pkg/shared",
    )
    .await;

    let payload = VersionCheckResultsPayload {
        results: vec![
            VersionCheckResult {
                software_item_id: sw.id,
                installed_version: Some("sha_a".to_string()),
                installed_display_version: None,
                latest_version: None,
                error: None,
                update_category: UpdateCategory::Unknown,
                host_software_item_id: Some(hsi_a.id),
                not_ready: None,
            },
            VersionCheckResult {
                software_item_id: sw.id,
                installed_version: Some("sha_b".to_string()),
                installed_display_version: None,
                latest_version: None,
                error: None,
                update_category: UpdateCategory::Unknown,
                host_software_item_id: Some(hsi_b.id),
                not_ready: None,
            },
        ],
    };

    handle_version_check_results(&state, svc.id, &payload).await;

    let row_a = host_software_item::Entity::find_by_id(hsi_a.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    let row_b = host_software_item::Entity::find_by_id(hsi_b.id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        row_a.installed_display_version.as_deref(),
        Some("date_for_sha_a"),
        "host_a must see its own SHA-derived display"
    );
    assert_eq!(
        row_b.installed_display_version.as_deref(),
        Some("date_for_sha_b"),
        "host_b must see its own SHA-derived display"
    );
}
