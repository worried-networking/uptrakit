//! Shared test harness for REST API integration tests.
//!
//! Provides [`TestApp`] — a self-contained test fixture that spins up a
//! fully wired Axum router backed by an in-memory SQLite database with
//! all migrations applied. Each test gets its own database, so tests are
//! parallel-safe without any cleanup.
//!
//! Gated behind `#[cfg(all(test, feature = "db-sqlite"))]`.

pub(crate) mod fixtures;
pub(crate) mod http_client;

use std::sync::Arc;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

use crate::auth::jwt::JwtManager;
use crate::auth::registration::{RegistrationMode, RegistrationSettings};
use crate::settings::Settings;
use crate::{AppState, ServiceCredentialSources, build_router};

/// Self-contained test fixture for integration tests.
#[allow(dead_code)]
pub(crate) struct TestApp {
    /// The shared application state.
    pub state: Arc<AppState>,
    /// The Axum router (clone-friendly).
    pub router: axum::Router,
    /// Direct database handle for fixture insertion / assertion queries.
    pub db: DatabaseConnection,
    /// JWT manager for issuing tokens outside the HTTP flow.
    pub jwt: Arc<JwtManager>,
    /// The UUID of the default tenant seeded during setup.
    pub tenant_id: uuid::Uuid,
}

impl TestApp {
    /// Build a fully initialised test app with migrated SQLite DB and
    /// seeded default tenant.
    pub(crate) async fn new() -> Self {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db.clone(), tenant_id).await;
        let router = build_router(Arc::clone(&state));

        Self {
            state,
            router,
            db,
            jwt,
            tenant_id,
        }
    }

    /// Create a [`http_client::TestClient`] for this app.
    pub(crate) fn client(&self) -> http_client::TestClient {
        http_client::TestClient::new(self.router.clone())
    }
}

/// Create an in-memory SQLite database with all migrations applied.
pub(crate) async fn setup_migrated_db() -> DatabaseConnection {
    let opt = ConnectOptions::new("sqlite::memory:".to_owned());
    let db = Database::connect(opt).await.expect("test db");
    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("run migrations");
    db
}

/// Insert a default tenant row and return its UUID.
pub(crate) async fn insert_default_tenant(db: &DatabaseConnection) -> uuid::Uuid {
    use sea_orm::ActiveModelTrait;
    use sea_orm::Set;
    use uptrakit_shared_db::entity::tenant;

    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    tenant::ActiveModel {
        id: Set(id),
        name: Set("default".to_string()),
        slug: Set(id.to_string()),
        is_default: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert default tenant");
    id
}

/// Build an [`AppState`] wired for testing — mirrors the canonical
/// `test_state()` from `lib.rs` but uses a migrated DB and a real tenant.
pub(crate) async fn build_test_state(
    db: DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> (Arc<AppState>, Arc<JwtManager>) {
    build_test_state_with_plugin_ops(db, tenant_id, None).await
}

pub(crate) async fn build_test_state_with_plugin_ops(
    db: DatabaseConnection,
    tenant_id: uuid::Uuid,
    plugin_ops_override: Option<Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps>>,
) -> (Arc<AppState>, Arc<JwtManager>) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Enable plaintext mode for crypto — tests don't need real encryption
    // and the master key is not initialized in the test harness.
    uptrakit_crypto::enable_plaintext_mode();

    // CA snapshot (dummy PEM).
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

    // Dummy RustlsConfig — tests don't do TLS handshakes.
    let rustls_cfg = {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
            .expect("cert params")
            .self_signed(&key_pair)
            .expect("self-sign");
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der())
                    .expect("key der"),
            )
            .expect("server config");
        axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
    };

    let settings = Settings::new(
        RegistrationSettings {
            mode: RegistrationMode::Open,
            token_hash: None,
            require_token_for_oidc: false,
        },
        168,
    );

    let jwt = Arc::new(JwtManager::from_secret(
        b"integration-test-jwt-secret-key-do-not-use",
    ));

    let service_connections = crate::service_connections::ServiceConnectionRegistry::new();
    let controller_id = uuid::Uuid::nil();
    let notification_service = crate::notification_service::NotificationService::new(
        service_connections.clone(),
        controller_id,
    );

    let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> =
        plugin_ops_override.unwrap_or_else(|| {
            Arc::new(
                uptrakit_plugin_infrastructure_registry::PluginRegistry::with_notifications(
                    uptrakit_plugin_infrastructure_registry::NotificationRegistryConfig::default(),
                )
                .expect("notification registry should build in tests"),
            )
        });

    let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
        db.clone(),
        Arc::clone(&plugin_ops),
        "https://localhost".to_string(),
    );

    let state = Arc::new(AppState {
        db: db.clone(),
        cert: crate::app_state::CertState {
            ca_snapshot: ca_rx,
            ca_key_store,
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
            ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
        },
        auth: crate::app_state::AuthState {
            jwt: Arc::clone(&jwt),
            device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
            rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
            token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
        },
        broadcast: crate::app_state::BroadcastState {
            event_broadcaster: crate::event_broadcaster::EventBroadcaster::new(),
            device_flow_broadcaster: crate::device_flow_broadcaster::DeviceFlowBroadcaster::new(),
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
        default_tenant_id: tenant_id,
        controller_id,
        notification_service,
        notification_dispatcher,
        plugin_ops,
        credential_sources: ServiceCredentialSources::default(),
        shutdown_token: Default::default(),
        embedded_service_notifier: None,
        audit_log_filter: uptrakit_audit_log::AuditFilter::default(),
        audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
            uptrakit_audit_log::NoopBackend,
        )),
        extension_registry: Arc::new(crate::extension_registry::ExtensionRegistry::new(vec![])),
        extension_proxy: Arc::new(crate::extension_proxy::ExtensionProxy::new()),
        config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
        workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
        pki_path: std::path::PathBuf::from("/tmp/test-pki"),
        rustls_config: rustls_cfg,
        reject_dangerous_commands: false,
        #[cfg(feature = "interactive")]
        interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
    });

    (state, jwt)
}

/// No-op certificate signer for tests.
struct NoopCertSigner;

#[async_trait::async_trait]
impl crate::cert_signer::AgentCertSigner for NoopCertSigner {
    async fn sign_agent_csr(
        &self,
        _csr_pem: &str,
        _service_id: &uuid::Uuid,
        _lifetime: time::Duration,
    ) -> std::result::Result<
        crate::cert_signer::SignedCertBundle,
        rootcause::Report<crate::cert_signer::CertSignerError>,
    > {
        Err(rootcause::report!(
            crate::cert_signer::CertSignerError::Signing("noop signer".to_string())
        ))
    }

    fn active_ca_fingerprint(&self) -> String {
        "0".repeat(64)
    }
}
