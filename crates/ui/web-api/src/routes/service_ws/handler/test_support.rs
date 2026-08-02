//! Shared test fixtures for WebSocket handler tests.
//!
//! Each public item is `pub(super)` so it is visible within the `handler`
//! module and to all sibling submodules declared there.

// `let _ = must_use_future.await` in register_test_connection and
// run_embedded_register_once — only compiled when db-sqlite is active.
#![cfg_attr(
    feature = "db-sqlite",
    expect(
        clippy::let_underscore_must_use,
        reason = "fire-and-forget sends in WS handler intentionally drop results"
    )
)]

use std::sync::Arc;

use rootcause::prelude::*;
use uptrakit_wire::surfaces;
use uuid::Uuid;

use crate::AppState;

// Private types from handler accessible to child modules.
#[cfg(feature = "db-sqlite")]
use super::session_authenticated::AuthenticatedSessionState;

// pub(crate) constants / types from service_ws::protocol.
#[cfg(feature = "db-sqlite")]
use super::super::protocol::{MessageRateLimiter, WS_MESSAGE_RATE_LIMIT, WS_MESSAGE_RATE_WINDOW};

// pub(crate) handler function used by run_embedded_register_once.
#[cfg(feature = "db-sqlite")]
use super::run_embedded_message_handler;

// Items only compiled when the db-sqlite feature is active.
#[cfg(feature = "db-sqlite")]
use crate::embedded_support::EmbeddedServiceNotifier;
#[cfg(feature = "db-sqlite")]
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
#[cfg(feature = "db-sqlite")]
use std::collections::{BTreeSet, HashSet};
#[cfg(feature = "db-sqlite")]
use time::OffsetDateTime;
#[cfg(feature = "db-sqlite")]
use tokio_util::sync::CancellationToken;
#[cfg(feature = "db-sqlite")]
use uptrakit_shared_db::entity::{host, service, service_host, software_item, update_history};
#[cfg(feature = "db-sqlite")]
use uptrakit_shared_types::ServiceStatus;
#[cfg(feature = "db-sqlite")]
use uptrakit_wire::{
    Capability, DisconnectReason, DisconnectingPayload, RegisterPayload, ServiceMessage,
};

// ---------------------------------------------------------------------------
// Fixtures (no db required)
// ---------------------------------------------------------------------------

pub(super) fn test_surface_registration(
    provider_id: &str,
    tenant_id: Uuid,
) -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: provider_id.to_string(),
            provider_kind: surfaces::ProviderKind::Service,
            provider_namespace: "service".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::TargetedTargeting,
            surfaces::Capability::ProviderInitiatedActions,
            surfaces::Capability::MutationAction,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Tenant,
            tenant_id: Some(tenant_id.to_string()),
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                .label("SSH Guest Panel")
                .priority(100)
                .slot(surfaces::SLOT_SOFTWARE_TABS)
                .scope(surfaces::Scope::Tenant)
                .targeting(surfaces::Targeting::Targeted)
                .required_permission("view_software")
                .provider_kind(surfaces::ProviderKind::Service)
                .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::TargetedTargeting,
                    surfaces::Capability::MutationAction,
                ]))
                .root_node(surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                })
                .build(),
            interactions: vec![{
                let mut i = surfaces::InteractionDescriptor::new(
                    surfaces::InteractionId::new("refresh").unwrap(),
                    surfaces::InteractionKind::MutationAction,
                    "Refresh",
                    surfaces::InteractionTransport::ProviderProxied,
                );
                i.required_permission = Some("update_software".to_string());
                i.input_schema = Some(surfaces::SchemaContract::Object);
                i.result_schema = Some(surfaces::SchemaContract::Object);
                i.timeout_seconds = Some(30);
                i
            }],
            data_sources: Vec::new(),
        }],
        encryption_metadata: None,
    }
}

pub(super) struct NoopCertSigner;

#[async_trait::async_trait]
impl crate::cert_signer::AgentCertSigner for NoopCertSigner {
    async fn sign_agent_csr(
        &self,
        _: &str,
        _: &uuid::Uuid,
        _: time::Duration,
    ) -> std::result::Result<
        crate::cert_signer::SignedCertBundle,
        Report<crate::cert_signer::CertSignerError>,
    > {
        Err(report!(crate::cert_signer::CertSignerError::Signing(
            "noop signer".to_string(),
        )))
    }

    fn active_ca_fingerprint(&self) -> String {
        "0000000000000000000000000000000000000000000000000000000000000000".to_string()
    }
}

pub(super) async fn build_handler_test_state(
    surface_registry: Arc<crate::surface_registry::SurfaceRegistry>,
    surface_proxy: Arc<crate::surface_proxy::SurfaceProxy>,
) -> Arc<AppState> {
    let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
    let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
        active_cert_pem: ca_pem.to_string(),
        active_fingerprint: "0".repeat(64),
        previous_cert_pem: None,
        previous_fingerprint: None,
        trusted_cas: vec![crate::ca_snapshot::TrustedCaPublic {
            cert_pem: ca_pem.to_string(),
            fingerprint: "0".repeat(64),
            not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
        }],
        trusted_ca_cns: Vec::new(),
        bundle_pem: ca_pem.to_string(),
        bundle_hash: "0".repeat(64),
        managed: true,
        active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
        pki_addr: None,
    };
    let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
    let ca_key_store: crate::CaKeyStoreRef =
        Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
            active_key_pem: zeroize::Zeroizing::new(String::new()),
            previous_key_pem: None,
            trusted_ca_keys: vec![],
        }));

    let rustls_cfg = {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)
            .expect("test key generation should succeed");
        let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
            .expect("test cert params should be valid")
            .self_signed(&key_pair)
            .expect("test certificate should self-sign");
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der())
                    .expect("test private key should parse"),
            )
            .expect("test rustls config should build");
        axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
    };

    let db = sea_orm::Database::connect(sea_orm::ConnectOptions::new("sqlite::memory:"))
        .await
        .expect("test db should connect");
    let settings = crate::settings::Settings::new(
        crate::auth::registration::RegistrationSettings {
            mode: crate::auth::registration::RegistrationMode::Open,
            token_hash: None,
            require_token_for_oidc: false,
        },
        168,
    );
    let service_connections = crate::service_connections::ServiceConnectionRegistry::new();
    let controller_id = Uuid::nil();
    let notification_service = crate::notification_service::NotificationService::new(
        service_connections.clone(),
        controller_id,
    );
    let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
        )
        .expect("catalog should build in tests"),
    );
    let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
        db.clone(),
        Arc::clone(&plugin_ops),
        "https://localhost".to_string(),
    );

    let (_, config_rx_for_ws_handler) = uptrakit_config_reload::RuntimeConfigChannels::from_runtime(
        &uptrakit_config_reload::RuntimeConfig::default(),
    );

    Arc::new(AppState {
        db: crate::app_state::DbState::new(db.clone()),
        access_engine: Arc::new(uptrakit_controller_core::access::AccessEngine::new(
            db.clone(),
        )),
        cert: crate::app_state::CertState {
            ca_snapshot: ca_rx,
            ca_key_store,
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            crl_pem_cache: Arc::new(parking_lot::RwLock::new(String::new())),
            ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
        },
        auth: crate::app_state::AuthState::new(
            Arc::new(crate::auth::jwt::JwtManager::from_secret(
                b"test-secret-handler",
            )),
            crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
            crate::auth::rate_limit::RateLimitStore::new(db.clone()),
            Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
        ),
        notification: crate::app_state::NotificationState::new(
            notification_service,
            notification_dispatcher,
            crate::event_broadcaster::EventBroadcaster::new(),
        ),
        broadcast: crate::app_state::BroadcastState {
            update_output_broadcaster:
                crate::update_output_broadcaster::UpdateOutputBroadcaster::new(),
            batch_progress_broadcaster:
                crate::batch_progress_broadcaster::BatchProgressBroadcaster::new(),
        },
        #[cfg(feature = "oidc")]
        oidc: crate::app_state::OidcState {
            oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
            account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
            oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                db.clone(),
            ),
            oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                db.clone(),
            ),
        },
        settings,
        cert_signer: Arc::new(NoopCertSigner),
        service_connections,
        plugin: crate::app_state::PluginState::new(
            plugin_ops,
            Arc::new(crate::global_providers::GlobalProviders::new(db.clone())),
        ),
        credential_sources: crate::ServiceCredentialSources::default(),
        shutdown_token: Default::default(),
        embedded_service_notifier: None,
        audit_log_filter_rx: tokio::sync::watch::channel(std::sync::Arc::new(
            uptrakit_config_reload::config::AuditConfig::default(),
        ))
        .1,
        audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
            uptrakit_audit_log::NoopBackend,
        )),
        audit_emitter: uptrakit_audit_log::AuditEmitter::new(
            uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(uptrakit_audit_log::NoopBackend)),
        ),
        surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
            surface_registry,
            surface_proxy,
            Arc::new(crate::surface_proxy::AllProvidersVisible),
        ),
        config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
        workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
        server: crate::app_state::ServerState::new(
            std::path::PathBuf::from("/tmp/test-pki"),
            rustls_cfg,
        ),
        default_tenant_id: Uuid::nil(),
        controller_id,
        reject_dangerous_commands: false,
        #[cfg(feature = "interactive")]
        interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
        #[cfg(feature = "test-utils")]
        test_reexec_notify: None,
        update_dispatcher: Arc::new(uptrakit_controller_core::update::NoopUpdateDispatcher),
        instance_plugin_snapshot: Arc::new(arc_swap::ArcSwap::from_pointee(
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot::empty(),
        )),
        coordinator_handle: {
            let (tx, _) = tokio::sync::mpsc::unbounded_channel();
            uptrakit_config_reload::ReloadCoordinator::new(
                vec![],
                tx,
                std::sync::Arc::new(uptrakit_config_reload::NoopAlertWriter),
            )
            .1
        },
        settings_version_cache: uptrakit_config_reload::SettingsVersionCache::new(),
        db_config_rx: config_rx_for_ws_handler.db,
        network_config_rx: config_rx_for_ws_handler.network,
        nats_config_rx: config_rx_for_ws_handler.nats,
        tls_config_rx: config_rx_for_ws_handler.tls,
        audit_config_rx: config_rx_for_ws_handler.audit,
        log_config_rx: config_rx_for_ws_handler.log,
        master_key_config_rx: config_rx_for_ws_handler.master_key,
        embedded_services_config_rx: config_rx_for_ws_handler.embedded_services,
        zeroconf_config_rx: config_rx_for_ws_handler.zeroconf,
        oauth: crate::oauth::OAuthState::disabled(),
        config_file_state: tokio::sync::watch::channel(
            uptrakit_config_reload::ConfigFileState::default(),
        )
        .1,
        last_reload: tokio::sync::watch::channel(None).1,
        recent_reload_events: tokio::sync::watch::channel(Vec::new()).1,
    })
}

// ---------------------------------------------------------------------------
// Fixtures (db-sqlite required)
// ---------------------------------------------------------------------------

#[cfg(feature = "db-sqlite")]
pub(super) fn test_authenticated_session(
    service_id: Uuid,
    connection_id: Uuid,
) -> AuthenticatedSessionState {
    let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
    let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
    let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
    AuthenticatedSessionState {
        service_id,
        connection_id,
        is_system: false,
        has_update_tracking: false,
        has_software_discovery: false,
        has_workload_claims: false,
        service_tenant_id: None,
        linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
        push_rx,
        cancel_token: tokio_util::sync::CancellationToken::new(),
        msg_tx,
        resp_rx,
        processor_cancel: tokio_util::sync::CancellationToken::new(),
        processor_handle: tokio::spawn(async {}),
        rate_limiter: MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT),
    }
}

#[cfg(feature = "db-sqlite")]
pub(super) fn register_test_runtime_state(
    state: &Arc<AppState>,
    service_id: Uuid,
    tenant_id: Uuid,
) {
    state
        .surface_proxy_deps
        .registry
        .register_service(
            service_id,
            "uptrakit-agent-ssh",
            Some(tenant_id),
            test_surface_registration("service.provider-a", tenant_id),
        )
        .expect("surface registration should succeed");
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn register_test_connection(state: &Arc<AppState>, service_id: Uuid) -> Uuid {
    let capabilities = BTreeSet::from([Capability::UiSurfaces]);
    let (_push_rx, handle) = state
        .service_connections
        .register(
            service_id,
            capabilities,
            None,
            None,
            Some("uptrakit-agent-ssh".to_string()),
        )
        .await;
    handle.connection_id()
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn insert_test_service_row(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    service_id: Uuid,
    service_app_name: &str,
) {
    let now = time::OffsetDateTime::now_utc();
    uptrakit_shared_db::entity::service::ActiveModel {
        id: Set(service_id),
        tenant_id: Set(tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set(format!("svc-{service_id}")),
        friendly_name: Set(format!("Service {service_id}")),
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
        service_app_name: Set(Some(service_app_name.to_string())),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn build_db_audited_state(
    db: sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Arc<AppState> {
    let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
    state
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn insert_test_system_service_row(
    db: &sea_orm::DatabaseConnection,
    service_id: Uuid,
    service_app_name: &str,
) {
    let now = time::OffsetDateTime::now_utc();
    uptrakit_shared_db::entity::system_service::ActiveModel {
        id: Set(service_id),
        capabilities: Set("[]".to_string()),
        hostname: Set(format!("sys-{service_id}")),
        friendly_name: Set(format!("System Service {service_id}")),
        ip_address: Set(None),
        status: Set(uptrakit_shared_db::entity::system_service::SystemServiceStatus::Approved),
        enrollment_secret_hash: Set(format!("secret-{service_id}")),
        client_version: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        cert_lifetime_hours: Set(None),
        system_enrollment_token_id: Set(None),
        service_app_name: Set(Some(service_app_name.to_string())),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn tenant_audit_row_for_action(
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

#[cfg(feature = "db-sqlite")]
pub(super) async fn system_audit_row_for_action(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> uptrakit_shared_db::entity::system_audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = uptrakit_shared_db::entity::system_audit_log::Entity::find()
            .filter(
                uptrakit_shared_db::entity::system_audit_log::Column::ActionType.eq(action_type),
            )
            .order_by_desc(uptrakit_shared_db::entity::system_audit_log::Column::OccurredAt)
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

// ---------------------------------------------------------------------------
// Fixtures from the embedded / reconnect cluster (db-sqlite required)
// ---------------------------------------------------------------------------

#[cfg(feature = "db-sqlite")]
#[derive(Default)]
pub(super) struct MockEmbeddedNotifier {
    pub(super) disconnected: parking_lot::Mutex<Vec<Uuid>>,
}

#[cfg(feature = "db-sqlite")]
impl EmbeddedServiceNotifier for MockEmbeddedNotifier {
    fn on_external_connected(
        &self,
        _service_id: Uuid,
        _capabilities: &BTreeSet<Capability>,
        _hostname: Option<&str>,
        _is_system: bool,
    ) {
    }

    fn on_external_disconnected(&self, service_id: &Uuid) {
        self.disconnected.lock().push(*service_id);
    }

    fn on_machine_id_reported(&self, _service_id: &Uuid, _machine_id: &str) {}

    fn is_capability_yielded(&self, _capability: &Capability) -> bool {
        false
    }
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn insert_service_row(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    service_id: Uuid,
    service_app_name: &str,
) {
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
        service_app_name: Set(Some(service_app_name.to_string())),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn insert_linked_host_and_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    service_id: Uuid,
) -> (Uuid, Uuid) {
    let now = OffsetDateTime::now_utc();
    insert_service_row(db, tenant_id, service_id, "uptrakit-agent").await;

    let host_id = host::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        machine_id: Set(format!("machine-{service_id}")),
        hostname: Set(format!("host-{service_id}")),
        friendly_name: Set(format!("Host {service_id}")),
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
    .unwrap()
    .id;

    let software_item_id = software_item::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        name: Set("demo".to_string()),
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
    .id;

    service_host::ActiveModel {
        service_id: Set(service_id),
        host_id: Set(host_id),
        linked_at: Set(now),
    }
    .insert(db)
    .await
    .unwrap();

    (host_id, software_item_id)
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn relink_service_host(
    db: &sea_orm::DatabaseConnection,
    service_id: Uuid,
    host_id: Uuid,
) {
    service_host::ActiveModel {
        service_id: Set(service_id),
        host_id: Set(host_id),
        linked_at: Set(OffsetDateTime::now_utc()),
    }
    .insert(db)
    .await
    .unwrap();
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn insert_owned_in_progress_update(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    owner_service_id: Uuid,
    owner_instance_id: Option<Uuid>,
) -> Uuid {
    let now = OffsetDateTime::now_utc();
    let id = Uuid::now_v7();
    update_history::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(None),
        from_version: Set(Some("1.0.0".to_string())),
        to_version: Set(Some("1.1.0".to_string())),
        status: Set(update_history::UpdateStatus::InProgress),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set("user".to_string()),
        actor_id: Set(String::new()),
        execution_owner_service_id: Set(Some(owner_service_id)),
        execution_owner_instance_id: Set(owner_instance_id),
        started_at: Set(Some(now)),
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
    id
}

#[cfg(feature = "db-sqlite")]
pub(super) async fn run_embedded_register_once(
    state: Arc<AppState>,
    service_id: Uuid,
    tenant_id: Uuid,
    capabilities: BTreeSet<Capability>,
    runtime_instance_id: Uuid,
) {
    let (_, connection_handle) = state
        .service_connections
        .register(
            service_id,
            capabilities.clone(),
            None,
            None,
            Some("uptrakit-agent".to_string()),
        )
        .await;
    let connection_id = connection_handle.connection_id();

    let (service_tx, service_rx) = tokio::sync::mpsc::channel(4);
    let cancel = CancellationToken::new();
    let handler_capabilities = capabilities.clone();
    let handle = tokio::spawn(async move {
        run_embedded_message_handler(
            super::embedded::EmbeddedHandlerCallParams {
                state: Arc::clone(&state),
                service_id,
                connection_id,
                capabilities: &handler_capabilities,
                app_name: "uptrakit-agent",
                service_rx,
                cancel: cancel.clone(),
            },
            tenant_id,
        )
        .await
    });

    service_tx
        .send(ServiceMessage::Register(
            RegisterPayload::new(capabilities.clone())
                .with_runtime_instance_id(runtime_instance_id),
        ))
        .await
        .unwrap();
    service_tx
        .send(ServiceMessage::Disconnecting(DisconnectingPayload::new(
            DisconnectReason::Shutdown,
        )))
        .await
        .unwrap();

    handle.await.unwrap();
}
