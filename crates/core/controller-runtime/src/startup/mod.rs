//! Controller startup phase functions.
//!
//! Each public function maps to a distinct startup phase extracted from the
//! monolithic `run()` function. Intermediate results are passed between phases
//! via explicit structs ([`ReconciledSettings`], [`ValidatedConfig`],
//! [`PkiRuntime`]).

mod bootstrap;
mod database;
mod encryption;
mod installation_id;
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
pub(crate) use installation_id::init_installation_id;
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

// ---------------------------------------------------------------------------
// Config-reload boot
// ---------------------------------------------------------------------------

/// Result of loading the TOML config file at startup.
pub(crate) struct BootedConfig {
    /// Parsed and validated runtime configuration (used for further startup wiring).
    ///
    /// Plan 2 tasks consume this to seed subsystem-level Reloadables.
    #[expect(
        dead_code,
        reason = "read by Plan 2 tasks; unused until those tasks are wired in"
    )]
    pub runtime: uptrakit_config_reload::RuntimeConfig,
    /// Coordinator handle for state introspection and request submission.
    pub coordinator_handle: uptrakit_config_reload::ReloadCoordinatorHandle,
    /// Settings-version counter cache.
    pub settings_version_cache: uptrakit_config_reload::SettingsVersionCache,
    /// Watch channel senders held by the coordinator for publishing live updates.
    ///
    /// Plan 2 Reloadables call `channels.db.send(...)` etc. to push new values.
    #[expect(
        dead_code,
        reason = "consumed by Plan 2 Reloadables; unused until those are wired in"
    )]
    pub channels: uptrakit_config_reload::RuntimeConfigChannels,
    /// Watch channel receivers distributed to subsystems at startup.
    pub receivers: uptrakit_config_reload::RuntimeConfigReceivers,
}

/// Load the TOML config file, seed per-section watch channels, and start the
/// reload coordinator and its triggers.
///
/// Called once at boot before `AppState` is constructed. The returned
/// [`BootedConfig`] feeds into `AppStateBuilder`.
///
/// # Errors
///
/// Returns an error if the file cannot be read, the TOML is malformed, or any
/// config section fails validation.
pub(crate) async fn boot_config(config_path: PathBuf) -> Result<BootedConfig, rootcause::Report> {
    let loaded = uptrakit_config_reload::TomlConfigLoader::load(&config_path)?;
    for w in &loaded.warnings {
        tracing::warn!("config: {w}");
    }
    let (channels, receivers) =
        uptrakit_config_reload::RuntimeConfigChannels::from_runtime(&loaded.config);
    let (audit_tx, _audit_rx) = tokio::sync::mpsc::unbounded_channel();
    // Reloadables are populated in Plan 2; start with an empty list.
    let reloadables: Vec<std::sync::Arc<dyn uptrakit_config_reload::ReloadableErased>> = Vec::new();
    let (coordinator, handle) =
        uptrakit_config_reload::ReloadCoordinator::new(reloadables, audit_tx);
    tokio::spawn(coordinator.run());
    let _sighup = uptrakit_config_reload::triggers::sighup::spawn_sighup_task(handle.sender());
    let _watch = uptrakit_config_reload::triggers::file_watch::spawn_file_watch_task(
        config_path,
        handle.sender(),
    );
    let settings_version_cache = uptrakit_config_reload::SettingsVersionCache::new();
    Ok(BootedConfig {
        runtime: loaded.config,
        coordinator_handle: handle,
        settings_version_cache,
        channels,
        receivers,
    })
}

// ---------------------------------------------------------------------------
// PKI runtime
// ---------------------------------------------------------------------------

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
