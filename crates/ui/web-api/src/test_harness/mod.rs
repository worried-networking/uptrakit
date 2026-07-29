//! Shared test harness for REST API integration tests.
//!
//! Provides [`TestApp`] — a self-contained test fixture that spins up a
//! fully wired Axum router backed by an in-memory SQLite database with
//! all migrations applied. Each test gets its own database, so tests are
//! parallel-safe without any cleanup.
//!
//! Gated behind `#[cfg(all(test, feature = "db-sqlite"))]`.

#![expect(
    clippy::expect_used,
    reason = "test fixture: panics on setup failure are acceptable"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget sends in test setup drop results intentionally"
)]

pub(crate) mod fixtures;
pub(crate) mod http_client;

use std::sync::Arc;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};

use crate::auth::jwt::JwtManager;
use crate::auth::registration::{RegistrationMode, RegistrationSettings};
use crate::settings::Settings;
use crate::{AppState, ServiceCredentialSources, build_router};

/// Self-contained test fixture for integration tests.
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

    /// Build a [`TestApp`] with a single stub surface provider (`"test.stub"`)
    /// registered on `ControllerLocal` transport, backed by a
    /// [`RecordingSurfaceExecutor`] that records every dispatched
    /// [`surfaces::SurfaceActionRequest`] and always returns
    /// `{"ok": true}`.
    ///
    /// **Dispatch-only.** [`crate::surface_registry::SurfaceRegistry::register_provider_for_test`]
    /// bypasses the admission pipeline entirely (no `validate_against`, no
    /// `normalize_interaction_methods`), so this stub is unsuitable for
    /// testing admission-time rejection or normalization behavior — only for
    /// exercising router → proxy → local-executor dispatch once a descriptor
    /// is already registered. Admission coverage (`register_service` /
    /// `bootstrap_plugin`) is Plan 3 scope.
    ///
    /// Each [`StubInteraction`]'s stored `http_method` is normalized to its
    /// `effective_http_method()` before registration, mirroring what
    /// production admission (`normalize_interaction_methods`) would have
    /// done — so DataLoad stubs carry `Get` exactly like real descriptors.
    pub(crate) async fn with_stub_surfaces(
        stubs: Vec<StubInteraction>,
    ) -> (Self, StubSurfaceCalls) {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db.clone(), tenant_id).await;

        let calls: StubSurfaceCalls = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let surface_registry = Arc::new(crate::surface_registry::SurfaceRegistry::new(
            crate::surface_registry::SurfaceRegistryConfig::default(),
        ));
        let surface_proxy = Arc::new(
            crate::surface_proxy::SurfaceProxy::new()
                .with_local_executor(Arc::new(RecordingSurfaceExecutor {
                    calls: Arc::clone(&calls),
                }))
                .with_provider_visibility(Arc::new(crate::surface_proxy::AllProvidersVisible)),
        );

        let mut app_state = (*state).clone();
        app_state.surface_proxy_deps = crate::app_state::SurfaceProxyDeps::new(
            Arc::clone(&surface_registry),
            surface_proxy,
            Arc::new(crate::surface_proxy::AllProvidersVisible),
        );
        let state = Arc::new(app_state);

        surface_registry.register_provider_for_test(stub_surface_registration(stubs), None, None);

        let router = build_router(Arc::clone(&state));

        let app = Self {
            state,
            router,
            db,
            jwt,
            tenant_id,
        };
        (app, calls)
    }
}

/// Input descriptor for [`TestApp::with_stub_surfaces`]: a minimal,
/// caller-authored interaction to register on the `"test.stub"` surface.
pub(crate) struct StubInteraction {
    pub interaction_id: &'static str,
    pub kind: uptrakit_wire::surfaces::InteractionKind,
    pub http_method: Option<uptrakit_wire::surfaces::InteractionHttpMethod>,
    pub params: Vec<uptrakit_wire::surfaces::ParamFieldDescriptor>,
    pub required_permission: Option<String>,
}

/// Requests recorded by [`RecordingSurfaceExecutor`], shared with the test
/// via [`TestApp::with_stub_surfaces`]'s return value.
pub(crate) type StubSurfaceCalls =
    Arc<parking_lot::Mutex<Vec<uptrakit_wire::surfaces::SurfaceActionRequest>>>;

/// Builds the `"test.stub"` [`surfaces::SurfaceRegistration`] carrying
/// `stubs` as `ControllerLocal` interactions on a global, universally
/// targeted surface (provider kind `Plugin` — required by
/// `SurfaceProxy::invoke` for `ControllerLocal` transport regardless of the
/// local executor wired in).
fn stub_surface_registration(
    stubs: Vec<StubInteraction>,
) -> uptrakit_wire::surfaces::SurfaceRegistration {
    use uptrakit_wire::surfaces;

    let interactions = stubs
        .into_iter()
        .map(|stub| {
            let declared_method = stub.http_method.unwrap_or_default();
            let mut descriptor = surfaces::InteractionDescriptor::new(
                surfaces::InteractionId::new(stub.interaction_id)
                    .expect("stub interaction_id must be a valid InteractionId"),
                stub.kind,
                stub.interaction_id,
                surfaces::InteractionTransport::ControllerLocal,
            )
            .with_http_method(declared_method)
            .with_params(stub.params);
            descriptor.required_permission = stub.required_permission;
            // Normalize before registering: `register_provider_for_test`
            // bypasses admission's `normalize_interaction_methods`, so do it
            // here instead — DataLoad stubs must carry `Get` like production
            // descriptors do.
            let normalized_method = descriptor.effective_http_method();
            descriptor.with_http_method(normalized_method)
        })
        .collect();

    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: "test.stub.provider".to_string(),
            provider_kind: surfaces::ProviderKind::Plugin,
            provider_namespace: "plugin".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
            surfaces::Capability::MutationAction,
            surfaces::Capability::DataLoad,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(surfaces::SurfaceId::new("test.stub").expect("valid surface id"))
                .label("Stub Surface")
                .priority(100)
                .slot(surfaces::SLOT_SETTINGS_BELOW_GLOBAL)
                .scope(surfaces::Scope::Global)
                .targeting(surfaces::Targeting::Universal)
                .provider_kind(surfaces::ProviderKind::Plugin)
                .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::UniversalTargeting,
                ]))
                .root_node(surfaces::SurfaceNode::section(None::<String>, vec![]))
                .build(),
            interactions,
            data_sources: vec![],
        }],
        encryption_metadata: None,
    }
}

/// [`crate::surface_proxy::SurfaceLocalActionExecutor`] that records every
/// dispatched request into a shared [`StubSurfaceCalls`] and always succeeds
/// with `{"ok": true}` — the harness counterpart to
/// [`crate::surface_proxy::PluginSurfaceLocalExecutor`] (real dispatch) and
/// the crate-internal `NoopSurfaceLocalExecutor` (always errors).
struct RecordingSurfaceExecutor {
    calls: StubSurfaceCalls,
}

#[async_trait::async_trait]
impl crate::surface_proxy::SurfaceLocalActionExecutor for RecordingSurfaceExecutor {
    async fn execute(
        &self,
        _resolved: &crate::surface_registry::ResolvedSurfaceAction,
        request: &uptrakit_wire::surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, crate::surface_proxy::SurfaceProxyError> {
        self.calls.lock().push(request.clone());
        Ok(serde_json::json!({"ok": true}))
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

/// Create an in-memory SQLite database with core **and plugin** migrations
/// applied — mirrors production's plugin-aware migration set
/// (`uptrakit_shared_db::migration::run_migrations_with_plugins`, driven by
/// `controller-runtime`'s boot sequence) rather than [`setup_migrated_db`]'s
/// core-only set.
///
/// Needed by tests that seed plugin-owned tables (e.g.
/// `proxmox_host_mapping`) directly, or that exercise `ControllerLocal`
/// surface dispatch via [`build_test_state_with_plugin_surfaces`] — without
/// this, plugin tables don't exist and inserts/queries fail with
/// "no such table".
pub(crate) async fn setup_migrated_db_with_plugins() -> DatabaseConnection {
    let opt = ConnectOptions::new("sqlite::memory:".to_owned());
    let db = Database::connect(opt).await.expect("test db");
    uptrakit_shared_db::migration::run_migrations_with_plugins(&db, || {
        uptrakit_plugin_infrastructure_registry::all_descriptors()
            .into_iter()
            .filter_map(|descriptor| descriptor.migrations)
            .flat_map(|migrations_fn| migrations_fn())
            .collect()
    })
    .await
    .expect("run core + plugin migrations");
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

/// Build a test [`AppState`] with `server.has_external_tls_cert` forced to `true`.
///
/// Used by tests that assert the manual server-cert renewal API rejects when
/// the TLS certificate is externally managed.
pub(crate) async fn build_test_state_with_external_tls_cert(
    db: DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> (Arc<AppState>, Arc<JwtManager>) {
    let (state, jwt) = build_test_state_with_plugin_ops(db, tenant_id, None).await;
    let mut server = state.server.clone();
    server.has_external_tls_cert = true;
    let mut app_state = (*state).clone();
    app_state.server = server;
    (Arc::new(app_state), jwt)
}

/// Build a test [`AppState`] wired to execute `ControllerLocal` plugin
/// surface actions end to end.
///
/// The default [`build_test_state`] leaves the [`SurfaceRegistry`] empty and
/// the [`SurfaceProxy`] on its no-op local executor (every `ControllerLocal`
/// action errors unconditionally) — other tests depend on that inert
/// behaviour, so it is left untouched. This variant instead mirrors the
/// production wiring in `controller-runtime/src/boot/components.rs`:
/// bootstrap every plugin's [`surfaces::SurfaceRegistration`] into the
/// registry, then construct the proxy with a real
/// [`crate::surface_proxy::PluginSurfaceLocalExecutor`] backed by the same
/// `db` connection and `plugin_ops` catalog.
///
/// Pair with [`setup_migrated_db_with_plugins`] so plugin-owned tables exist
/// on `db` before seeding/querying them.
pub(crate) async fn build_test_state_with_plugin_surfaces(
    db: DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> (Arc<AppState>, Arc<JwtManager>) {
    let (state, jwt) = build_test_state_with_plugin_ops(db.clone(), tenant_id, None).await;

    let surface_registry = Arc::new(crate::surface_registry::SurfaceRegistry::new(
        crate::surface_registry::SurfaceRegistryConfig::default(),
    ));
    for registration in state.plugin.plugin_ops.surface_registrations() {
        surface_registry
            .bootstrap_plugin(registration)
            .expect("bootstrap plugin surfaces in test harness");
    }

    let surface_visibility: Arc<dyn crate::surface_proxy::SurfaceProviderVisibility> =
        Arc::new(crate::visibility::PluginEffectiveEnablement::new(
            Arc::clone(&state.plugin.plugin_ops),
            Arc::clone(&state.instance_plugin_snapshot),
        ));

    let surface_proxy = Arc::new(
        crate::surface_proxy::SurfaceProxy::new()
            .with_local_executor(Arc::new(
                crate::surface_proxy::PluginSurfaceLocalExecutor::new(
                    Arc::new(db),
                    Arc::clone(&state.plugin.plugin_ops),
                )
                .with_audit_emitter(state.audit_emitter.clone()),
            ))
            .with_provider_visibility(Arc::clone(&surface_visibility)),
    );

    let mut app_state = (*state).clone();
    app_state.surface_proxy_deps = crate::app_state::SurfaceProxyDeps::new(
        surface_registry,
        surface_proxy,
        surface_visibility,
    );
    (Arc::new(app_state), jwt)
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
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("keygen");
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

    let global_providers = Arc::new(crate::global_providers::GlobalProviders::new(db.clone()));
    let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> =
        plugin_ops_override.unwrap_or_else(|| {
            let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig {
                global_provider_lookup: Some(global_providers.clone()),
                ..uptrakit_plugin_infrastructure_registry::CatalogConfig::default()
            };
            Arc::new(
                uptrakit_plugin_infrastructure_registry::build_catalog(
                    &catalog_config,
                    uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
                )
                .expect("catalog should build in tests"),
            )
        });

    let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
        db.clone(),
        Arc::clone(&plugin_ops),
        "https://localhost".to_string(),
    );

    let notification = crate::app_state::NotificationState::new(
        notification_service,
        notification_dispatcher,
        crate::event_broadcaster::EventBroadcaster::new(),
    );

    let audit_db_backend: Arc<dyn uptrakit_audit_log::AuditLogBackend> =
        Arc::new(uptrakit_audit_log::DatabaseBackend::new(db.clone()));
    let audit_emitter = uptrakit_audit_log::AuditEmitter::with_backends(
        uptrakit_audit_log::AuditLogDispatcher::new(Arc::clone(&audit_db_backend)),
        Arc::clone(&audit_db_backend),
        Arc::new(uptrakit_audit_log::NoopBackend),
    );

    let update_output_broadcaster =
        crate::update_output_broadcaster::UpdateOutputBroadcaster::new();

    let update_dispatcher_for_test: Arc<dyn uptrakit_controller_core::update::UpdateDispatcher> =
        Arc::new(
            uptrakit_controller_core::update::controller::ControllerUpdateDispatcher::new(
                db.clone(),
                notification.clone(),
                Arc::new(update_output_broadcaster.clone()),
                Arc::clone(&plugin_ops),
                audit_emitter.clone(),
            ),
        );

    let (_, config_rx_for_harness) = uptrakit_config_reload::RuntimeConfigChannels::from_runtime(
        &uptrakit_config_reload::RuntimeConfig::default(),
    );

    let instance_plugin_snapshot = Arc::new(arc_swap::ArcSwap::from_pointee(
        uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot::empty(),
    ));
    let surface_visibility: Arc<dyn crate::surface_proxy::SurfaceProviderVisibility> =
        Arc::new(crate::visibility::PluginEffectiveEnablement::new(
            Arc::clone(&plugin_ops),
            Arc::clone(&instance_plugin_snapshot),
        ));

    let state = Arc::new(AppState {
        db: crate::app_state::DbState::new(db.clone()),
        cert: crate::app_state::CertState {
            ca_snapshot: ca_rx,
            ca_key_store,
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            crl_pem_cache: Arc::new(parking_lot::RwLock::new(String::new())),
            ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
        },
        auth: crate::app_state::AuthState::new(
            Arc::clone(&jwt),
            crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
            crate::auth::rate_limit::RateLimitStore::new(db.clone()),
            Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
        ),
        notification,
        broadcast: crate::app_state::BroadcastState {
            update_output_broadcaster,
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
        plugin: crate::app_state::PluginState::new(plugin_ops, global_providers),
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
        audit_emitter,
        surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
            Arc::new(crate::surface_registry::SurfaceRegistry::new(
                crate::surface_registry::SurfaceRegistryConfig::default(),
            )),
            Arc::new(crate::surface_proxy::SurfaceProxy::new()),
            surface_visibility,
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
        update_dispatcher: update_dispatcher_for_test,
        instance_plugin_snapshot,
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
        db_config_rx: config_rx_for_harness.db,
        network_config_rx: config_rx_for_harness.network,
        nats_config_rx: config_rx_for_harness.nats,
        tls_config_rx: config_rx_for_harness.tls,
        audit_config_rx: config_rx_for_harness.audit,
        log_config_rx: config_rx_for_harness.log,
        master_key_config_rx: config_rx_for_harness.master_key,
        embedded_services_config_rx: config_rx_for_harness.embedded_services,
        zeroconf_config_rx: config_rx_for_harness.zeroconf,
        oauth: crate::oauth::OAuthState::disabled(),
        config_file_state: tokio::sync::watch::channel(
            uptrakit_config_reload::ConfigFileState::default(),
        )
        .1,
        last_reload: tokio::sync::watch::channel(None).1,
        recent_reload_events: tokio::sync::watch::channel(Vec::new()).1,
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
