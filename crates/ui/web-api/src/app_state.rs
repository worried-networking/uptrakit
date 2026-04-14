use std::sync::Arc;

use axum::extract::FromRef;
use sea_orm::DatabaseConnection;
use tokio_util::sync::CancellationToken;
use uptrakit_plugin_infrastructure_registry::PluginOps;

use crate::auth::device_flow::DeviceFlowStore;
use crate::auth::jwt::JwtManager;
#[cfg(feature = "oidc")]
use crate::auth::oidc_state::{
    AccountLinkStore, OidcFlowStore, OidcRegistrationStore, OidcTokenExchangeStore,
};
use crate::auth::rate_limit::RateLimitStore;
use crate::ca_snapshot::{CaKeyStoreRef, CaSnapshotReceiver};
use crate::config_test_proxy::ConfigTestProxy;
use crate::embedded_support::EmbeddedServiceNotifier;
use crate::extension_proxy::ExtensionProxy;
use crate::extension_registry::ExtensionRegistry;
use crate::notification_service::NotificationService;
use crate::service_connections::ServiceConnectionRegistry;
use crate::settings::Settings;

/// Credential sources for building [`ServiceCredentialsPayload`] for services
/// that advertise credential capabilities (`database_access`, `nats_access`,
/// `master_key_access`).
///
/// Stored in [`AppState`] and only used by the service WebSocket handler during
/// credential delivery. The values are set at controller startup from CLI
/// arguments and environment variables.
#[derive(Clone, Default)]
pub struct ServiceCredentialSources {
    /// Database URL to provide to services with `database_access` capability.
    pub db_url: Option<String>,
    /// NATS URL to provide to services with `nats_access` capability.
    pub nats_url: Option<String>,
    /// Master key hex to provide to services with `master_key_access` capability.
    pub master_key_hex: Option<uptrakit_internal_wire::SecretString>,
}

/// Newtype wrapper for [`DatabaseConnection`] used as a focused Axum sub-state.
///
/// The inner field is private. External code accesses the connection via
/// [`DbState::db`]. Construction is restricted to within this crate via
/// [`DbState::new`], preventing external crates from constructing `DbState`
/// directly.
#[derive(Clone)]
pub struct DbState(DatabaseConnection);

impl DbState {
    pub(crate) fn new(db: DatabaseConnection) -> Self {
        Self(db)
    }

    /// Returns a reference to the underlying database connection.
    pub fn db(&self) -> &DatabaseConnection {
        &self.0
    }
}

/// Certificate-authority related state: snapshot receiver, key store, and
/// notification/cache handles for CRL and rotation operations.
#[derive(Clone)]
pub struct CertState {
    /// Watch receiver for the current CA snapshot (bundle PEM, fingerprints, etc.).
    pub ca_snapshot: CaSnapshotReceiver,
    /// Private CA key store — only for OCSP, CRL, and cert signing operations.
    pub ca_key_store: CaKeyStoreRef,
    /// Notify channel: fire after any certificate revocation to trigger CRL rebuild.
    pub revocation_notify: Arc<tokio::sync::Notify>,
    /// Cached PEM-encoded CRL bundle, updated by the CRL manager.
    pub crl_pem_cache: Arc<tokio::sync::RwLock<String>>,
    /// Trigger for immediate CA rotation (fired by the rotate-ca API endpoint).
    pub ca_rotation_trigger: Arc<tokio::sync::Notify>,
}

/// Authentication state: JWT manager, device/OIDC flow stores, rate limiter,
/// and token denylist.
#[derive(Clone)]
pub struct AuthState {
    /// JWT signing/validation manager for access tokens.
    pub jwt: Arc<JwtManager>,
    /// Database-backed store for pending device authorization flows.
    pub device_flow_store: DeviceFlowStore,
    /// Database-backed rate limiter for public authentication endpoints.
    pub rate_limit_store: RateLimitStore,
    /// In-memory denylist for immediate JWT access token revocation.
    pub token_denylist: Arc<crate::auth::token_denylist::TokenDenylist>,
}

/// Real-time broadcast channels for SSE event delivery.
#[derive(Clone)]
pub struct BroadcastState {
    /// Per-device-flow broadcast channels for real-time device auth SSE delivery.
    pub device_flow_broadcaster: crate::device_flow_broadcaster::DeviceFlowBroadcaster,
    /// Per-update broadcast channels for real-time output streaming via SSE.
    pub update_output_broadcaster: crate::update_output_broadcaster::UpdateOutputBroadcaster,
    /// Per-batch broadcast channels for real-time batch progress streaming via SSE.
    pub batch_progress_broadcaster: crate::batch_progress_broadcaster::BatchProgressBroadcaster,
}

/// Notification side-effect state used by mutation actions.
#[derive(Clone)]
pub struct NotificationState {
    /// Cross-controller notification service for push message delivery via outbox pattern.
    pub notification_service: NotificationService,
    /// Notification dispatcher for fire-and-forget event delivery.
    pub notification_dispatcher: crate::notifications::dispatcher::NotificationDispatcher,
    /// Per-tenant broadcast channels for real-time admin event SSE delivery.
    pub event_broadcaster: crate::event_broadcaster::EventBroadcaster,
}

impl NotificationState {
    /// Returns a [`MutationContext`] borrowing the common notification side-effect handles.
    pub(crate) fn mutation_context(&self) -> crate::actions::MutationContext<'_> {
        crate::actions::MutationContext {
            notification_service: &self.notification_service,
            notification_dispatcher: &self.notification_dispatcher,
            event_broadcaster: &self.event_broadcaster,
        }
    }
}

/// Focused Axum sub-state for plugin registry operations.
#[derive(Clone)]
pub struct PluginOpsState(pub Arc<dyn PluginOps>);

/// OIDC-specific flow stores (only compiled when the `oidc` feature is active).
#[cfg(feature = "oidc")]
#[derive(Clone)]
pub struct OidcState {
    /// Database-backed store for pending OIDC authorization flows.
    pub oidc_flow_store: OidcFlowStore,
    /// Database-backed store for pending OIDC account links.
    pub account_link_store: AccountLinkStore,
    /// Database-backed store for pending OIDC token exchanges.
    pub oidc_token_exchange_store: OidcTokenExchangeStore,
    /// Database-backed store for pending OIDC registrations (token-gated).
    pub oidc_registration_store: OidcRegistrationStore,
}

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    /// Database connection pool wrapped in [`DbState`] for focused sub-state extraction.
    pub(crate) db: DbState,
    /// Certificate-authority state: snapshot, key store, revocation, CRL, rotation.
    pub cert: CertState,
    /// Authentication state: JWT, device flow, rate limiting, token denylist.
    pub auth: AuthState,
    /// Notification side-effect state used by mutations.
    pub notification: NotificationState,
    /// Real-time broadcast channels for SSE delivery.
    pub broadcast: BroadcastState,
    /// OIDC flow stores (feature-gated).
    #[cfg(feature = "oidc")]
    pub oidc: OidcState,
    /// Application settings catalogue (includes network settings).
    pub settings: Settings,
    /// Agent certificate signer for mTLS enrollment.
    pub cert_signer: Arc<dyn crate::cert_signer::AgentCertSigner>,
    /// Unified registry of connected services (agents and MQTT) for push notifications.
    pub service_connections: ServiceConnectionRegistry,
    /// Plugin operations abstraction used by plugin-config route handlers.
    ///
    /// Injected via `Arc<dyn PluginOps>` so that route handlers and query
    /// helpers are decoupled from the concrete [`uptrakit_plugin_infrastructure_registry::PluginCatalog`]
    /// and can be tested with a mock implementation.
    pub plugin_ops: Arc<dyn PluginOps>,
    /// Credential sources for services with credential capabilities.
    pub credential_sources: ServiceCredentialSources,
    /// Cancellation token signalled during server shutdown to terminate open SSE streams.
    ///
    /// SSE handler loops `tokio::select!` on this token so that in-flight streams exit
    /// cleanly when axum initiates a graceful shutdown rather than blocking indefinitely
    /// on a broadcast channel that may never close.
    pub shutdown_token: CancellationToken,
    /// Notifier for the embedded service infrastructure. Set by the controller
    /// when embedded services are active; `None` when no services are embedded.
    ///
    /// The WS handler calls methods on this trait at service connect/disconnect
    /// points so that embedded services can yield to external counterparts.
    pub embedded_service_notifier: Option<Arc<dyn EmbeddedServiceNotifier>>,
    /// Audit log filter (global mode, per-tenant overrides checked at log time).
    pub audit_log_filter: uptrakit_audit_log::AuditFilter,
    /// Audit log dispatcher for fire-and-forget entry persistence.
    pub audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher,
    /// Registry tracking which plugin/service extensions are available.
    pub extension_registry: Arc<ExtensionRegistry>,
    /// Request/response proxy for extension action invocations.
    pub extension_proxy: Arc<ExtensionProxy>,
    /// Request/response proxy for plugin configuration test invocations.
    pub config_test_proxy: Arc<ConfigTestProxy>,
    /// Path to the PKI directory (for server cert renewal).
    pub pki_path: std::path::PathBuf,
    /// RustlsConfig handle for hot-reloading TLS.
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    /// UUID of the default (seeded) tenant. Used as fallback when no tenant header is present.
    pub default_tenant_id: uuid::Uuid,
    /// Unique identifier for this controller instance (used for cross-controller notification delivery).
    pub controller_id: uuid::Uuid,
    /// Global workload claim registry for exclusive config-key ownership.
    ///
    /// Services with the `WorkloadClaims` capability send `WorkloadClaim` to
    /// request exclusive ownership of config keys. The registry tracks grants
    /// and derives tenant routing indexes for SoftwareStates/HostConnectivity
    /// delivery.
    pub workload_claim_registry: Arc<crate::workload_claims::WorkloadClaimRegistry>,
    /// When `true` (default), plugin config create/update requests that contain
    /// dangerous command patterns (e.g. `curl|bash`, `rm -rf /`) are rejected
    /// with HTTP 400. Set to `false` via `--allow-dangerous-commands` CLI flag
    /// to downgrade to advisory-only warnings.
    pub reject_dangerous_commands: bool,
    /// Registry of active interactive update sessions (single-writer enforcement).
    #[cfg(feature = "interactive")]
    pub interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry,
}

/// Error returned when [`AppStateBuilder::build`] is called with a missing required field.
#[derive(Debug)]
pub struct AppStateBuildError(pub &'static str);

impl std::fmt::Display for AppStateBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "missing required AppState field: {}", self.0)
    }
}

impl std::error::Error for AppStateBuildError {}

/// Incremental builder for [`AppState`].
///
/// Obtain a builder via [`AppState::builder`], call each setter exactly once
/// (all fields are required), then call [`AppStateBuilder::build`].
pub struct AppStateBuilder {
    ca_snapshot: Option<CaSnapshotReceiver>,
    ca_key_store: Option<CaKeyStoreRef>,
    db: Option<DatabaseConnection>,
    settings: Option<Settings>,
    cert_signer: Option<Arc<dyn crate::cert_signer::AgentCertSigner>>,
    service_connections: Option<ServiceConnectionRegistry>,
    revocation_notify: Option<Arc<tokio::sync::Notify>>,
    #[cfg(feature = "oidc")]
    oidc_flow_store: Option<OidcFlowStore>,
    #[cfg(feature = "oidc")]
    account_link_store: Option<AccountLinkStore>,
    jwt: Option<Arc<JwtManager>>,
    #[cfg(feature = "oidc")]
    oidc_token_exchange_store: Option<OidcTokenExchangeStore>,
    #[cfg(feature = "oidc")]
    oidc_registration_store: Option<OidcRegistrationStore>,
    device_flow_store: Option<DeviceFlowStore>,
    rate_limit_store: Option<RateLimitStore>,
    pki_path: Option<std::path::PathBuf>,
    rustls_config: Option<axum_server::tls_rustls::RustlsConfig>,
    crl_pem_cache: Option<Arc<tokio::sync::RwLock<String>>>,
    ca_rotation_trigger: Option<Arc<tokio::sync::Notify>>,
    default_tenant_id: Option<uuid::Uuid>,
    controller_id: Option<uuid::Uuid>,
    notification_service: Option<NotificationService>,
    notification_dispatcher: Option<crate::notifications::dispatcher::NotificationDispatcher>,
    token_denylist: Option<Arc<crate::auth::token_denylist::TokenDenylist>>,
    plugin_ops: Option<Arc<dyn PluginOps>>,
    credential_sources: Option<ServiceCredentialSources>,
    event_broadcaster: Option<crate::event_broadcaster::EventBroadcaster>,
    device_flow_broadcaster: Option<crate::device_flow_broadcaster::DeviceFlowBroadcaster>,
    update_output_broadcaster: Option<crate::update_output_broadcaster::UpdateOutputBroadcaster>,
    batch_progress_broadcaster: Option<crate::batch_progress_broadcaster::BatchProgressBroadcaster>,
    shutdown_token: Option<CancellationToken>,
    embedded_service_notifier: Option<Arc<dyn EmbeddedServiceNotifier>>,
    audit_log_filter: Option<uptrakit_audit_log::AuditFilter>,
    audit_log_dispatcher: Option<uptrakit_audit_log::AuditLogDispatcher>,
    extension_registry: Option<Arc<ExtensionRegistry>>,
    extension_proxy: Option<Arc<ExtensionProxy>>,
    config_test_proxy: Option<Arc<ConfigTestProxy>>,
    workload_claim_registry: Option<Arc<crate::workload_claims::WorkloadClaimRegistry>>,
    reject_dangerous_commands: bool,
}

impl AppStateBuilder {
    fn new() -> Self {
        Self {
            ca_snapshot: None,
            ca_key_store: None,
            db: None,
            settings: None,
            cert_signer: None,
            service_connections: None,
            revocation_notify: None,
            #[cfg(feature = "oidc")]
            oidc_flow_store: None,
            #[cfg(feature = "oidc")]
            account_link_store: None,
            jwt: None,
            #[cfg(feature = "oidc")]
            oidc_token_exchange_store: None,
            #[cfg(feature = "oidc")]
            oidc_registration_store: None,
            device_flow_store: None,
            rate_limit_store: None,
            pki_path: None,
            rustls_config: None,
            crl_pem_cache: None,
            ca_rotation_trigger: None,
            default_tenant_id: None,
            controller_id: None,
            notification_service: None,
            notification_dispatcher: None,
            token_denylist: None,
            plugin_ops: None,
            credential_sources: None,
            event_broadcaster: None,
            device_flow_broadcaster: None,
            update_output_broadcaster: None,
            batch_progress_broadcaster: None,
            shutdown_token: None,
            embedded_service_notifier: None,
            audit_log_filter: None,
            audit_log_dispatcher: None,
            extension_registry: None,
            extension_proxy: None,
            config_test_proxy: None,
            workload_claim_registry: None,
            reject_dangerous_commands: false,
        }
    }

    pub fn ca_snapshot(mut self, v: CaSnapshotReceiver) -> Self {
        self.ca_snapshot = Some(v);
        self
    }

    pub fn ca_key_store(mut self, v: CaKeyStoreRef) -> Self {
        self.ca_key_store = Some(v);
        self
    }

    /// Set the database connection. Accessible via [`AppState::db`] after build.
    pub fn db(mut self, v: DatabaseConnection) -> Self {
        self.db = Some(v);
        self
    }

    pub fn settings(mut self, v: Settings) -> Self {
        self.settings = Some(v);
        self
    }

    pub fn cert_signer(mut self, v: Arc<dyn crate::cert_signer::AgentCertSigner>) -> Self {
        self.cert_signer = Some(v);
        self
    }

    pub fn service_connections(mut self, v: ServiceConnectionRegistry) -> Self {
        self.service_connections = Some(v);
        self
    }

    pub fn revocation_notify(mut self, v: Arc<tokio::sync::Notify>) -> Self {
        self.revocation_notify = Some(v);
        self
    }

    #[cfg(feature = "oidc")]
    pub fn oidc_flow_store(mut self, v: OidcFlowStore) -> Self {
        self.oidc_flow_store = Some(v);
        self
    }

    #[cfg(feature = "oidc")]
    pub fn account_link_store(mut self, v: AccountLinkStore) -> Self {
        self.account_link_store = Some(v);
        self
    }

    pub fn jwt(mut self, v: Arc<JwtManager>) -> Self {
        self.jwt = Some(v);
        self
    }

    #[cfg(feature = "oidc")]
    pub fn oidc_token_exchange_store(mut self, v: OidcTokenExchangeStore) -> Self {
        self.oidc_token_exchange_store = Some(v);
        self
    }

    #[cfg(feature = "oidc")]
    pub fn oidc_registration_store(mut self, v: OidcRegistrationStore) -> Self {
        self.oidc_registration_store = Some(v);
        self
    }

    pub fn device_flow_store(mut self, v: DeviceFlowStore) -> Self {
        self.device_flow_store = Some(v);
        self
    }

    pub fn rate_limit_store(mut self, v: RateLimitStore) -> Self {
        self.rate_limit_store = Some(v);
        self
    }

    pub fn pki_path(mut self, v: std::path::PathBuf) -> Self {
        self.pki_path = Some(v);
        self
    }

    pub fn rustls_config(mut self, v: axum_server::tls_rustls::RustlsConfig) -> Self {
        self.rustls_config = Some(v);
        self
    }

    pub fn crl_pem_cache(mut self, v: Arc<tokio::sync::RwLock<String>>) -> Self {
        self.crl_pem_cache = Some(v);
        self
    }

    pub fn ca_rotation_trigger(mut self, v: Arc<tokio::sync::Notify>) -> Self {
        self.ca_rotation_trigger = Some(v);
        self
    }

    pub fn default_tenant_id(mut self, v: uuid::Uuid) -> Self {
        self.default_tenant_id = Some(v);
        self
    }

    pub fn controller_id(mut self, v: uuid::Uuid) -> Self {
        self.controller_id = Some(v);
        self
    }

    pub fn notification_service(mut self, v: NotificationService) -> Self {
        self.notification_service = Some(v);
        self
    }

    pub fn notification_dispatcher(
        mut self,
        v: crate::notifications::dispatcher::NotificationDispatcher,
    ) -> Self {
        self.notification_dispatcher = Some(v);
        self
    }

    pub fn token_denylist(mut self, v: Arc<crate::auth::token_denylist::TokenDenylist>) -> Self {
        self.token_denylist = Some(v);
        self
    }

    /// Override the plugin operations implementation.
    ///
    /// Defaults to [`uptrakit_plugin_infrastructure_registry::PluginCatalog`] when not set.
    /// Use this in tests to inject a mock implementation.
    pub fn plugin_ops(mut self, v: Arc<dyn PluginOps>) -> Self {
        self.plugin_ops = Some(v);
        self
    }

    /// Set the credential sources for services with credential capabilities.
    ///
    /// Optional — defaults to empty sources (no credentials delivered).
    pub fn credential_sources(mut self, v: ServiceCredentialSources) -> Self {
        self.credential_sources = Some(v);
        self
    }

    /// Override the admin event broadcaster.
    ///
    /// Optional — defaults to [`EventBroadcaster::new()`] (single-instance mode,
    /// no NATS).  Pass a broadcaster built with [`EventBroadcaster::with_nats`] to
    /// enable cross-controller SSE fan-out when NATS is configured.
    pub fn event_broadcaster(mut self, v: crate::event_broadcaster::EventBroadcaster) -> Self {
        self.event_broadcaster = Some(v);
        self
    }

    /// Override the batch progress broadcaster.
    ///
    /// Optional — defaults to [`BatchProgressBroadcaster::new()`] (single-instance
    /// mode with no NATS).  Pass a broadcaster built with
    /// [`BatchProgressBroadcaster::with_nats`] to enable cross-instance SSE
    /// delivery when NATS is configured.
    pub fn batch_progress_broadcaster(
        mut self,
        v: crate::batch_progress_broadcaster::BatchProgressBroadcaster,
    ) -> Self {
        self.batch_progress_broadcaster = Some(v);
        self
    }

    /// Set the shutdown cancellation token for SSE stream graceful termination.
    ///
    /// Optional — defaults to a new, never-cancelled token (no-op for tests
    /// or deployments that rely on broadcast channel close for termination).
    /// Production controller should pass the shared token that is cancelled
    /// during graceful shutdown so that SSE streams exit cleanly.
    pub fn shutdown_token(mut self, v: CancellationToken) -> Self {
        self.shutdown_token = Some(v);
        self
    }

    /// Set the embedded service notifier.
    ///
    /// Optional — defaults to `None` (no embedded services). When set, the WS
    /// handler calls trait methods at service connect/disconnect points so that
    /// embedded services can yield to external counterparts.
    pub fn embedded_service_notifier(mut self, v: Arc<dyn EmbeddedServiceNotifier>) -> Self {
        self.embedded_service_notifier = Some(v);
        self
    }

    /// Set the audit log filter mode.
    ///
    /// Optional — defaults to `AuditFilter::default()` (mode = All).
    pub fn audit_log_filter(mut self, v: uptrakit_audit_log::AuditFilter) -> Self {
        self.audit_log_filter = Some(v);
        self
    }

    /// Set the audit log dispatcher.
    ///
    /// Optional — defaults to a dispatcher backed by `NoopBackend`.
    pub fn audit_log_dispatcher(mut self, v: uptrakit_audit_log::AuditLogDispatcher) -> Self {
        self.audit_log_dispatcher = Some(v);
        self
    }

    /// Override the extension registry.
    ///
    /// Optional — defaults to an empty registry (no plugin extensions).
    /// The registry is initialized from `plugin_ops.extension_manifests()`.
    pub fn extension_registry(mut self, v: Arc<ExtensionRegistry>) -> Self {
        self.extension_registry = Some(v);
        self
    }

    /// Override the extension proxy.
    ///
    /// Optional — defaults to an empty proxy with no pending requests.
    pub fn extension_proxy(mut self, v: Arc<ExtensionProxy>) -> Self {
        self.extension_proxy = Some(v);
        self
    }

    /// Override the config test proxy.
    ///
    /// Optional — defaults to an empty proxy with no pending requests.
    pub fn config_test_proxy(mut self, v: Arc<ConfigTestProxy>) -> Self {
        self.config_test_proxy = Some(v);
        self
    }

    /// Set the workload claim registry for exclusive config-key ownership.
    pub fn workload_claim_registry(
        mut self,
        v: Arc<crate::workload_claims::WorkloadClaimRegistry>,
    ) -> Self {
        self.workload_claim_registry = Some(v);
        self
    }

    /// Enable dangerous command pattern rejection.
    ///
    /// When `true`, plugin config create/update requests containing dangerous
    /// patterns (e.g. `curl|bash`, `rm -rf /`) are rejected with HTTP 400.
    /// Default: `false` (advisory-only warnings).
    pub fn reject_dangerous_commands(mut self, v: bool) -> Self {
        self.reject_dangerous_commands = v;
        self
    }

    /// Consume the builder and produce an [`AppState`].
    ///
    /// Returns [`AppStateBuildError`] naming the first field that was not set.
    ///
    /// # Errors
    ///
    /// Returns an error if any required field was not set before calling `build`.
    pub fn build(self) -> Result<AppState, AppStateBuildError> {
        Ok(AppState {
            db: DbState::new(self.db.ok_or(AppStateBuildError("db"))?),
            cert: CertState {
                ca_snapshot: self.ca_snapshot.ok_or(AppStateBuildError("ca_snapshot"))?,
                ca_key_store: self
                    .ca_key_store
                    .ok_or(AppStateBuildError("ca_key_store"))?,
                revocation_notify: self
                    .revocation_notify
                    .ok_or(AppStateBuildError("revocation_notify"))?,
                crl_pem_cache: self
                    .crl_pem_cache
                    .ok_or(AppStateBuildError("crl_pem_cache"))?,
                ca_rotation_trigger: self
                    .ca_rotation_trigger
                    .ok_or(AppStateBuildError("ca_rotation_trigger"))?,
            },
            auth: AuthState {
                jwt: self.jwt.ok_or(AppStateBuildError("jwt"))?,
                device_flow_store: self
                    .device_flow_store
                    .ok_or(AppStateBuildError("device_flow_store"))?,
                rate_limit_store: self
                    .rate_limit_store
                    .ok_or(AppStateBuildError("rate_limit_store"))?,
                token_denylist: self
                    .token_denylist
                    .ok_or(AppStateBuildError("token_denylist"))?,
            },
            notification: NotificationState {
                notification_service: self
                    .notification_service
                    .ok_or(AppStateBuildError("notification_service"))?,
                notification_dispatcher: self
                    .notification_dispatcher
                    .ok_or(AppStateBuildError("notification_dispatcher"))?,
                event_broadcaster: self.event_broadcaster.unwrap_or_default(),
            },
            broadcast: BroadcastState {
                device_flow_broadcaster: self.device_flow_broadcaster.unwrap_or_default(),
                update_output_broadcaster: self.update_output_broadcaster.unwrap_or_default(),
                batch_progress_broadcaster: self.batch_progress_broadcaster.unwrap_or_default(),
            },
            #[cfg(feature = "oidc")]
            oidc: OidcState {
                oidc_flow_store: self
                    .oidc_flow_store
                    .ok_or(AppStateBuildError("oidc_flow_store"))?,
                account_link_store: self
                    .account_link_store
                    .ok_or(AppStateBuildError("account_link_store"))?,
                oidc_token_exchange_store: self
                    .oidc_token_exchange_store
                    .ok_or(AppStateBuildError("oidc_token_exchange_store"))?,
                oidc_registration_store: self
                    .oidc_registration_store
                    .ok_or(AppStateBuildError("oidc_registration_store"))?,
            },
            settings: self.settings.ok_or(AppStateBuildError("settings"))?,
            cert_signer: self.cert_signer.ok_or(AppStateBuildError("cert_signer"))?,
            service_connections: self
                .service_connections
                .ok_or(AppStateBuildError("service_connections"))?,
            plugin_ops: self.plugin_ops.unwrap_or_else(|| {
                Arc::new(
                    uptrakit_plugin_infrastructure_registry::build_catalog(
                        &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
                    )
                    .expect("default catalog should build"),
                )
            }),
            credential_sources: self.credential_sources.unwrap_or_default(),
            shutdown_token: self.shutdown_token.unwrap_or_default(),
            embedded_service_notifier: self.embedded_service_notifier,
            audit_log_filter: self.audit_log_filter.unwrap_or_default(),
            audit_log_dispatcher: self.audit_log_dispatcher.unwrap_or_else(|| {
                uptrakit_audit_log::AuditLogDispatcher::new(std::sync::Arc::new(
                    uptrakit_audit_log::NoopBackend,
                ))
            }),
            extension_registry: self
                .extension_registry
                .unwrap_or_else(|| Arc::new(ExtensionRegistry::new(vec![]))),
            extension_proxy: self
                .extension_proxy
                .unwrap_or_else(|| Arc::new(ExtensionProxy::new())),
            config_test_proxy: self
                .config_test_proxy
                .unwrap_or_else(|| Arc::new(ConfigTestProxy::new())),
            pki_path: self.pki_path.ok_or(AppStateBuildError("pki_path"))?,
            rustls_config: self
                .rustls_config
                .ok_or(AppStateBuildError("rustls_config"))?,
            default_tenant_id: self
                .default_tenant_id
                .ok_or(AppStateBuildError("default_tenant_id"))?,
            controller_id: self
                .controller_id
                .ok_or(AppStateBuildError("controller_id"))?,
            workload_claim_registry: self
                .workload_claim_registry
                .unwrap_or_else(|| Arc::new(crate::workload_claims::WorkloadClaimRegistry::new())),
            reject_dangerous_commands: self.reject_dangerous_commands,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
        })
    }
}

impl AppState {
    /// Create a new builder for [`AppState`].
    ///
    /// This is the only public way to construct an `AppState` from outside the
    /// crate, preserving the `db` field encapsulation enforced by [`AppState::db`].
    pub fn builder() -> AppStateBuilder {
        AppStateBuilder::new()
    }

    /// Returns a reference to the underlying database connection.
    pub fn db(&self) -> &DatabaseConnection {
        self.db.db()
    }

    /// Returns a [`MutationContext`] borrowing the three common side-effect
    /// handles from this `AppState`. Pass it to action functions together with
    /// any domain-specific handles.
    pub(crate) fn mutation_context(&self) -> crate::actions::MutationContext<'_> {
        self.notification.mutation_context()
    }
}

impl FromRef<Arc<AppState>> for DbState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        state.db.clone()
    }
}

impl FromRef<Arc<AppState>> for CertState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        state.cert.clone()
    }
}

impl FromRef<Arc<AppState>> for AuthState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        state.auth.clone()
    }
}

impl FromRef<Arc<AppState>> for NotificationState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        state.notification.clone()
    }
}

impl FromRef<Arc<AppState>> for BroadcastState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        state.broadcast.clone()
    }
}

impl FromRef<Arc<AppState>> for PluginOpsState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        PluginOpsState(state.plugin_ops.clone())
    }
}

#[cfg(feature = "oidc")]
impl FromRef<Arc<AppState>> for OidcState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        state.oidc.clone()
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod from_ref_tests {
    use super::*;
    use axum::extract::FromRef;

    async fn test_app_state() -> Arc<AppState> {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        state
    }

    #[tokio::test]
    async fn extract_db_state() {
        let state = test_app_state().await;
        let db_state = DbState::from_ref(&state);
        let _db: &DatabaseConnection = db_state.db();
    }

    #[tokio::test]
    async fn extract_auth_state() {
        let state = test_app_state().await;
        let auth = AuthState::from_ref(&state);
        let _jwt = &auth.jwt;
    }

    #[tokio::test]
    async fn extract_notification_state() {
        let state = test_app_state().await;
        let notification = NotificationState::from_ref(&state);
        let _service = &notification.notification_service;
    }

    #[tokio::test]
    async fn extract_broadcast_state() {
        let state = test_app_state().await;
        let broadcast = BroadcastState::from_ref(&state);
        let _batch = &broadcast.batch_progress_broadcaster;
    }

    #[tokio::test]
    async fn app_state_mutation_context_delegates_to_notification_state() {
        let state = test_app_state().await;
        let app_ctx = state.mutation_context();
        let notification_ctx = state.notification.mutation_context();

        assert!(std::ptr::eq(
            app_ctx.notification_service,
            notification_ctx.notification_service,
        ));
        assert!(std::ptr::eq(
            app_ctx.notification_dispatcher,
            notification_ctx.notification_dispatcher,
        ));
        assert!(std::ptr::eq(
            app_ctx.event_broadcaster,
            notification_ctx.event_broadcaster,
        ));
    }

    #[tokio::test]
    async fn extract_cert_state() {
        let state = test_app_state().await;
        let cert = CertState::from_ref(&state);
        let _cache = &cert.crl_pem_cache;
    }

    #[cfg(feature = "oidc")]
    #[tokio::test]
    async fn extract_oidc_state() {
        let state = test_app_state().await;
        let oidc = OidcState::from_ref(&state);
        let _store = &oidc.oidc_flow_store;
    }
}
