use std::sync::Arc;

use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_registry::PluginOps;

use crate::auth::device_flow::DeviceFlowStore;
use crate::auth::jwt::JwtManager;
#[cfg(feature = "oidc")]
use crate::auth::oidc_state::{
    AccountLinkStore, OidcFlowStore, OidcRegistrationStore, OidcTokenExchangeStore,
};
use crate::auth::rate_limit::RateLimitStore;
use crate::ca_snapshot::{CaKeyStoreRef, CaSnapshotReceiver};
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

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    /// Watch receiver for the current CA snapshot (bundle PEM, fingerprints, etc.).
    pub ca_snapshot: CaSnapshotReceiver,
    /// Private CA key store — only for OCSP, CRL, and cert signing operations.
    pub ca_key_store: CaKeyStoreRef,
    /// Database connection pool.
    pub(crate) db: DatabaseConnection,
    /// Application settings catalogue (includes network settings).
    pub settings: Settings,
    /// Agent certificate signer for mTLS enrollment.
    pub cert_signer: Arc<dyn crate::cert_signer::AgentCertSigner>,
    /// Unified registry of connected services (agents and MQTT) for push notifications.
    pub service_connections: ServiceConnectionRegistry,
    /// Notify channel: fire after any certificate revocation to trigger CRL rebuild.
    pub revocation_notify: Arc<tokio::sync::Notify>,
    /// Database-backed store for pending OIDC authorization flows.
    #[cfg(feature = "oidc")]
    pub oidc_flow_store: OidcFlowStore,
    /// Database-backed store for pending OIDC account links.
    #[cfg(feature = "oidc")]
    pub account_link_store: AccountLinkStore,
    /// JWT signing/validation manager for access tokens.
    pub jwt: Arc<JwtManager>,
    /// Database-backed store for pending OIDC token exchanges.
    #[cfg(feature = "oidc")]
    pub oidc_token_exchange_store: OidcTokenExchangeStore,
    /// Database-backed store for pending OIDC registrations (token-gated).
    #[cfg(feature = "oidc")]
    pub oidc_registration_store: OidcRegistrationStore,
    /// Database-backed store for pending device authorization flows.
    pub device_flow_store: DeviceFlowStore,
    /// Database-backed rate limiter for public authentication endpoints.
    pub rate_limit_store: RateLimitStore,
    /// Path to the PKI directory (for server cert renewal).
    pub pki_path: std::path::PathBuf,
    /// RustlsConfig handle for hot-reloading TLS.
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    /// Cached PEM-encoded CRL bundle, updated by the CRL manager.
    pub crl_pem_cache: Arc<tokio::sync::RwLock<String>>,
    /// Trigger for immediate CA rotation (fired by the rotate-ca API endpoint).
    pub ca_rotation_trigger: Arc<tokio::sync::Notify>,
    /// UUID of the default (seeded) tenant. Used as fallback when no tenant header is present.
    pub default_tenant_id: uuid::Uuid,
    /// Unique identifier for this controller instance (used for cross-controller notification delivery).
    pub controller_id: uuid::Uuid,
    /// Cross-controller notification service for push message delivery via outbox pattern.
    pub notification_service: NotificationService,
    /// In-memory denylist for immediate JWT access token revocation.
    pub token_denylist: Arc<crate::auth::token_denylist::TokenDenylist>,
    /// Plugin operations abstraction used by plugin-config route handlers.
    ///
    /// Injected via `Arc<dyn PluginOps>` so that route handlers and query
    /// helpers are decoupled from the concrete [`uptrakit_plugin_infrastructure_registry::PluginRegistry`]
    /// and can be tested with a mock implementation.
    pub plugin_ops: Arc<dyn PluginOps>,
    /// Credential sources for services with credential capabilities.
    pub credential_sources: ServiceCredentialSources,
    /// Per-update broadcast channels for real-time output streaming via SSE.
    pub update_output_broadcaster: crate::update_output_broadcaster::UpdateOutputBroadcaster,
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
    token_denylist: Option<Arc<crate::auth::token_denylist::TokenDenylist>>,
    plugin_ops: Option<Arc<dyn PluginOps>>,
    credential_sources: Option<ServiceCredentialSources>,
    update_output_broadcaster: Option<crate::update_output_broadcaster::UpdateOutputBroadcaster>,
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
            token_denylist: None,
            plugin_ops: None,
            credential_sources: None,
            update_output_broadcaster: None,
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

    pub fn token_denylist(mut self, v: Arc<crate::auth::token_denylist::TokenDenylist>) -> Self {
        self.token_denylist = Some(v);
        self
    }

    /// Override the plugin operations implementation.
    ///
    /// Defaults to [`uptrakit_plugin_infrastructure_registry::PluginRegistry`] when not set.
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

    /// Consume the builder and produce an [`AppState`].
    ///
    /// Returns [`AppStateBuildError`] naming the first field that was not set.
    ///
    /// # Errors
    ///
    /// Returns an error if any required field was not set before calling `build`.
    pub fn build(self) -> Result<AppState, AppStateBuildError> {
        Ok(AppState {
            ca_snapshot: self.ca_snapshot.ok_or(AppStateBuildError("ca_snapshot"))?,
            ca_key_store: self
                .ca_key_store
                .ok_or(AppStateBuildError("ca_key_store"))?,
            db: self.db.ok_or(AppStateBuildError("db"))?,
            settings: self.settings.ok_or(AppStateBuildError("settings"))?,
            cert_signer: self.cert_signer.ok_or(AppStateBuildError("cert_signer"))?,
            service_connections: self
                .service_connections
                .ok_or(AppStateBuildError("service_connections"))?,
            revocation_notify: self
                .revocation_notify
                .ok_or(AppStateBuildError("revocation_notify"))?,
            #[cfg(feature = "oidc")]
            oidc_flow_store: self
                .oidc_flow_store
                .ok_or(AppStateBuildError("oidc_flow_store"))?,
            #[cfg(feature = "oidc")]
            account_link_store: self
                .account_link_store
                .ok_or(AppStateBuildError("account_link_store"))?,
            jwt: self.jwt.ok_or(AppStateBuildError("jwt"))?,
            #[cfg(feature = "oidc")]
            oidc_token_exchange_store: self
                .oidc_token_exchange_store
                .ok_or(AppStateBuildError("oidc_token_exchange_store"))?,
            #[cfg(feature = "oidc")]
            oidc_registration_store: self
                .oidc_registration_store
                .ok_or(AppStateBuildError("oidc_registration_store"))?,
            device_flow_store: self
                .device_flow_store
                .ok_or(AppStateBuildError("device_flow_store"))?,
            rate_limit_store: self
                .rate_limit_store
                .ok_or(AppStateBuildError("rate_limit_store"))?,
            pki_path: self.pki_path.ok_or(AppStateBuildError("pki_path"))?,
            rustls_config: self
                .rustls_config
                .ok_or(AppStateBuildError("rustls_config"))?,
            crl_pem_cache: self
                .crl_pem_cache
                .ok_or(AppStateBuildError("crl_pem_cache"))?,
            ca_rotation_trigger: self
                .ca_rotation_trigger
                .ok_or(AppStateBuildError("ca_rotation_trigger"))?,
            default_tenant_id: self
                .default_tenant_id
                .ok_or(AppStateBuildError("default_tenant_id"))?,
            controller_id: self
                .controller_id
                .ok_or(AppStateBuildError("controller_id"))?,
            notification_service: self
                .notification_service
                .ok_or(AppStateBuildError("notification_service"))?,
            token_denylist: self
                .token_denylist
                .ok_or(AppStateBuildError("token_denylist"))?,
            plugin_ops: self.plugin_ops.unwrap_or_else(|| {
                Arc::new(uptrakit_plugin_infrastructure_registry::PluginRegistry)
            }),
            credential_sources: self.credential_sources.unwrap_or_default(),
            update_output_broadcaster: self.update_output_broadcaster.unwrap_or_default(),
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
        &self.db
    }
}
