use std::sync::Arc;

use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use uptrakit_web_api::auth::jwt::JwtManager;
use uptrakit_web_api::{AppState, build_router};

use super::http_client::TestClient;

/// Self-contained test fixture for database integration tests.
///
/// Mirrors the `TestApp` from `web-api/src/test_harness/mod.rs` but
/// constructs `AppState` via the public builder API so it can be used
/// from an external crate. Supports SQLite and PostgreSQL.
#[allow(dead_code)]
pub(crate) struct TestHarness {
    pub state: Arc<AppState>,
    pub router: axum::Router,
    pub db: DatabaseConnection,
    pub jwt: Arc<JwtManager>,
    pub tenant_id: uuid::Uuid,
    /// Keeps container/tempdir alive for the duration of the test.
    _guard: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

impl TestHarness {
    /// Build a harness from an existing database connection and optional guard.
    async fn with_db(
        db: DatabaseConnection,
        guard: Option<Arc<dyn std::any::Any + Send + Sync>>,
    ) -> Self {
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db.clone(), tenant_id).await;
        let router = build_router(Arc::clone(&state));

        Self {
            state,
            router,
            db,
            jwt,
            tenant_id,
            _guard: guard,
        }
    }

    /// Create a harness backed by file-based SQLite.
    pub(crate) async fn new_sqlite() -> Self {
        let (db, guard) = super::db_providers::setup_sqlite().await;
        Self::with_db(db, guard).await
    }

    /// Create a harness backed by PostgreSQL (testcontainers).
    pub(crate) async fn new_postgres() -> Self {
        let (db, guard) = super::db_providers::setup_postgres().await;
        Self::with_db(db, guard).await
    }

    /// Create a [`TestClient`] for this harness.
    pub(crate) fn client(&self) -> TestClient {
        TestClient::new(self.router.clone())
    }
}

/// Insert a default tenant row and return its UUID.
async fn insert_default_tenant(db: &DatabaseConnection) -> uuid::Uuid {
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

/// Build an [`AppState`] via the public builder API.
///
/// Mirrors `build_test_state()` from `web-api/src/test_harness/mod.rs`
/// but uses `AppState::builder()` instead of direct struct construction.
async fn build_test_state(
    db: DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> (Arc<AppState>, Arc<JwtManager>) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    uptrakit_crypto::enable_plaintext_mode();

    // CA snapshot (dummy PEM).
    let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
    let snapshot_data = uptrakit_web_api::ca_snapshot::CaPublicSnapshot {
        active_cert_pem: ca_pem.to_string(),
        active_fingerprint: "0".repeat(64),
        previous_cert_pem: None,
        previous_fingerprint: None,
        trusted_cas: vec![uptrakit_web_api::ca_snapshot::TrustedCaPublic {
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

    let ca_key_store: uptrakit_web_api::CaKeyStoreRef = Arc::new(tokio::sync::RwLock::new(
        uptrakit_web_api::ca_snapshot::CaKeyStore {
            active_key_pem: zeroize::Zeroizing::new(String::new()),
            previous_key_pem: None,
            trusted_ca_keys: vec![],
        },
    ));

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

    let jwt = Arc::new(JwtManager::from_secret(
        b"integration-test-jwt-secret-key-do-not-use",
    ));

    let service_connections =
        uptrakit_web_api::service_connections::ServiceConnectionRegistry::new();
    let controller_id = uuid::Uuid::nil();

    let notification_service = uptrakit_web_api::notification_service::NotificationService::new(
        service_connections.clone(),
        controller_id,
    );

    let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("plugin catalog should build in tests"),
    );

    let notification_dispatcher =
        uptrakit_web_api::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

    let settings = uptrakit_web_api::settings::Settings::new(
        uptrakit_web_api::auth::registration::RegistrationSettings {
            mode: uptrakit_web_api::auth::registration::RegistrationMode::Open,
            token_hash: None,
            require_token_for_oidc: false,
        },
        168,
    );

    // Build stores using cloned db handles (stores take ownership).
    let device_flow_store = uptrakit_web_api::auth::device_flow::DeviceFlowStore::new(db.clone());
    let rate_limit_store = uptrakit_web_api::auth::rate_limit::RateLimitStore::new(db.clone());
    let oidc_flow_store = uptrakit_web_api::auth::oidc_state::OidcFlowStore::new(db.clone());
    let account_link_store = uptrakit_web_api::auth::oidc_state::AccountLinkStore::new(db.clone());
    let oidc_token_exchange_store =
        uptrakit_web_api::auth::oidc_state::OidcTokenExchangeStore::new(db.clone());
    let oidc_registration_store =
        uptrakit_web_api::auth::oidc_state::OidcRegistrationStore::new(db.clone());

    let state = AppState::builder()
        .db(db)
        .ca_snapshot(ca_rx)
        .ca_key_store(ca_key_store)
        .revocation_notify(Arc::new(tokio::sync::Notify::const_new()))
        .crl_pem_cache(Arc::new(tokio::sync::RwLock::new(String::new())))
        .ca_rotation_trigger(Arc::new(tokio::sync::Notify::const_new()))
        .jwt(Arc::clone(&jwt))
        .device_flow_store(device_flow_store)
        .rate_limit_store(rate_limit_store)
        .token_denylist(Arc::new(
            uptrakit_web_api::auth::token_denylist::TokenDenylist::new(),
        ))
        .oidc_flow_store(oidc_flow_store)
        .account_link_store(account_link_store)
        .oidc_token_exchange_store(oidc_token_exchange_store)
        .oidc_registration_store(oidc_registration_store)
        .settings(settings)
        .cert_signer(Arc::new(NoopCertSigner))
        .service_connections(service_connections)
        .default_tenant_id(tenant_id)
        .controller_id(controller_id)
        .notification_service(notification_service)
        .notification_dispatcher(notification_dispatcher)
        .plugin_ops(plugin_ops)
        .pki_path(std::path::PathBuf::from("/tmp/test-pki"))
        .rustls_config(rustls_cfg)
        .build()
        .expect("build AppState");

    (Arc::new(state), jwt)
}

/// No-op certificate signer for tests.
struct NoopCertSigner;

#[async_trait::async_trait]
impl uptrakit_web_api::cert_signer::AgentCertSigner for NoopCertSigner {
    async fn sign_agent_csr(
        &self,
        _csr_pem: &str,
        _service_id: &uuid::Uuid,
        _lifetime: time::Duration,
    ) -> std::result::Result<
        uptrakit_web_api::cert_signer::SignedCertBundle,
        rootcause::Report<uptrakit_web_api::cert_signer::CertSignerError>,
    > {
        Err(rootcause::report!(
            uptrakit_web_api::cert_signer::CertSignerError::Signing("noop signer".to_string())
        ))
    }

    fn active_ca_fingerprint(&self) -> String {
        "0".repeat(64)
    }
}

/// Initialize tracing for test output (only once per process).
pub(crate) fn init_test_tracing() {
    uptrakit_tracing_init::init_test_tracing();
}
