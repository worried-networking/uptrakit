//! Controller startup phase functions.
//!
//! Each public function maps to a distinct startup phase extracted from the
//! monolithic `run()` function. Intermediate results are passed between phases
//! via explicit structs ([`ReconciledSettings`], [`ValidatedConfig`],
//! [`PkiRuntime`]).

mod bootstrap;
mod database;
mod encryption;
mod jwt;
mod key_rotation;
mod master_key;
mod pki_init;
mod settings;
mod validation;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) use bootstrap::{bootstrap_enrollment_tokens, bootstrap_oidc};
pub(crate) use database::{init_audit_database, init_database};
pub(crate) use encryption::{init_data_key_ring, verify_master_key};
pub(crate) use jwt::init_jwt;
pub(crate) use key_rotation::rotate_master_key;
pub(crate) use master_key::init_master_key;
pub(crate) use pki_init::init_pki_runtime;
pub(crate) use settings::reconcile_all_settings;
pub(crate) use validation::validate_configuration;

// ---------------------------------------------------------------------------
// Intermediate result types
// ---------------------------------------------------------------------------

/// Values produced by reconciling CLI / DB / default settings.
pub(crate) struct ReconciledSettings {
    pub sans: Vec<String>,
    pub pki_addr: Option<String>,
    pub https_addr: SocketAddr,
    /// Raw (decrypted) NATS URL, or `None` when not configured.
    /// Populated only when the `nats` feature is enabled.
    #[cfg(feature = "nats")]
    pub nats_url: Option<String>,
}

/// Configuration values validated after reconciliation.
pub(crate) struct ValidatedConfig {
    pub static_dir: Option<PathBuf>,
    pub pki_http_port: Option<u16>,
}

/// All PKI and TLS runtime state needed by `AppState` and background tasks.
pub(crate) struct PkiRuntime {
    pub ca_managed: bool,
    pub pki_path: PathBuf,
    pub ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    pub ca_rx: tokio::sync::watch::Receiver<crate::pki::CaSnapshot>,
    pub ca_key_store: uptrakit_web_api::CaKeyStoreRef,
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    pub revocation_notify: Arc<tokio::sync::Notify>,
    pub ca_rotation_trigger: Arc<tokio::sync::Notify>,
    pub crl_pem_cache: Arc<tokio::sync::RwLock<String>>,
    pub crl_manager: Arc<crate::crl_manager::CrlManager>,
    pub initial_ca_version: i64,
}
