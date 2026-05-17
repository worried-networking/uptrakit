use std::sync::Arc;

use rootcause::prelude::*;

use axum::extract::FromRef;
use sea_orm::DatabaseConnection;
use tokio_util::sync::CancellationToken;
use uptrakit_controller_core::db::DbStateSource;
use uptrakit_plugin_infrastructure_registry::{ControllerUpdateProtection, PluginOps};

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
use crate::notification_service::NotificationService;
use crate::service_connections::ServiceConnectionRegistry;
use crate::settings::Settings;
use crate::surface_proxy::SurfaceProxy;
use crate::surface_registry::SurfaceRegistry;

/// Grouped surface-proxy dependencies stored in [`AppState`].
///
/// Bundles the surface registry (provider catalog) and the surface proxy
/// (in-flight request/response tracker) so they can be replaced together in
/// tests and accessed through a single named sub-field.
#[derive(Clone)]
#[non_exhaustive]
pub struct SurfaceProxyDeps {
    /// Registry tracking normalized surface contracts from built-ins/services.
    pub registry: Arc<SurfaceRegistry>,
    /// Request/response proxy for surface interaction invocations.
    pub proxy: Arc<SurfaceProxy>,
}

impl SurfaceProxyDeps {
    /// Creates a new [`SurfaceProxyDeps`] from a registry and proxy.
    #[must_use]
    pub fn new(registry: Arc<SurfaceRegistry>, proxy: Arc<SurfaceProxy>) -> Self {
        Self { registry, proxy }
    }
}

/// Credential sources for building [`ServiceCredentialsPayload`] for services
/// that advertise credential capabilities (`database_access`, `nats_access`,
/// `master_key_access`).
///
/// Stored in [`AppState`] and only used by the service WebSocket handler during
/// credential delivery. The values are set at controller startup from CLI
/// arguments and environment variables.
#[non_exhaustive]
#[derive(Clone, Default)]
pub struct ServiceCredentialSources {
    /// Database URL to provide to services with `database_access` capability.
    pub db_url: Option<String>,
    /// NATS URL to provide to services with `nats_access` capability.
    pub nats_url: Option<String>,
    /// Master key hex to provide to services with `master_key_access` capability.
    pub master_key_hex: Option<uptrakit_wire::SecretString>,
}

impl ServiceCredentialSources {
    /// Creates a new [`ServiceCredentialSources`] with all fields explicit.
    pub fn new(
        db_url: Option<String>,
        nats_url: Option<String>,
        master_key_hex: Option<uptrakit_wire::SecretString>,
    ) -> Self {
        Self {
            db_url,
            nats_url,
            master_key_hex,
        }
    }
}

pub use uptrakit_controller_core::db::DbState;

/// Certificate-authority related state: snapshot receiver, key store, and
/// notification/cache handles for CRL and rotation operations.
#[non_exhaustive]
#[derive(Clone)]
pub struct CertState {
    /// Watch receiver for the current CA snapshot (bundle PEM, fingerprints, etc.).
    pub ca_snapshot: CaSnapshotReceiver,
    /// Private CA key store — only for OCSP, CRL, and cert signing operations.
    pub ca_key_store: CaKeyStoreRef,
    /// Notify channel: fire after any certificate revocation to trigger CRL rebuild.
    pub revocation_notify: Arc<tokio::sync::Notify>,
    /// Cached PEM-encoded CRL bundle, updated by the CRL manager.
    pub crl_pem_cache: Arc<parking_lot::RwLock<String>>,
    /// Trigger for immediate CA rotation (fired by the rotate-ca API endpoint).
    pub ca_rotation_trigger: Arc<tokio::sync::Notify>,
}

pub use uptrakit_controller_core::auth::{
    AuthFailure, AuthState, AuthenticatedApiTokenId, AuthenticatedUser,
};

/// Real-time broadcast channels for SSE event delivery.
#[non_exhaustive]
#[derive(Clone)]
pub struct BroadcastState {
    /// Per-update broadcast channels for real-time output streaming via SSE.
    pub update_output_broadcaster: crate::update_output_broadcaster::UpdateOutputBroadcaster,
    /// Per-batch broadcast channels for real-time batch progress streaming via SSE.
    pub batch_progress_broadcaster: crate::batch_progress_broadcaster::BatchProgressBroadcaster,
}

pub use uptrakit_controller_core::notification::NotificationState;

pub(crate) trait NotificationStateMutationExt {
    fn mutation_context(&self) -> crate::actions::MutationContext<'_>;
}

impl NotificationStateMutationExt for NotificationState {
    fn mutation_context(&self) -> crate::actions::MutationContext<'_> {
        crate::actions::MutationContext {
            notification_service: &self.notification_service,
            event_broadcaster: &self.event_broadcaster,
        }
    }
}

/// Focused Axum sub-state for audit log emission.
#[derive(Clone)]
pub struct AuditEmitterState(pub uptrakit_audit_log::AuditEmitter);

/// Focused Axum sub-state for plugin registry operations.
#[derive(Clone)]
pub struct PluginOpsState(pub Arc<dyn PluginOps>);

/// Focused Axum sub-state for host-owned global provider runtimes.
#[derive(Clone)]
pub struct GlobalProvidersState(pub Arc<crate::global_providers::GlobalProviders>);

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

/// Grouped TLS server configuration for hot-reload.
///
/// `#[non_exhaustive]`: fields may be added (e.g. OCSP stapling config).
#[non_exhaustive]
#[derive(Clone)]
pub struct ServerState {
    /// Path to the PKI directory (for server cert renewal).
    pub pki_path: std::path::PathBuf,
    /// RustlsConfig handle for hot-reloading TLS.
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    /// Optional hot-swap handle for server cert renewal.
    ///
    /// When set, certificate renewal swaps the resolver atomically instead of
    /// reloading the full TLS config.  `None` when the controller was started
    /// without the dynamic-resolver path (e.g. in tests).
    pub server_cert_resolver: Option<Arc<dyn crate::server_cert_swap::ServerCertSwap>>,
}

impl ServerState {
    /// Creates a new [`ServerState`] without a hot-swap resolver.
    ///
    /// Used by tests and callers that do not need zero-downtime cert swaps.
    #[must_use]
    pub fn new(
        pki_path: std::path::PathBuf,
        rustls_config: axum_server::tls_rustls::RustlsConfig,
    ) -> Self {
        Self {
            pki_path,
            rustls_config,
            server_cert_resolver: None,
        }
    }

    /// Creates a new [`ServerState`] with a hot-swap resolver handle.
    ///
    /// The resolver is called during server certificate renewal to atomically
    /// replace the [`CertifiedKey`] without rebuilding the full TLS config.
    #[must_use]
    pub fn with_cert_resolver(
        pki_path: std::path::PathBuf,
        rustls_config: axum_server::tls_rustls::RustlsConfig,
        resolver: Arc<dyn crate::server_cert_swap::ServerCertSwap>,
    ) -> Self {
        Self {
            pki_path,
            rustls_config,
            server_cert_resolver: Some(resolver),
        }
    }
}

/// Grouped plugin-ops state for plugin configuration and global provider runtimes.
///
/// `#[non_exhaustive]`: fields may be added (e.g. plugin metrics registry).
#[non_exhaustive]
#[derive(Clone)]
pub struct PluginState {
    /// Plugin operations abstraction used by plugin-config route handlers.
    pub plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps>,
    /// Host-owned global provider runtimes shared by singleton plugins.
    pub global_providers: Arc<crate::global_providers::GlobalProviders>,
}

impl PluginState {
    /// Creates a new [`PluginState`].
    #[must_use]
    pub fn new(
        plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps>,
        global_providers: Arc<crate::global_providers::GlobalProviders>,
    ) -> Self {
        Self {
            plugin_ops,
            global_providers,
        }
    }
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
    /// Grouped plugin operations and global provider runtimes.
    pub plugin: PluginState,
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
    /// Live audit config receiver — updated by [`AuditDispatcherReloadable`] on every
    /// config reload cycle. Route handlers read `(*state.audit_log_filter_rx.borrow()).filter`
    /// instead of a stale snapshot captured at boot.
    pub audit_log_filter_rx:
        tokio::sync::watch::Receiver<std::sync::Arc<uptrakit_config_reload::config::AuditConfig>>,
    /// Audit log dispatcher for fire-and-forget entry persistence.
    pub audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher,
    /// Audit emitter used by semantic producers.
    pub audit_emitter: uptrakit_audit_log::AuditEmitter,
    /// Grouped surface-proxy dependencies (registry + proxy).
    pub surface_proxy_deps: SurfaceProxyDeps,
    /// Request/response proxy for plugin configuration test invocations.
    pub config_test_proxy: Arc<ConfigTestProxy>,
    /// Grouped TLS server configuration for hot-reload.
    pub server: ServerState,
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
    /// Notified by `POST /test/force-reexec` to trigger an unconditional reexec.
    /// `None` when `UPTRAKIT_TEST_UTILS_ENABLED` is not `"true"` at startup.
    #[cfg(feature = "test-utils")]
    pub(crate) test_reexec_notify: Option<Arc<tokio::sync::Notify>>,
    /// Update dispatcher: runs pre-update protection then dispatches to the agent.
    ///
    /// Defaults to [`ControllerUpdateDispatcher`] wired from the state's own fields.
    /// Override via [`AppStateBuilder::update_dispatcher`] to inject a test double.
    pub update_dispatcher: Arc<dyn uptrakit_controller_core::update::UpdateDispatcher>,
    /// Snapshot of instance-scoped plugin enable state and configuration.
    ///
    /// Loaded once at boot from `instance_plugin_setting` and updated atomically
    /// on every upsert. Uses `ArcSwap` for lock-free reads on the hot path.
    pub instance_plugin_snapshot: Arc<
        arc_swap::ArcSwap<
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot,
        >,
    >,
    /// Reload coordinator handle for state introspection (Plan 3 endpoint reads this).
    pub coordinator_handle: uptrakit_config_reload::ReloadCoordinatorHandle,
    /// Settings-version counter cache — read by the IfMatch extractor in Plan 3.
    pub settings_version_cache: uptrakit_config_reload::SettingsVersionCache,
    /// Per-section watch receivers seeded at boot from TOML. Plan 2 Reloadables publish updates.
    pub db_config_rx:
        tokio::sync::watch::Receiver<std::sync::Arc<uptrakit_config_reload::config::DbConfig>>,
    /// Network config watch receiver.
    pub network_config_rx:
        tokio::sync::watch::Receiver<std::sync::Arc<uptrakit_config_reload::config::NetworkConfig>>,
    /// NATS config watch receiver.
    pub nats_config_rx:
        tokio::sync::watch::Receiver<std::sync::Arc<uptrakit_config_reload::config::NatsConfig>>,
    /// TLS config watch receiver.
    pub tls_config_rx:
        tokio::sync::watch::Receiver<std::sync::Arc<uptrakit_config_reload::config::TlsConfig>>,
    /// Audit config watch receiver.
    pub audit_config_rx:
        tokio::sync::watch::Receiver<std::sync::Arc<uptrakit_config_reload::config::AuditConfig>>,
    /// Log config watch receiver.
    pub log_config_rx:
        tokio::sync::watch::Receiver<std::sync::Arc<uptrakit_config_reload::config::LogConfig>>,
    /// Master key watch receiver (boot-time only; changes require reexec).
    pub master_key_config_rx:
        tokio::sync::watch::Receiver<std::sync::Arc<uptrakit_shared_types::SecretString>>,
    /// Embedded services config watch receiver.
    pub embedded_services_config_rx: tokio::sync::watch::Receiver<
        std::sync::Arc<uptrakit_config_reload::config::EmbeddedServicesConfig>,
    >,
    /// Zeroconf config watch receiver.
    pub zeroconf_config_rx: tokio::sync::watch::Receiver<
        std::sync::Arc<uptrakit_config_reload::config::ZeroconfConfig>,
    >,
    /// MCP OAuth 2.1 authorization-server state.
    ///
    /// When `oauth.enabled = false` (the default) this carries inert placeholder
    /// values; all `/oauth/*` route handlers must guard on `state.oauth.enabled`
    /// before doing any work.
    pub oauth: crate::oauth::OAuthState,
    /// Config file state watch receiver — updated by the reload audit bridge
    /// whenever a reload cycle applies a new file.
    pub config_file_state: tokio::sync::watch::Receiver<uptrakit_config_reload::ConfigFileState>,
    /// Last successful reload info — updated by the reload audit bridge after
    /// each applied reload cycle.
    pub last_reload: tokio::sync::watch::Receiver<Option<uptrakit_config_reload::LastReloadInfo>>,
    /// Recent reload events (max 20) — updated by the reload audit bridge.
    pub recent_reload_events: tokio::sync::watch::Receiver<Vec<serde_json::Value>>,
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
    server_cert_resolver: Option<Arc<dyn crate::server_cert_swap::ServerCertSwap>>,
    crl_pem_cache: Option<Arc<parking_lot::RwLock<String>>>,
    ca_rotation_trigger: Option<Arc<tokio::sync::Notify>>,
    default_tenant_id: Option<uuid::Uuid>,
    controller_id: Option<uuid::Uuid>,
    notification_service: Option<NotificationService>,
    notification_dispatcher: Option<crate::notifications::dispatcher::NotificationDispatcher>,
    token_denylist: Option<Arc<crate::auth::token_denylist::TokenDenylist>>,
    plugin_ops: Option<Arc<dyn PluginOps>>,
    global_providers: Option<Arc<crate::global_providers::GlobalProviders>>,
    credential_sources: Option<ServiceCredentialSources>,
    event_broadcaster: Option<crate::event_broadcaster::EventBroadcaster>,
    update_output_broadcaster: Option<crate::update_output_broadcaster::UpdateOutputBroadcaster>,
    batch_progress_broadcaster: Option<crate::batch_progress_broadcaster::BatchProgressBroadcaster>,
    shutdown_token: Option<CancellationToken>,
    embedded_service_notifier: Option<Arc<dyn EmbeddedServiceNotifier>>,
    audit_log_filter_rx: Option<
        tokio::sync::watch::Receiver<std::sync::Arc<uptrakit_config_reload::config::AuditConfig>>,
    >,
    audit_log_dispatcher: Option<uptrakit_audit_log::AuditLogDispatcher>,
    audit_emitter: Option<uptrakit_audit_log::AuditEmitter>,
    surface_registry: Option<Arc<SurfaceRegistry>>,
    surface_proxy: Option<Arc<SurfaceProxy>>,
    config_test_proxy: Option<Arc<ConfigTestProxy>>,
    workload_claim_registry: Option<Arc<crate::workload_claims::WorkloadClaimRegistry>>,
    reject_dangerous_commands: bool,
    update_dispatcher: Option<Arc<dyn uptrakit_controller_core::update::UpdateDispatcher>>,
    instance_plugin_snapshot: Option<
        Arc<
            arc_swap::ArcSwap<
                uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot,
            >,
        >,
    >,
    coordinator_handle: Option<uptrakit_config_reload::ReloadCoordinatorHandle>,
    settings_version_cache: Option<uptrakit_config_reload::SettingsVersionCache>,
    config_receivers: Option<uptrakit_config_reload::RuntimeConfigReceivers>,
    oauth: Option<crate::oauth::OAuthState>,
    config_file_state_rx:
        Option<tokio::sync::watch::Receiver<uptrakit_config_reload::ConfigFileState>>,
    last_reload_rx:
        Option<tokio::sync::watch::Receiver<Option<uptrakit_config_reload::LastReloadInfo>>>,
    recent_reload_events_rx: Option<tokio::sync::watch::Receiver<Vec<serde_json::Value>>>,
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
            server_cert_resolver: None,
            crl_pem_cache: None,
            ca_rotation_trigger: None,
            default_tenant_id: None,
            controller_id: None,
            notification_service: None,
            notification_dispatcher: None,
            token_denylist: None,
            plugin_ops: None,
            global_providers: None,
            credential_sources: None,
            event_broadcaster: None,
            update_output_broadcaster: None,
            batch_progress_broadcaster: None,
            shutdown_token: None,
            embedded_service_notifier: None,
            audit_log_filter_rx: None,
            audit_log_dispatcher: None,
            audit_emitter: None,
            surface_registry: None,
            surface_proxy: None,
            config_test_proxy: None,
            workload_claim_registry: None,
            reject_dangerous_commands: false,
            update_dispatcher: None,
            instance_plugin_snapshot: None,
            coordinator_handle: None,
            settings_version_cache: None,
            config_receivers: None,
            oauth: None,
            config_file_state_rx: None,
            last_reload_rx: None,
            recent_reload_events_rx: None,
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

    /// Set the hot-swap server certificate resolver.
    ///
    /// Optional — defaults to `None`.  When set, the server certificate renewal
    /// handler calls [`ServerCertSwap::swap_cert`] instead of reloading the
    /// full TLS config from disk.
    pub fn server_cert_resolver(
        mut self,
        v: Arc<dyn crate::server_cert_swap::ServerCertSwap>,
    ) -> Self {
        self.server_cert_resolver = Some(v);
        self
    }

    pub fn crl_pem_cache(mut self, v: Arc<parking_lot::RwLock<String>>) -> Self {
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

    /// Override the host-owned global provider runtimes.
    ///
    /// Optional — defaults to a runtime registry built from the configured DB.
    pub fn global_providers(mut self, v: Arc<crate::global_providers::GlobalProviders>) -> Self {
        self.global_providers = Some(v);
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

    /// Set the live audit config watch receiver.
    ///
    /// Optional — defaults to a channel seeded with `AuditConfig::default()`.
    /// Production code should pass the receiver returned by
    /// [`AuditDispatcherReloadable::new`] so that the filter updates atomically
    /// after each config reload cycle.
    pub fn audit_log_filter_rx(
        mut self,
        v: tokio::sync::watch::Receiver<
            std::sync::Arc<uptrakit_config_reload::config::AuditConfig>,
        >,
    ) -> Self {
        self.audit_log_filter_rx = Some(v);
        self
    }

    /// Set the audit log dispatcher.
    ///
    /// Optional — defaults to a dispatcher backed by `NoopBackend`.
    pub fn audit_log_dispatcher(mut self, v: uptrakit_audit_log::AuditLogDispatcher) -> Self {
        self.audit_log_dispatcher = Some(v);
        self
    }

    /// Set the audit emitter.
    ///
    /// Optional — defaults to an emitter backed by `audit_log_dispatcher`.
    pub fn audit_emitter(mut self, v: uptrakit_audit_log::AuditEmitter) -> Self {
        self.audit_emitter = Some(v);
        self
    }

    /// Override the surface registry.
    ///
    /// Optional — defaults to an empty registry with default admission policy.
    pub fn surface_registry(mut self, v: Arc<SurfaceRegistry>) -> Self {
        self.surface_registry = Some(v);
        self
    }

    /// Override the surface proxy.
    ///
    /// Optional — defaults to an empty proxy with no pending requests.
    pub fn surface_proxy(mut self, v: Arc<SurfaceProxy>) -> Self {
        self.surface_proxy = Some(v);
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

    /// Override the update dispatcher.
    ///
    /// Optional — defaults to [`uptrakit_controller_core::update::controller::ControllerUpdateDispatcher`]
    /// wired from the builder's own fields (db, notification, output broadcaster, plugin ops,
    /// audit emitter). Override in tests to inject a [`uptrakit_controller_core::update::NoopUpdateDispatcher`]
    /// or a mock.
    pub fn update_dispatcher(
        mut self,
        v: Arc<dyn uptrakit_controller_core::update::UpdateDispatcher>,
    ) -> Self {
        self.update_dispatcher = Some(v);
        self
    }

    /// Set the instance-scoped plugin snapshot.
    ///
    /// Optional — defaults to an empty snapshot (all plugins disabled / unconfigured).
    /// The controller wires the real snapshot loaded from the DB at boot; tests may
    /// pass a pre-populated snapshot to exercise instance-gated routes.
    pub fn instance_plugin_snapshot(
        mut self,
        v: Arc<
            arc_swap::ArcSwap<
                uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot,
            >,
        >,
    ) -> Self {
        self.instance_plugin_snapshot = Some(v);
        self
    }

    /// Set the reload coordinator handle.
    ///
    /// Optional — defaults to a no-op coordinator with an empty Reloadable list
    /// and default-seeded config channels. Override to inject the real coordinator
    /// started by [`crate::startup::boot_config`].
    pub fn coordinator_handle(
        mut self,
        v: uptrakit_config_reload::ReloadCoordinatorHandle,
    ) -> Self {
        self.coordinator_handle = Some(v);
        self
    }

    /// Set the settings-version counter cache.
    ///
    /// Optional — defaults to an empty cache.
    pub fn settings_version_cache(
        mut self,
        v: uptrakit_config_reload::SettingsVersionCache,
    ) -> Self {
        self.settings_version_cache = Some(v);
        self
    }

    /// Set the per-section config watch receivers seeded at boot from TOML.
    ///
    /// Optional — defaults to receivers seeded with `RuntimeConfig::default()`.
    pub fn config_receivers(mut self, v: uptrakit_config_reload::RuntimeConfigReceivers) -> Self {
        self.config_receivers = Some(v);
        self
    }

    /// Set the config-reload status watch receivers.
    ///
    /// Optional — defaults to receivers seeded with empty/default values.
    /// The controller wires in receivers backed by the `reload_audit_bridge` task.
    pub fn config_reload_status_receivers(
        mut self,
        file_state: tokio::sync::watch::Receiver<uptrakit_config_reload::ConfigFileState>,
        last_reload: tokio::sync::watch::Receiver<Option<uptrakit_config_reload::LastReloadInfo>>,
        recent_events: tokio::sync::watch::Receiver<Vec<serde_json::Value>>,
    ) -> Self {
        self.config_file_state_rx = Some(file_state);
        self.last_reload_rx = Some(last_reload);
        self.recent_reload_events_rx = Some(recent_events);
        self
    }

    /// Set the MCP OAuth 2.1 authorization-server state.
    ///
    /// Optional — defaults to [`crate::oauth::OAuthState::disabled()`] (all
    /// `/oauth/*` routes return `404 Not Found`).  Pass a fully-constructed
    /// [`crate::oauth::OAuthState`] at boot when `oauth.mcp_enabled = true`.
    pub fn oauth(mut self, v: crate::oauth::OAuthState) -> Self {
        self.oauth = Some(v);
        self
    }

    /// Consume the builder and produce an [`AppState`].
    ///
    /// Returns [`AppStateBuildError`] naming the first field that was not set.
    ///
    /// # Errors
    ///
    /// Returns an error if any required field was not set before calling `build`.
    pub fn build(self) -> Result<AppState, rootcause::Report<AppStateBuildError>> {
        let db = self.db.ok_or_else(|| report!(AppStateBuildError("db")))?;
        let global_providers = self
            .global_providers
            .unwrap_or_else(|| Arc::new(crate::global_providers::GlobalProviders::new(db.clone())));
        let audit_log_dispatcher = self.audit_log_dispatcher.unwrap_or_else(|| {
            uptrakit_audit_log::AuditLogDispatcher::new(std::sync::Arc::new(
                uptrakit_audit_log::NoopBackend,
            ))
        });
        let audit_emitter = self
            .audit_emitter
            .unwrap_or_else(|| uptrakit_audit_log::AuditEmitter::new(audit_log_dispatcher.clone()));
        let notification = NotificationState::new(
            self.notification_service
                .ok_or_else(|| report!(AppStateBuildError("notification_service")))?,
            self.notification_dispatcher
                .ok_or_else(|| report!(AppStateBuildError("notification_dispatcher")))?,
            self.event_broadcaster.unwrap_or_default(),
        );
        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> =
            match self.plugin_ops {
                Some(p) => p,
                None => {
                    let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig {
                        global_provider_lookup: Some(global_providers.clone()),
                        ..uptrakit_plugin_infrastructure_registry::CatalogConfig::default()
                    };
                    Arc::new(
                    uptrakit_plugin_infrastructure_registry::build_catalog(
                        &catalog_config,
                        uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(
                        ),
                    )
                    .map_err(|e| {
                        tracing::error!(error = %e, "failed to build plugin catalog");
                        AppStateBuildError("plugin_catalog")
                    })?,
                )
                }
            };
        let update_output_broadcaster = self.update_output_broadcaster.unwrap_or_default();
        let update_dispatcher: Arc<dyn uptrakit_controller_core::update::UpdateDispatcher> =
            self.update_dispatcher.unwrap_or_else(|| {
                Arc::new(
                    uptrakit_controller_core::update::controller::ControllerUpdateDispatcher::new(
                        db.clone(),
                        notification.clone(),
                        Arc::new(update_output_broadcaster.clone()),
                        plugin_ops.clone(),
                        audit_emitter.clone(),
                    ),
                )
            });
        // Config-reload foundation: coordinator + per-section watch receivers.
        // If not set (e.g. in tests that don't load a TOML file), create a no-op
        // coordinator with an empty Reloadable list and default-seeded channels.
        let (coordinator_handle, settings_version_cache, config_receivers) = match (
            self.coordinator_handle,
            self.settings_version_cache,
            self.config_receivers,
        ) {
            (Some(h), Some(c), Some(r)) => (h, c, r),
            _ => {
                let default_runtime = uptrakit_config_reload::RuntimeConfig::default();
                let (_, receivers) =
                    uptrakit_config_reload::RuntimeConfigChannels::from_runtime(&default_runtime);
                let (audit_tx, _) = tokio::sync::mpsc::unbounded_channel();
                let (_, handle) = uptrakit_config_reload::ReloadCoordinator::new(
                    vec![],
                    audit_tx,
                    std::sync::Arc::new(uptrakit_config_reload::NoopAlertWriter),
                );
                let cache = uptrakit_config_reload::SettingsVersionCache::new();
                (handle, cache, receivers)
            }
        };

        // Config-reload status receivers — default to empty state for tests.
        let config_file_state = self.config_file_state_rx.unwrap_or_else(|| {
            tokio::sync::watch::channel(uptrakit_config_reload::ConfigFileState::default()).1
        });
        let last_reload = self
            .last_reload_rx
            .unwrap_or_else(|| tokio::sync::watch::channel(None).1);
        let recent_reload_events = self
            .recent_reload_events_rx
            .unwrap_or_else(|| tokio::sync::watch::channel(Vec::new()).1);

        Ok(AppState {
            db: DbState::new(db),
            cert: CertState {
                ca_snapshot: self
                    .ca_snapshot
                    .ok_or_else(|| report!(AppStateBuildError("ca_snapshot")))?,
                ca_key_store: self
                    .ca_key_store
                    .ok_or_else(|| report!(AppStateBuildError("ca_key_store")))?,
                revocation_notify: self
                    .revocation_notify
                    .ok_or_else(|| report!(AppStateBuildError("revocation_notify")))?,
                crl_pem_cache: self
                    .crl_pem_cache
                    .ok_or_else(|| report!(AppStateBuildError("crl_pem_cache")))?,
                ca_rotation_trigger: self
                    .ca_rotation_trigger
                    .ok_or_else(|| report!(AppStateBuildError("ca_rotation_trigger")))?,
            },
            auth: AuthState::new(
                self.jwt.ok_or_else(|| report!(AppStateBuildError("jwt")))?,
                self.device_flow_store
                    .ok_or_else(|| report!(AppStateBuildError("device_flow_store")))?,
                self.rate_limit_store
                    .ok_or_else(|| report!(AppStateBuildError("rate_limit_store")))?,
                self.token_denylist
                    .ok_or_else(|| report!(AppStateBuildError("token_denylist")))?,
            ),
            notification,
            broadcast: BroadcastState {
                update_output_broadcaster,
                batch_progress_broadcaster: self.batch_progress_broadcaster.unwrap_or_default(),
            },
            #[cfg(feature = "oidc")]
            oidc: OidcState {
                oidc_flow_store: self
                    .oidc_flow_store
                    .ok_or_else(|| report!(AppStateBuildError("oidc_flow_store")))?,
                account_link_store: self
                    .account_link_store
                    .ok_or_else(|| report!(AppStateBuildError("account_link_store")))?,
                oidc_token_exchange_store: self
                    .oidc_token_exchange_store
                    .ok_or_else(|| report!(AppStateBuildError("oidc_token_exchange_store")))?,
                oidc_registration_store: self
                    .oidc_registration_store
                    .ok_or_else(|| report!(AppStateBuildError("oidc_registration_store")))?,
            },
            settings: self
                .settings
                .ok_or_else(|| report!(AppStateBuildError("settings")))?,
            cert_signer: self
                .cert_signer
                .ok_or_else(|| report!(AppStateBuildError("cert_signer")))?,
            service_connections: self
                .service_connections
                .ok_or(AppStateBuildError("service_connections"))?,
            plugin: PluginState::new(plugin_ops, global_providers),
            credential_sources: self.credential_sources.unwrap_or_default(),
            shutdown_token: self.shutdown_token.unwrap_or_default(),
            embedded_service_notifier: self.embedded_service_notifier,
            audit_log_filter_rx: self.audit_log_filter_rx.unwrap_or_else(|| {
                tokio::sync::watch::channel(std::sync::Arc::new(
                    uptrakit_config_reload::config::AuditConfig::default(),
                ))
                .1
            }),
            audit_log_dispatcher,
            audit_emitter,
            surface_proxy_deps: SurfaceProxyDeps::new(
                self.surface_registry.unwrap_or_else(|| {
                    Arc::new(SurfaceRegistry::new(
                        crate::surface_registry::SurfaceRegistryConfig::default(),
                    ))
                }),
                self.surface_proxy
                    .unwrap_or_else(|| Arc::new(SurfaceProxy::new())),
            ),
            config_test_proxy: self
                .config_test_proxy
                .unwrap_or_else(|| Arc::new(ConfigTestProxy::new())),
            server: {
                let pki_path = self.pki_path.ok_or(AppStateBuildError("pki_path"))?;
                let rustls_config = self
                    .rustls_config
                    .ok_or(AppStateBuildError("rustls_config"))?;
                match self.server_cert_resolver {
                    Some(resolver) => {
                        ServerState::with_cert_resolver(pki_path, rustls_config, resolver)
                    }
                    None => ServerState::new(pki_path, rustls_config),
                }
            },
            default_tenant_id: self
                .default_tenant_id
                .ok_or_else(|| report!(AppStateBuildError("default_tenant_id")))?,
            controller_id: self
                .controller_id
                .ok_or_else(|| report!(AppStateBuildError("controller_id")))?,
            workload_claim_registry: self
                .workload_claim_registry
                .unwrap_or_else(|| Arc::new(crate::workload_claims::WorkloadClaimRegistry::new())),
            reject_dangerous_commands: self.reject_dangerous_commands,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
            #[cfg(feature = "test-utils")]
            test_reexec_notify: if std::env::var("UPTRAKIT_TEST_UTILS_ENABLED").as_deref()
                == Ok("true")
            {
                Some(Arc::new(tokio::sync::Notify::new()))
            } else {
                None
            },
            update_dispatcher,
            instance_plugin_snapshot: self.instance_plugin_snapshot.unwrap_or_else(|| {
                Arc::new(arc_swap::ArcSwap::from_pointee(
                    uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot::empty(),
                ))
            }),
            coordinator_handle,
            settings_version_cache,
            db_config_rx: config_receivers.db,
            network_config_rx: config_receivers.network,
            nats_config_rx: config_receivers.nats,
            tls_config_rx: config_receivers.tls,
            audit_config_rx: config_receivers.audit,
            log_config_rx: config_receivers.log,
            master_key_config_rx: config_receivers.master_key,
            embedded_services_config_rx: config_receivers.embedded_services,
            zeroconf_config_rx: config_receivers.zeroconf,
            oauth: self
                .oauth
                .unwrap_or_else(crate::oauth::OAuthState::disabled),
            config_file_state,
            last_reload,
            recent_reload_events,
        })
    }
}

impl AppState {
    /// Create a new builder for [`AppState`].
    ///
    /// This is the only public way to construct an `AppState` from outside the
    /// crate, preserving the `db` field encapsulation enforced by [`AppState::db`].
    #[must_use]
    pub fn builder() -> AppStateBuilder {
        AppStateBuilder::new()
    }

    /// Returns the host-owned global provider runtimes.
    pub fn global_providers(&self) -> Arc<crate::global_providers::GlobalProviders> {
        Arc::clone(&self.plugin.global_providers)
    }

    /// Returns the plugin operations abstraction.
    pub fn plugin_ops(&self) -> Arc<dyn PluginOps> {
        self.plugin.plugin_ops.clone()
    }

    /// Returns a reference to the underlying database connection.
    pub fn db(&self) -> &DatabaseConnection {
        self.db.db()
    }

    /// Returns a [`MutationContext`] borrowing the three common side-effect
    /// handles from this `AppState`. Pass it to action functions together with
    /// any domain-specific handles.
    pub(crate) fn mutation_context(&self) -> crate::actions::MutationContext<'_> {
        use crate::app_state::NotificationStateMutationExt;
        self.notification.mutation_context()
    }

    /// Returns the controller-side update-protection singleton, if registered.
    pub fn controller_update_protection(&self) -> Option<Arc<dyn ControllerUpdateProtection>> {
        self.plugin.plugin_ops.controller_update_protection()
    }

    /// Returns the controller-side update-hook singleton, if registered.
    #[cfg(feature = "plugin-ops")]
    pub fn controller_update_hook(
        &self,
    ) -> Option<Arc<dyn uptrakit_plugin_infrastructure_registry::ControllerUpdateHook>> {
        self.plugin.plugin_ops.controller_update_hook()
    }

    /// Returns the reexec notify handle used by the `POST /test/force-reexec`
    /// endpoint, if the `test-utils` feature is enabled and
    /// `UPTRAKIT_TEST_UTILS_ENABLED=true` was set at startup.
    ///
    /// The returned `Arc` keeps the `Notify` alive independently of `AppState`.
    #[cfg(feature = "test-utils")]
    pub fn test_reexec_notify(&self) -> Option<Arc<tokio::sync::Notify>> {
        self.test_reexec_notify.as_ref().map(Arc::clone)
    }
}

/// Allows Axum to extract [`DbState`] from `Arc<AppState>` via the blanket
/// `impl<S: DbStateSource> FromRef<Arc<S>> for DbState` provided by
/// `uptrakit-controller-core`.
impl DbStateSource for AppState {
    fn db_state(&self) -> DbState {
        self.db.clone()
    }
}

impl FromRef<Arc<AppState>> for CertState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        state.cert.clone()
    }
}

impl uptrakit_controller_core::auth::AuthStateSource for AppState {
    fn auth_state(&self) -> AuthState {
        self.auth.clone()
    }
}

impl uptrakit_controller_core::notification::NotificationStateSource for AppState {
    fn notification_state(&self) -> NotificationState {
        self.notification.clone()
    }
}

impl FromRef<Arc<AppState>> for BroadcastState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        state.broadcast.clone()
    }
}

impl FromRef<Arc<AppState>> for AuditEmitterState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        AuditEmitterState(state.audit_emitter.clone())
    }
}

impl FromRef<Arc<AppState>> for PluginOpsState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        PluginOpsState(state.plugin.plugin_ops.clone())
    }
}

impl FromRef<Arc<AppState>> for GlobalProvidersState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        GlobalProvidersState(state.plugin.global_providers.clone())
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
