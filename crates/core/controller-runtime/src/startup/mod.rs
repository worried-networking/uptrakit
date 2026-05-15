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
mod master_key;
mod oauth;
mod pki_init;
mod settings;
mod validation;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) use bootstrap::{bootstrap_enrollment_tokens, bootstrap_oidc};
pub(crate) use database::init_database;
pub(crate) use encryption::{init_data_key_ring, verify_master_key};
pub(crate) use installation_id::init_installation_id;
pub(crate) use jwt::init_jwt;
pub(crate) use master_key::init_master_key;
pub(crate) use oauth::seed_oauth_defaults;
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
    /// Read at boot to wire `trust_domain` from `[tls]` into `RcgenAgentCertSigner`.
    /// Also consumed by Plan 2 tasks to seed subsystem-level Reloadables.
    pub runtime: uptrakit_config_reload::RuntimeConfig,
    /// `Arc`-wrapped clone of `runtime` for seeding the coordinator's
    /// `current_config` field without cloning the whole struct again.
    #[expect(dead_code, reason = "seeded into ReloadCoordinator in Plan 3")]
    pub runtime_arc: std::sync::Arc<uptrakit_config_reload::RuntimeConfig>,
    /// Coordinator (not yet spawned). Caller adds Reloadables via
    /// [`uptrakit_config_reload::ReloadCoordinator::extend_reloadables`], extracts
    /// a handle, then spawns.
    pub coordinator: uptrakit_config_reload::ReloadCoordinator,
    /// Settings-version counter cache.
    pub settings_version_cache: uptrakit_config_reload::SettingsVersionCache,
    /// Watch channel senders held by the coordinator for publishing live updates.
    ///
    /// The per-section senders are not yet consumed by the run loop; the status
    /// senders (`reload_file_state_tx` etc.) serve the config-state endpoint.
    #[expect(
        dead_code,
        reason = "per-section senders consumed by coordinator fan-out (future plan); unused until then"
    )]
    pub channels: uptrakit_config_reload::RuntimeConfigChannels,
    /// Watch channel receivers distributed to subsystems at startup.
    pub receivers: uptrakit_config_reload::RuntimeConfigReceivers,
    /// Receiver for audit events emitted by the reload coordinator.
    ///
    /// Consumed by the bridge task in `run_server` to convert `ReloadAuditEvent`
    /// values into `AuditEntry` rows via `AuditEmitter::emit_event`.
    pub audit_rx: tokio::sync::mpsc::UnboundedReceiver<uptrakit_config_reload::ReloadAuditEvent>,
    /// Config file state sender — held by the `reload_audit_bridge` task.
    ///
    /// Updated at boot with the initial file state; updated again whenever a
    /// reload cycle successfully applies a new config file.
    pub reload_file_state_tx: tokio::sync::watch::Sender<uptrakit_config_reload::ConfigFileState>,
    /// Config file state receiver — distributed to `AppState`.
    pub reload_file_state_rx: tokio::sync::watch::Receiver<uptrakit_config_reload::ConfigFileState>,
    /// Last successful reload info sender — held by the `reload_audit_bridge` task.
    pub reload_last_reload_tx:
        tokio::sync::watch::Sender<Option<uptrakit_config_reload::LastReloadInfo>>,
    /// Last successful reload info receiver — distributed to `AppState`.
    pub reload_last_reload_rx:
        tokio::sync::watch::Receiver<Option<uptrakit_config_reload::LastReloadInfo>>,
    /// Recent reload events sender (max 20 items) — held by the `reload_audit_bridge` task.
    pub reload_recent_events_tx: tokio::sync::watch::Sender<Vec<serde_json::Value>>,
    /// Recent reload events receiver — distributed to `AppState`.
    pub reload_recent_events_rx: tokio::sync::watch::Receiver<Vec<serde_json::Value>>,
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
    let (audit_tx, audit_rx) = tokio::sync::mpsc::unbounded_channel();
    let (coordinator, handle) = uptrakit_config_reload::ReloadCoordinator::new(
        Vec::new(),
        audit_tx,
        std::sync::Arc::new(uptrakit_config_reload::NoopAlertWriter),
    );
    let _sighup = uptrakit_config_reload::triggers::sighup::spawn_sighup_task(handle.sender());
    let _watch = uptrakit_config_reload::triggers::file_watch::spawn_file_watch_task(
        config_path.clone(),
        handle.sender(),
    );
    let settings_version_cache = uptrakit_config_reload::SettingsVersionCache::new();

    // Compute initial file state using SHA-256 digest.
    let digest = file_digest(&config_path);
    let initial_file_state = uptrakit_config_reload::ConfigFileState::new(
        config_path.display().to_string(),
        digest,
        time::OffsetDateTime::now_utc(),
        None,
        None,
    );
    let (reload_file_state_tx, reload_file_state_rx) =
        tokio::sync::watch::channel(initial_file_state);
    let (reload_last_reload_tx, reload_last_reload_rx) = tokio::sync::watch::channel(None);
    let (reload_recent_events_tx, reload_recent_events_rx) =
        tokio::sync::watch::channel(Vec::new());

    let runtime_arc = std::sync::Arc::new(loaded.config.clone());

    Ok(BootedConfig {
        runtime: loaded.config,
        runtime_arc,
        coordinator,
        settings_version_cache,
        channels,
        receivers,
        audit_rx,
        reload_file_state_tx,
        reload_file_state_rx,
        reload_last_reload_tx,
        reload_last_reload_rx,
        reload_recent_events_tx,
        reload_recent_events_rx,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute a `"sha256:<hex>"` digest of the file at `path`.
///
/// Falls back to `"size:<N>"` on I/O error so that the status endpoint
/// always has a value rather than returning an empty string.
pub(crate) fn file_digest(path: &std::path::Path) -> String {
    use sha2::{Digest as _, Sha256};
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut h = Sha256::new();
            h.update(&bytes);
            format!("sha256:{:x}", h.finalize())
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "could not read config for digest; using size stub"
            );
            format!("size:{}", path.metadata().map(|m| m.len()).unwrap_or(0))
        }
    }
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
    /// Hot-swap handle for atomically replacing the server certificate without
    /// rebuilding [`rustls::ServerConfig`].
    pub server_cert_resolver:
        std::sync::Arc<crate::server_cert_resolver::ControllerServerCertResolver>,
    pub revocation_notify: Arc<tokio::sync::Notify>,
    pub ca_rotation_trigger: Arc<tokio::sync::Notify>,
    pub crl_pem_cache: Arc<parking_lot::RwLock<String>>,
    pub crl_manager: Arc<crate::crl_manager::CrlManager>,
    pub initial_ca_version: i64,
    /// `true` when `tls.cert_path` and `tls.key_path` were set in TOML config,
    /// meaning the server certificate is externally managed.
    pub has_external_tls_cert: bool,
}
