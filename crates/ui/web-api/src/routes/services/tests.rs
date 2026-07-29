#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget sends in tests drop results intentionally"
)]

use super::*;
use crate::AppState;
use crate::ServiceCredentialSources;
use crate::auth::AuthMethod;
use crate::auth::permissions::Permission;
use crate::auth::registration::{RegistrationMode, RegistrationSettings};
use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
use crate::middleware::permission::{
    CanApproveServices, CanRejectServices, CanRemoveServices, CanUpdateServices,
};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
use crate::settings::Settings;
use crate::tenant_db::TenantDb;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use sea_orm::{
    ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, QueryOrder, Set,
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{audit_log, prelude::Service, service, tenant};

async fn setup_test_db() -> DatabaseConnection {
    let opt = ConnectOptions::new("sqlite::memory:");
    let db = Database::connect(opt).await.expect("test db");
    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("migrations");
    db
}

async fn insert_tenant(db: &DatabaseConnection, id: uuid::Uuid) {
    let now = OffsetDateTime::now_utc();
    tenant::ActiveModel {
        id: Set(id),
        name: Set("Test Tenant".to_string()),
        slug: Set(id.to_string()),
        is_default: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert tenant");
}

async fn test_state(db: DatabaseConnection, tenant_id: uuid::Uuid) -> Arc<AppState> {
    struct NoopCertSigner;
    #[async_trait::async_trait]
    impl AgentCertSigner for NoopCertSigner {
        async fn sign_agent_csr(
            &self,
            _: &str,
            _: &uuid::Uuid,
            _: time::Duration,
        ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>> {
            Err(rootcause::report!(CertSignerError::Signing(
                "noop signer".to_string(),
            )))
        }
        fn active_ca_fingerprint(&self) -> String {
            "0".repeat(64)
        }
    }

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
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .self_signed(&key_pair)
            .unwrap();
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap(),
            )
            .unwrap();
        axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
    };

    let notification_service = crate::notification_service::NotificationService::new(
        crate::service_connections::ServiceConnectionRegistry::new(),
        uuid::Uuid::nil(),
    );

    let settings = Settings::new(
        RegistrationSettings {
            mode: RegistrationMode::Open,
            token_hash: None,
            require_token_for_oidc: false,
        },
        168,
    );

    let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
        )
        .expect("default catalog should build"),
    );

    let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
        db.clone(),
        Arc::clone(&plugin_ops),
        "https://localhost".to_string(),
    );

    let (_, config_rx_for_services) = uptrakit_config_reload::RuntimeConfigChannels::from_runtime(
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
                b"test-secret-for-service-merge-tests",
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
        default_tenant_id: tenant_id,
        settings,
        cert_signer: Arc::new(NoopCertSigner),
        service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
        controller_id: uuid::Uuid::nil(),
        plugin: crate::app_state::PluginState::new(
            plugin_ops,
            Arc::new(crate::global_providers::GlobalProviders::new(db.clone())),
        ),
        credential_sources: ServiceCredentialSources::default(),
        shutdown_token: Default::default(),
        embedded_service_notifier: None,
        audit_log_filter_rx: tokio::sync::watch::channel(std::sync::Arc::new(
            uptrakit_config_reload::config::AuditConfig::default(),
        ))
        .1,
        audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
            uptrakit_audit_log::DatabaseBackend::new(db.clone()),
        )),
        audit_emitter: {
            let db_backend =
                std::sync::Arc::new(uptrakit_audit_log::DatabaseBackend::new(db.clone()))
                    as std::sync::Arc<dyn uptrakit_audit_log::AuditLogBackend>;
            let mirror = std::sync::Arc::new(uptrakit_audit_log::NoopBackend)
                as std::sync::Arc<dyn uptrakit_audit_log::AuditLogBackend>;
            uptrakit_audit_log::AuditEmitter::with_backends(
                uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                    uptrakit_audit_log::DatabaseBackend::new(db.clone()),
                )),
                db_backend,
                mirror,
            )
        },
        surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
            Arc::new(crate::surface_registry::SurfaceRegistry::new(
                crate::surface_registry::SurfaceRegistryConfig::default(),
            )),
            Arc::new(crate::surface_proxy::SurfaceProxy::new()),
            Arc::new(crate::surface_proxy::AllProvidersVisible),
        ),
        config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
        workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
        server: crate::app_state::ServerState::new(
            std::path::PathBuf::from("/tmp/test-pki"),
            rustls_cfg,
        ),
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
        db_config_rx: config_rx_for_services.db,
        network_config_rx: config_rx_for_services.network,
        nats_config_rx: config_rx_for_services.nats,
        tls_config_rx: config_rx_for_services.tls,
        audit_config_rx: config_rx_for_services.audit,
        log_config_rx: config_rx_for_services.log,
        master_key_config_rx: config_rx_for_services.master_key,
        embedded_services_config_rx: config_rx_for_services.embedded_services,
        zeroconf_config_rx: config_rx_for_services.zeroconf,
        oauth: crate::oauth::OAuthState::disabled(),
        config_file_state: tokio::sync::watch::channel(
            uptrakit_config_reload::ConfigFileState::default(),
        )
        .1,
        last_reload: tokio::sync::watch::channel(None).1,
        recent_reload_events: tokio::sync::watch::channel(Vec::new()).1,
    })
}

async fn latest_tenant_audit_row(db: &DatabaseConnection) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
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

fn agent_caps_json() -> String {
    use std::collections::BTreeSet;
    use uptrakit_wire::Capability;
    uptrakit_wire::service_profile::serialize_capabilities(&BTreeSet::from([
        Capability::GracefulShutdown,
        Capability::SoftwareDiscovery,
        Capability::UpdateHooks,
    ]))
}

/// Helper: insert a pair of test services (approved target + pending source).
async fn insert_target_and_source(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> (service::Model, service::Model) {
    let now = OffsetDateTime::now_utc();
    let target = service::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        capabilities: Set(agent_caps_json()),
        hostname: Set("target-host".to_string()),
        friendly_name: Set("Target".to_string()),
        ip_address: Set(None),
        status: Set(service::ServiceStatus::Approved),
        enrollment_secret_hash: Set("target-hash".to_string()),
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
    };
    let target = target.insert(db).await.unwrap();

    let source = service::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        capabilities: Set(agent_caps_json()),
        hostname: Set("source-host".to_string()),
        friendly_name: Set("Source".to_string()),
        ip_address: Set(None),
        status: Set(service::ServiceStatus::Pending),
        enrollment_secret_hash: Set("source-hash".to_string()),
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
    };
    let source = source.insert(db).await.unwrap();

    (target, source)
}

/// When the target agent is currently connected the merge must be rejected
/// with 409 CONFLICT and leave the source service completely unmodified.
#[tokio::test]
async fn merge_service_target_connected_returns_conflict() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (target, source) = insert_target_and_source(&db, tenant_id).await;

    // Register the target as connected — merge must be rejected before any DB changes.
    let caps = {
        use std::collections::BTreeSet;
        use uptrakit_wire::Capability;
        BTreeSet::from([
            Capability::GracefulShutdown,
            Capability::SoftwareDiscovery,
            Capability::UpdateHooks,
        ])
    };
    let (_rx, _token) = state
        .service_connections
        .register(target.id, caps, None, None, None)
        .await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::UpdateServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = merge_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateServices::new(auth_user),
        None,
        Path(target.id),
        Json(MergeAgentRequest {
            source_id: source.id,
        }),
    )
    .await;

    let status = match response {
        Err(e) => e.into_response().status(),
        Ok(_) => panic!("expected Err(ApiError) but got Ok"),
    };
    assert_eq!(status, StatusCode::CONFLICT);

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["source_service_id"], serde_json::json!(source.id));
    assert_eq!(
        details["reason_code"],
        serde_json::json!("service.target_connected")
    );

    // Source must not have been touched.
    let source_after = Service::find_by_id(source.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(source_after.deactivated_at.is_none());
    assert_eq!(source_after.enrollment_secret_hash, "source-hash");
}

/// A merge of a valid pending source into an approved target must succeed:
/// the source is deactivated (with its hash invalidated) and the target
/// adopts the source's identity fields.
#[tokio::test]
async fn merge_service_succeeds_and_deactivates_source() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (target, source) = insert_target_and_source(&db, tenant_id).await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::UpdateServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = merge_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateServices::new(auth_user),
        None,
        Path(target.id),
        Json(MergeAgentRequest {
            source_id: source.id,
        }),
    )
    .await;

    let status = match response {
        Ok(r) => r.into_response().status(),
        Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
    };
    assert_eq!(status, StatusCode::OK);

    // Source must be deactivated and its original hash must be invalidated.
    let source_after = Service::find_by_id(source.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        source_after.deactivated_at.is_some(),
        "source must be deactivated after merge"
    );
    assert_ne!(
        source_after.enrollment_secret_hash, "source-hash",
        "source hash must be invalidated after merge"
    );

    // Target must have adopted the source's identity.
    let target_after = Service::find_by_id(target.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target_after.hostname, "source-host");
    assert_eq!(target_after.enrollment_secret_hash, "source-hash");
}

#[tokio::test]
async fn merge_service_writes_service_merge_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (target, source) = insert_target_and_source(&db, tenant_id).await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::UpdateServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = merge_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateServices::new(auth_user),
        None,
        Path(target.id),
        Json(MergeAgentRequest {
            source_id: source.id,
        }),
    )
    .await;

    let status = match response {
        Ok(r) => r.into_response().status(),
        Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
    };
    assert_eq!(status, StatusCode::OK);

    let row = latest_tenant_audit_row(&db).await;
    let expected_target_id = target.id.to_string();
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("service"));
    assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::User.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["source_service_id"], serde_json::json!(source.id));
}

#[tokio::test]
async fn merge_service_api_token_actor_writes_api_token_actor_type() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (target, source) = insert_target_and_source(&db, tenant_id).await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::ApiToken,
        vec![Permission::UpdateServices],
        None,
    );
    let token_id = uuid::Uuid::now_v7();
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = merge_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateServices::new(auth_user),
        Some(Extension(AuthenticatedApiTokenId(token_id))),
        Path(target.id),
        Json(MergeAgentRequest {
            source_id: source.id,
        }),
    )
    .await;

    let status = match response {
        Ok(r) => r.into_response().status(),
        Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
    };
    assert_eq!(status, StatusCode::OK);

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::ApiToken.as_str()
    );
    assert_eq!(row.actor_id, Some(token_id));
}

#[tokio::test]
async fn approve_service_writes_service_approve_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (_target, source) = insert_target_and_source(&db, tenant_id).await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::ApproveServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = approve_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanApproveServices::new(auth_user),
        None,
        Path(source.id),
    )
    .await;

    let status = match response {
        Ok(r) => r.into_response().status(),
        Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
    };
    assert_eq!(status, StatusCode::OK);

    let row = latest_tenant_audit_row(&db).await;
    let expected_target_id = source.id.to_string();
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("service"));
    assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
}

#[tokio::test]
async fn update_service_missing_service_writes_denied_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::UpdateServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
    let missing_service_id = uuid::Uuid::now_v7();

    let response = update_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateServices::new(auth_user),
        None,
        Path(missing_service_id),
        Json(UpdateServiceRequest {
            ping_interval_seconds: Some(15),
            cert_lifetime_hours: Some(72),
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(
        row.target_id.as_deref(),
        Some(missing_service_id.to_string().as_str())
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("service.not_found")
    );
}

#[tokio::test]
async fn approve_service_missing_service_writes_denied_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::ApproveServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
    let missing_service_id = uuid::Uuid::now_v7();

    let response = approve_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanApproveServices::new(auth_user),
        None,
        Path(missing_service_id),
    )
    .await;

    let status = match response {
        Err(e) => e.into_response().status(),
        Ok(_) => panic!("expected Err(ApiError) but got Ok"),
    };
    assert_eq!(status, StatusCode::NOT_FOUND);

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(
        row.target_id.as_deref(),
        Some(missing_service_id.to_string().as_str())
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("service.not_found")
    );
}

#[tokio::test]
async fn reject_service_writes_service_reject_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (_target, source) = insert_target_and_source(&db, tenant_id).await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::RejectServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = reject_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanRejectServices::new(auth_user),
        None,
        Path(source.id),
    )
    .await;

    let status = match response {
        Ok(r) => r.into_response().status(),
        Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
    };
    assert_eq!(status, StatusCode::OK);

    let row = latest_tenant_audit_row(&db).await;
    let expected_target_id = source.id.to_string();
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_REJECT,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("service"));
    assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
}

#[tokio::test]
async fn deactivate_service_writes_service_deactivate_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (target, _source) = insert_target_and_source(&db, tenant_id).await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::RemoveServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = deactivate_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanRemoveServices::new(auth_user),
        None,
        Path(target.id),
    )
    .await;

    let status = match response {
        Ok(r) => r.into_response().status(),
        Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
    };
    assert_eq!(status, StatusCode::NO_CONTENT);

    let row = latest_tenant_audit_row(&db).await;
    let expected_target_id = target.id.to_string();
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("service"));
    assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
}

#[tokio::test]
async fn deactivate_service_missing_service_writes_denied_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::RemoveServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
    let missing_service_id = uuid::Uuid::now_v7();

    let response = deactivate_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanRemoveServices::new(auth_user),
        None,
        Path(missing_service_id),
    )
    .await;

    let status = match response {
        Ok(r) => r.into_response().status(),
        Err(e) => panic!(
            "expected Ok(response) but got Err: {}",
            e.into_response().status()
        ),
    };
    assert_eq!(status, StatusCode::NOT_FOUND);

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(
        row.target_id.as_deref(),
        Some(missing_service_id.to_string().as_str())
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("service.not_found")
    );
}

#[tokio::test]
async fn set_update_freeze_writes_service_freeze_enable_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (target, _source) = insert_target_and_source(&db, tenant_id).await;

    let caps = {
        use std::collections::BTreeSet;
        use uptrakit_wire::Capability;
        BTreeSet::from([
            Capability::GracefulShutdown,
            Capability::SoftwareDiscovery,
            Capability::UpdateHooks,
        ])
    };
    let (_rx, _token) = state
        .service_connections
        .register(target.id, caps, None, None, None)
        .await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::UpdateServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = set_update_freeze(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateServices::new(auth_user),
        None,
        Path(target.id),
        Json(SetUpdateFreezeRequest {
            enabled: true,
            reason: Some("maintenance".to_string()),
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let row = latest_tenant_audit_row(&db).await;
    let expected_target_id = target.id.to_string();
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_UPDATE_FREEZE_ENABLE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("service"));
    assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
    let details = row.details_json.expect("details");
    assert_eq!(details["enabled"], serde_json::json!(true));
    assert_eq!(details["reason_present"], serde_json::json!(true));
}

#[tokio::test]
async fn set_update_freeze_not_connected_writes_denied_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (target, _source) = insert_target_and_source(&db, tenant_id).await;
    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::UpdateServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = set_update_freeze(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateServices::new(auth_user),
        None,
        Path(target.id),
        Json(SetUpdateFreezeRequest {
            enabled: false,
            reason: None,
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_UPDATE_FREEZE_DISABLE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(
        row.target_id.as_deref(),
        Some(target.id.to_string().as_str())
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["enabled"], serde_json::json!(false));
    assert_eq!(details["reason_present"], serde_json::json!(false));
    assert_eq!(
        details["reason_code"],
        serde_json::json!("service.not_connected")
    );
}

#[tokio::test]
async fn batch_services_invalid_request_writes_validation_failed_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::ApproveServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = batch_services(
        State(Arc::clone(&state)),
        tenant_db,
        Extension(auth_user),
        None,
        Json(BatchActionRequest {
            action: "approve".to_string(),
            ids: vec![],
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
    assert_eq!(details["batch"], serde_json::json!(true));
}

#[tokio::test]
async fn batch_services_permission_denied_writes_denied_audit_event() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;
    let (target, _source) = insert_target_and_source(&db, tenant_id).await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::ViewServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = batch_services(
        State(Arc::clone(&state)),
        tenant_db,
        Extension(auth_user),
        None,
        Json(BatchActionRequest {
            action: "approve".to_string(),
            ids: vec![target.id],
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("insufficient_permissions")
    );
    assert_eq!(details["batch"], serde_json::json!(true));
}

/// Seed a single embedded service for merge-guard tests.
async fn insert_service_embedded(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
    service_id: uuid::Uuid,
    status: uptrakit_shared_db::entity::service::ServiceStatus,
) {
    use uptrakit_wire::Capability;
    use uptrakit_wire::service_profile::serialize_capabilities;
    let now = OffsetDateTime::now_utc();
    let caps = serialize_capabilities(&[Capability::SoftwareDiscovery].into_iter().collect());
    uptrakit_shared_db::entity::service::ActiveModel {
        id: Set(service_id),
        tenant_id: Set(tenant_id),
        capabilities: Set(caps),
        hostname: Set(format!("embedded-svc-{service_id}")),
        friendly_name: Set(format!("Embedded Service {service_id}")),
        ip_address: Set(None),
        status: Set(status),
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
        is_embedded: Set(true),
        embedded_owner_key: Set(None),
    }
    .insert(db)
    .await
    .expect("insert embedded service");
}

/// Merge into an embedded target must be rejected with 400 and a ValidationFailed audit.
#[tokio::test]
async fn merge_service_returns_400_when_target_embedded() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;

    let target_id = uuid::Uuid::now_v7();
    let source_id = uuid::Uuid::now_v7();
    insert_service_embedded(
        &db,
        tenant_id,
        target_id,
        uptrakit_shared_db::entity::service::ServiceStatus::Approved,
    )
    .await;
    let (_source, _) = insert_target_and_source(&db, tenant_id).await;
    // Re-use the approved non-embedded service from insert_target_and_source as source,
    // but we need an independent source — insert one separately.
    use uptrakit_wire::Capability;
    use uptrakit_wire::service_profile::serialize_capabilities;
    let now = OffsetDateTime::now_utc();
    let caps = serialize_capabilities(&[Capability::SoftwareDiscovery].into_iter().collect());
    uptrakit_shared_db::entity::service::ActiveModel {
        id: Set(source_id),
        tenant_id: Set(tenant_id),
        capabilities: Set(caps),
        hostname: Set(format!("svc-{source_id}")),
        friendly_name: Set(format!("Service {source_id}")),
        ip_address: Set(None),
        status: Set(uptrakit_shared_db::entity::service::ServiceStatus::Pending),
        enrollment_secret_hash: Set(format!("secret-{source_id}")),
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
    .insert(&db)
    .await
    .expect("insert source service");

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::UpdateServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = merge_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateServices::new(auth_user),
        None,
        Path(target_id),
        Json(MergeAgentRequest { source_id }),
    )
    .await;

    let response = match response {
        Ok(r) => r.into_response(),
        Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
    };
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"].as_str(), Some("service.embedded_target"));

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("service.embedded_target")
    );
    assert_eq!(details["source_service_id"], serde_json::json!(source_id));
}

/// Merge from an embedded source must be rejected with 400 and a ValidationFailed audit.
#[tokio::test]
async fn merge_service_returns_400_when_source_embedded() {
    let db = setup_test_db().await;
    let tenant_id = uuid::Uuid::now_v7();
    insert_tenant(&db, tenant_id).await;
    let state = test_state(db.clone(), tenant_id).await;

    let target_id = uuid::Uuid::now_v7();
    let source_id = uuid::Uuid::now_v7();

    use uptrakit_wire::Capability;
    use uptrakit_wire::service_profile::serialize_capabilities;
    let now = OffsetDateTime::now_utc();
    let caps = serialize_capabilities(&[Capability::SoftwareDiscovery].into_iter().collect());
    // Insert a normal (non-embedded) approved target.
    uptrakit_shared_db::entity::service::ActiveModel {
        id: Set(target_id),
        tenant_id: Set(tenant_id),
        capabilities: Set(caps.clone()),
        hostname: Set(format!("svc-{target_id}")),
        friendly_name: Set(format!("Service {target_id}")),
        ip_address: Set(None),
        status: Set(uptrakit_shared_db::entity::service::ServiceStatus::Approved),
        enrollment_secret_hash: Set(format!("secret-{target_id}")),
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
    .insert(&db)
    .await
    .expect("insert target service");

    // Insert an embedded source.
    insert_service_embedded(
        &db,
        tenant_id,
        source_id,
        uptrakit_shared_db::entity::service::ServiceStatus::Pending,
    )
    .await;

    let auth_user = AuthenticatedUser::new(
        uuid::Uuid::now_v7(),
        AuthMethod::Password,
        vec![Permission::UpdateServices],
        None,
    );
    let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

    let response = merge_service(
        State(Arc::clone(&state)),
        tenant_db,
        CanUpdateServices::new(auth_user),
        None,
        Path(target_id),
        Json(MergeAgentRequest { source_id }),
    )
    .await;

    let response = match response {
        Ok(r) => r.into_response(),
        Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
    };
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"].as_str(), Some("service.embedded_source"));

    let row = latest_tenant_audit_row(&db).await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("service.embedded_source")
    );
    assert_eq!(details["source_service_id"], serde_json::json!(source_id));
}
