//! Boot identity sub-module: Phases 7d (OAuth), 9 (PKI/TLS + cert_signer), 10 (JWT).
//!
//! Orchestrates three identity-related startup phases and returns an [`Identity`]
//! value whose fields are consumed by the `AppState` builder and the background
//! task spawner.

mod jwt;
mod oauth;
mod pki;

use std::path::PathBuf;
use std::sync::Arc;

use uptrakit_web_api::auth::jwt::JwtManager;
use uptrakit_web_api::cert_signer::AgentCertSigner;
use uptrakit_web_api::oauth::OAuthState;

// ---------------------------------------------------------------------------
// PkiFields — flat representation of PkiRuntime after destructuring
// ---------------------------------------------------------------------------

/// The 13 fields of [`crate::boot::init::PkiRuntime`], stored directly so that
/// the `AppState` builder (Task 12) and `ServeDeps` (Task 12.3) can access
/// individual fields without fighting over a nested un-destructured struct.
pub(crate) struct PkiFields {
    pub ca_managed: bool,
    pub pki_path: PathBuf,
    pub ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    pub ca_rx: tokio::sync::watch::Receiver<crate::pki::CaSnapshot>,
    pub ca_key_store: uptrakit_web_api::CaKeyStoreRef,
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    pub server_cert_resolver: Arc<crate::server_cert_resolver::ControllerServerCertResolver>,
    pub revocation_notify: Arc<tokio::sync::Notify>,
    pub ca_rotation_trigger: Arc<tokio::sync::Notify>,
    pub crl_pem_cache: Arc<parking_lot::RwLock<String>>,
    pub crl_manager: Arc<crate::crl_manager::CrlManager>,
    pub initial_ca_version: i64,
    pub has_external_tls_cert: bool,
}

// ---------------------------------------------------------------------------
// Identity — all outputs from Phases 7d + 9 + 10
// ---------------------------------------------------------------------------

/// Combined output of the identity boot phases (7d, 9, 10).
pub(crate) struct Identity {
    /// Flat PKI/TLS runtime state (destructured from [`crate::boot::init::PkiRuntime`]).
    pub pki: PkiFields,
    /// JWT signing-key manager (Phase 10).
    pub jwt_manager: JwtManager,
    /// Agent certificate signer built from PKI state.
    pub cert_signer: Arc<dyn AgentCertSigner>,
    /// MCP OAuth 2.1 authorization-server state (Phase 7d).
    pub oauth_state: OAuthState,
    /// Shutdown-cleanup handle: present when OAuth is enabled.
    pub oauth_instance_for_shutdown: Option<(uuid::Uuid, sea_orm::DatabaseConnection)>,
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Orchestrate Phases 7d → 9 → 10 and return a single [`Identity`] value.
///
/// # Arguments
///
/// - `runtime`: parsed TOML runtime configuration.
/// - `db`: live database connection.
/// - `config_dir`: path to the controller config directory (for PKI).
/// - `state_dir`: path to the controller state directory (for JWT key migration).
/// - `reconciled`: reconciled settings produced by Phase 6.
///
/// # Phase ordering
///
/// - **7d** (OAuth): reads DB only; independent of listeners and PKI.
/// - **9** (PKI/TLS): reads DB + filesystem; independent of OAuth.
/// - **10** (JWT): reads DB only; independent of PKI and OAuth.
///
/// Phase 7d previously ran immediately before `listeners::claim` in
/// `run_server`.  Both are independent (OAuth reads only DB; listeners reads
/// only reconciled settings), so running `identity::init` — including 7d —
/// *after* `listeners::claim` is behavior-equivalent.
pub(crate) async fn init(
    runtime: &uptrakit_config_reload::RuntimeConfig,
    db: &sea_orm::DatabaseConnection,
    config_dir: &std::path::Path,
    state_dir: &std::path::Path,
    reconciled: &crate::boot::init::ReconciledSettings,
) -> crate::Result<Identity> {
    // Phase 7d: OAuth
    let (oauth_state, oauth_instance_for_shutdown) = oauth::boot(db).await?;

    // Phase 9: PKI + TLS
    let pki_runtime = pki::init(runtime, db, config_dir, reconciled).await?;

    // Build cert signer from PKI runtime before consuming pki_runtime.
    let cert_signer = pki::build_cert_signer(&pki_runtime, runtime);

    // Destructure PkiRuntime into PkiFields so downstream consumers get plain
    // field access rather than a nested struct.
    let crate::boot::init::PkiRuntime {
        ca_managed,
        pki_path,
        ca_tx,
        ca_rx,
        ca_key_store,
        rustls_config,
        server_cert_resolver,
        revocation_notify,
        ca_rotation_trigger,
        crl_pem_cache,
        crl_manager,
        initial_ca_version,
        has_external_tls_cert,
    } = pki_runtime;

    let pki = PkiFields {
        ca_managed,
        pki_path,
        ca_tx,
        ca_rx,
        ca_key_store,
        rustls_config,
        server_cert_resolver,
        revocation_notify,
        ca_rotation_trigger,
        crl_pem_cache,
        crl_manager,
        initial_ca_version,
        has_external_tls_cert,
    };

    // Phase 10: JWT
    let jwt_manager = jwt::init(db, state_dir).await?;

    Ok(Identity {
        pki,
        jwt_manager,
        cert_signer,
        oauth_state,
        oauth_instance_for_shutdown,
    })
}
