#[cfg(feature = "embedded-agent")]
mod agent;
mod audit_enricher;
mod boot;
pub(crate) mod cert_signer;
pub(crate) mod cli;
pub(crate) mod crl_manager;
mod db;
mod db_migrate;
mod durations;
mod dynamic_verifier;
#[cfg_attr(
    not(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    )),
    expect(
        dead_code,
        reason = "infrastructure types used only when at least one embedded service feature is enabled"
    )
)]
pub(crate) mod embedded;
#[cfg(feature = "embedded-frontend")]
mod embedded_frontend;
mod migration;
#[cfg(feature = "embedded-mqtt")]
mod mqtt;
mod mtls_acceptor;
pub(crate) mod pki;
pub(crate) mod reconcile;
pub(crate) mod reencrypt;
pub(crate) mod reexec;
pub(crate) mod reload;
#[cfg(feature = "embedded-scheduler")]
mod scheduler;
pub(crate) mod server;
mod server_cert_resolver;
pub(crate) mod service_host;
#[cfg(feature = "embedded-ssh-agent")]
mod ssh_agent;
pub(crate) mod startup;
pub(crate) mod tasks;
#[cfg(feature = "zeroconf")]
mod zeroconf;

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use sea_orm::DatabaseConnection;
use thiserror::Error;
use uptrakit_audit_log::AuditLogDispatcher;
use uptrakit_build_info::BuildInfo;
use uptrakit_shared_macros::impl_report_conversion;

use uptrakit_config_reload::{ReexecHook, ReexecOutcome};
use uptrakit_web_api::AppState;
use uptrakit_web_api::oauth::boot::deregister_oauth_instance;

#[derive(Debug, Error)]
pub(crate) enum AppError {
    #[error("{0}")]
    Config(String),

    #[error("database initialization failed")]
    Database,

    #[error("settings initialization failed")]
    Settings,

    #[error("PKI initialization failed")]
    Pki,

    #[error("server error")]
    Server,
}

pub(crate) type Result<T> = std::result::Result<T, rootcause::Report<AppError>>;

impl_report_conversion!(
    uptrakit_crypto::CryptoError => AppError,
    |e| AppError::Config(e.to_string())
);

/// Reexec hook implementation for the controller process.
///
/// Captures listener FDs and exec arguments at startup so that
/// [`reexec::perform_reexec`] can reconstruct the command line without
/// re-reading the process environment after a signal.
pub(crate) struct ControllerReexecHook {
    /// Resolved from `std::env::current_exe()` at startup.
    current_exe: std::path::PathBuf,
    config_path: std::path::PathBuf,
    master_key_from: Option<String>,
    generation: u64,
    /// Number of bound listeners passed via `LISTEN_FDS` to the child process.
    /// 1 when PKI HTTP is disabled, 2 when enabled.
    listener_count: usize,
    /// Raw fd of the first (HTTPS) bound listener, passed as LISTEN_FDS_FIRST_FD.
    first_listener_fd: std::os::unix::io::RawFd,
    /// OAuth instance to deregister before exec. `None` when OAuth is disabled.
    oauth_instance: Option<(uuid::Uuid, sea_orm::DatabaseConnection)>,
}

impl ReexecHook for ControllerReexecHook {
    fn check_and_trigger(
        &self,
        prior: &uptrakit_config_reload::RuntimeConfig,
        new: &uptrakit_config_reload::RuntimeConfig,
    ) -> ReexecOutcome {
        let decision = reexec::triage::decide(prior, new);
        if !decision.needed {
            return ReexecOutcome::NotNeeded;
        }
        tracing::info!(reasons = ?decision.reasons, "reexec required by config change");

        let plan = reexec::ReexecPlan {
            current_exe: self.current_exe.clone(),
            config_path: self.config_path.clone(),
            master_key_from: self.master_key_from.clone(),
            listener_count: self.listener_count,
            generation: self.generation,
            first_listener_fd: self.first_listener_fd,
        };

        // INVARIANT: block_in_place requires the multi_thread Tokio runtime (panics on
        // current_thread). Tests exercising check_and_trigger must use
        // #[tokio::test(flavor = "multi_thread")].
        if let Some((instance_id, ref db)) = self.oauth_instance {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    if tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        deregister_oauth_instance(db, instance_id),
                    )
                    .await
                    .is_err()
                    {
                        tracing::warn!(
                            %instance_id,
                            "oauth deregister timed out before reexec; stale row will expire in ~90s"
                        );
                    }
                });
            });
        }

        match reexec::perform_reexec(&plan) {
            Ok(infallible) => match infallible {},
            Err(e) => ReexecOutcome::ExecFailed(e),
        }
    }
}

async fn async_main(info: BuildInfo) -> std::process::ExitCode {
    let args = cli::Args::parse();

    if args.version {
        print!("{}", info.render_human());
        return std::process::ExitCode::SUCCESS;
    }

    if args.check_config {
        let config_path = match args.find_config_path() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to resolve config directory: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        if let Err(e) = uptrakit_config_reload::TomlConfigLoader::validate_only(&config_path) {
            eprintln!("Config validation failed: {e}");
            return std::process::ExitCode::FAILURE;
        }
        return std::process::ExitCode::SUCCESS;
    }

    // Dispatch optional subcommands before entering the normal server path.
    if let Some(cli::ControllerCommand::DbMigrate(ref db_args)) = args.command {
        // Tracing is needed for db-migrate (master key init emits info).
        uptrakit_tracing_init::TracingBuilder::new().init();
        if let Err(report) = db_migrate::run(&args, db_args).await {
            eprintln!("db-migrate error: {report:?}");
            return std::process::ExitCode::FAILURE;
        }
        return std::process::ExitCode::SUCCESS;
    }

    if let Err(report) = Box::pin(boot::run_server(args, info)).await {
        eprintln!("Error:\n{report}");
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

// ---------------------------------------------------------------------------
// Extracted helper functions
// ---------------------------------------------------------------------------

/// Build the audit log filter and dispatcher from TOML configuration.
///
/// Configures the audit backend (database or noop) and the event filter
/// (all, mutations-only, or none). Backend defaults to database; filter
/// comes from `runtime.audit.filter`.
pub(crate) async fn build_audit_logger(
    runtime: &uptrakit_config_reload::RuntimeConfig,
    db_conn: &DatabaseConnection,
) -> Result<AuditLogDispatcher> {
    use uptrakit_audit_log::{FilterMode, NoopBackend};

    let filter_mode = match runtime.audit.filter.as_str() {
        "mutations" => FilterMode::Mutations,
        "none" => FilterMode::None,
        _ => FilterMode::All,
    };

    if filter_mode == FilterMode::None {
        let backend: std::sync::Arc<dyn uptrakit_audit_log::AuditLogBackend> =
            std::sync::Arc::new(NoopBackend);
        return Ok(AuditLogDispatcher::new(backend));
    }

    // Default: database backend using the main DB connection.
    let backend: std::sync::Arc<dyn uptrakit_audit_log::AuditLogBackend> =
        std::sync::Arc::new(uptrakit_audit_log::DatabaseBackend::new(db_conn.clone()));

    tracing::info!(
        filter = %filter_mode,
        "audit logging configured"
    );

    let enricher = std::sync::Arc::new(audit_enricher::DbActorEnricher::new(db_conn.clone()));

    Ok(AuditLogDispatcher::with_enricher(backend, enricher))
}

/// Spawn background tasks: CRL manager, denylist cleanup, CA reload/rotation,
/// server cert renewal, and NATS consumer. Embedded service registration is
/// handled by the caller after this function returns.
#[cfg_attr(
    feature = "nats",
    expect(
        clippy::too_many_arguments,
        reason = "spawns background infrastructure tasks; each parameter drives a distinct lifecycle phase"
    )
)]
pub(crate) async fn spawn_background_tasks(
    bg: &mut tasks::BackgroundTasks,
    app_state: &Arc<AppState>,
    crl_manager: &Arc<crl_manager::CrlManager>,
    ca_managed: bool,
    ca_tx: &tokio::sync::watch::Sender<pki::CaSnapshot>,
    initial_ca_version: i64,
    has_external_tls_cert: bool,
    #[cfg(feature = "nats")]
    service_connections: &uptrakit_web_api::service_connections::ServiceConnectionRegistry,
    #[cfg(feature = "nats")] nats_transport: &Option<
        uptrakit_web_api::nats_transport::NatsTransport,
    >,
) {
    // CRL manager: uses the child cancellation token for cooperative shutdown.
    // Must use track() (not track_abort()) so the manager finishes its current
    // cycle and writes the final TLS config before the process exits.
    let crl_handle = tokio::spawn(Arc::clone(crl_manager).run(Some(bg.child_token())));
    bg.track("crl-manager", crl_handle);

    // Token denylist cleanup (in-memory, per-instance — not in scheduler)
    let h =
        tasks::spawn_denylist_cleanup(bg.child_token(), Arc::clone(&app_state.auth.token_denylist));
    bg.track("denylist-cleanup", h);

    if ca_managed {
        let h = tasks::spawn_ca_reload(
            bg.child_token(),
            Arc::clone(app_state),
            ca_tx.clone(),
            Arc::clone(crl_manager),
            initial_ca_version,
        );
        bg.track("ca-reload", h);
    }

    if ca_managed {
        let h = tasks::spawn_ca_rotation(
            bg.child_token(),
            Arc::clone(app_state),
            ca_tx.clone(),
            Arc::clone(crl_manager),
        );
        bg.track("ca-rotation", h);
    }

    if !has_external_tls_cert {
        let h = tasks::spawn_server_cert_renewal(
            bg.child_token(),
            Arc::clone(app_state),
            Arc::clone(crl_manager),
        );
        bg.track("server-cert-renewal", h);
    }

    // NATS consumer (cross-controller event delivery)
    #[cfg(feature = "nats")]
    if let Some(ref nats) = *nats_transport {
        let h = tasks::spawn_nats_consumer(
            bg.child_token(),
            nats.clone(),
            uptrakit_web_api::nats_transport::NatsConsumerConfig {
                registry: service_connections.clone(),
                db: app_state.db().clone(),
                notification_service: app_state.notification.notification_service.clone(),
                event_broadcaster: app_state.notification.event_broadcaster.clone(),
                ca_rotation_trigger: Some(Arc::clone(&app_state.cert.ca_rotation_trigger)),
                revocation_notify: Some(Arc::clone(&app_state.cert.revocation_notify)),
                token_denylist: Some(Arc::clone(&app_state.auth.token_denylist)),
                claim_registry: Some(Arc::clone(&app_state.workload_claim_registry)),
            },
        );
        bg.track("nats-consumer", h);
    }
}

/// Spawn the zeroconf mDNS advertiser if the feature is enabled and configured.
#[cfg(feature = "zeroconf")]
pub(crate) fn spawn_zeroconf(
    bg: &mut tasks::BackgroundTasks,
    app_state: &Arc<AppState>,
    https_addr: SocketAddr,
) {
    let zeroconf_settings = app_state.settings.zeroconf();
    if zeroconf_settings.enabled {
        let ca_snap = app_state.cert.ca_snapshot.borrow().clone();
        let zc_cancel = bg.child_token();
        let handle = tokio::spawn(zeroconf::run_advertiser(
            zc_cancel,
            https_addr,
            ca_snap,
            zeroconf_settings,
        ));
        bg.track("zeroconf-advertiser", handle);
    }
}

/// Compute the hex-encoded SHA-256 digest of a file at `path`.
///
/// Returns an empty string and logs a warning if the file cannot be read.
fn file_digest(path: &std::path::Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => crate::pki::sha256_hex(&bytes),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "could not read config file for digest");
            String::new()
        }
    }
}

/// Status-watch channels + config path consumed by `reload_audit_bridge`.
pub(crate) struct ReloadBridgeChannels {
    pub file_state_tx: tokio::sync::watch::Sender<uptrakit_config_reload::ConfigFileState>,
    pub last_reload_tx: tokio::sync::watch::Sender<Option<uptrakit_config_reload::LastReloadInfo>>,
    pub recent_events_tx: tokio::sync::watch::Sender<Vec<serde_json::Value>>,
    pub config_path: std::path::PathBuf,
}

/// Bridge task: receive [`ReloadAuditEvent`]s from the coordinator and emit them as
/// system-scoped [`AuditEntry`] rows via [`AuditEmitter::emit_event`].
///
/// Also maintains the three status watch channels consumed by the
/// `GET /api/v1/instance/config-state` endpoint.
pub(crate) async fn reload_audit_bridge(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<uptrakit_config_reload::ReloadAuditEvent>,
    emitter: uptrakit_audit_log::AuditEmitter,
    channels: ReloadBridgeChannels,
) {
    use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome, Event};
    use uptrakit_config_reload::ReloadAuditEvent;

    while let Some(event) = rx.recv().await {
        // Update status watch channels.
        match &event {
            ReloadAuditEvent::FileChanged { path } => {
                let pending_digest = file_digest(path);
                channels.file_state_tx.send_modify(|s| {
                    s.pending_digest = Some(pending_digest);
                    s.pending_detected_at = Some(time::OffsetDateTime::now_utc());
                });
            }
            ReloadAuditEvent::Applied {
                sections,
                per_subsystem_ms,
                source,
            } => {
                let info = uptrakit_config_reload::LastReloadInfo::new(
                    time::OffsetDateTime::now_utc(),
                    sections.clone(),
                    per_subsystem_ms.clone(),
                );
                // Receivers may have been dropped (e.g. tests); ignore send errors.
                drop(channels.last_reload_tx.send(Some(info)));

                match source {
                    uptrakit_config_reload::ReloadSource::Sighup
                    | uptrakit_config_reload::ReloadSource::FileWatch { .. } => {
                        let new_digest = file_digest(&channels.config_path);
                        channels.file_state_tx.send_modify(|s| {
                            s.digest = new_digest;
                            s.loaded_at = time::OffsetDateTime::now_utc();
                            s.pending_digest = None;
                            s.pending_detected_at = None;
                        });
                    }
                    _ => {
                        tracing::debug!(
                            ?source,
                            "reload_audit_bridge: non-file source applied, no file-state update"
                        );
                    }
                }

                let event_json = serde_json::json!({
                    "type": "applied",
                    "at": time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| String::new()),
                    "sections": sections,
                });
                channels.recent_events_tx.send_modify(|v| {
                    v.push(event_json);
                    if v.len() > 20 {
                        v.remove(0);
                    }
                });
            }
            ReloadAuditEvent::Failed {
                phase,
                subsystem,
                error,
            } => {
                channels.file_state_tx.send_modify(|s| {
                    s.pending_digest = None;
                    s.pending_detected_at = None;
                });
                let event_json = serde_json::json!({
                    "type": "failed",
                    "at": time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| String::new()),
                    "phase": phase.as_str(),
                    "subsystem": subsystem,
                    "error": error,
                });
                channels.recent_events_tx.send_modify(|v| {
                    v.push(event_json);
                    if v.len() > 20 {
                        v.remove(0);
                    }
                });
            }
            ReloadAuditEvent::Reverted { subsystem, reason } => {
                let event_json = serde_json::json!({
                    "type": "reverted",
                    "at": time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| String::new()),
                    "subsystem": subsystem,
                    "reason": reason,
                });
                channels.recent_events_tx.send_modify(|v| {
                    v.push(event_json);
                    if v.len() > 20 {
                        v.remove(0);
                    }
                });
            }
            _ => {
                tracing::warn!(
                    "reload_audit_bridge: unhandled ReloadAuditEvent variant in status-watch; not updating any channel"
                );
            }
        }

        let (action, outcome, details) = match &event {
            ReloadAuditEvent::Requested { source } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_REQUESTED,
                AuditOutcome::Success,
                serde_json::json!({ "source": source }),
            ),
            ReloadAuditEvent::Refused { source, reason } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_REFUSED,
                AuditOutcome::Failed,
                serde_json::json!({ "source": source, "reason": reason }),
            ),
            ReloadAuditEvent::Applied {
                sections,
                per_subsystem_ms,
                source: _,
            } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_APPLIED,
                AuditOutcome::Success,
                serde_json::json!({ "sections": sections, "per_subsystem_ms": per_subsystem_ms }),
            ),
            ReloadAuditEvent::Failed {
                phase,
                subsystem,
                error,
            } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_FAILED,
                AuditOutcome::Failed,
                serde_json::json!({ "phase": phase, "subsystem": subsystem, "error": error }),
            ),
            ReloadAuditEvent::Reverted { subsystem, reason } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_REVERTED,
                AuditOutcome::Failed,
                serde_json::json!({ "subsystem": subsystem, "reason": reason }),
            ),
            ReloadAuditEvent::FileChanged { .. } => continue, // not audit-logged; handled in status watch
            _ => {
                tracing::warn!(
                    "reload_audit_bridge: unhandled ReloadAuditEvent variant (skipping audit emit)"
                );
                continue;
            }
        };
        if let Ok(entry) = AuditEntry::<Event>::builder_event(action)
            .system_scope()
            .outcome(outcome)
            .details(details)
            .build()
        {
            emitter.emit_event(entry);
        }
    }
}

/// Spawn the optional plain-HTTP PKI server on the given port.
///
/// `inherited` is a pre-bound socket to reuse on the reexec path; `None` on
/// cold start causes a fresh `bind(addr)`.
pub(crate) fn spawn_pki_http(
    bg: &mut tasks::BackgroundTasks,
    app_state: &Arc<AppState>,
    pki_http_port: Option<u16>,
    inherited: Option<std::net::TcpListener>,
) {
    let Some(port) = pki_http_port else {
        return;
    };
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let app_state_for_pki = Arc::clone(app_state);
    let pki_http_handle = tokio::spawn(async move {
        if let Err(e) = server::run_pki_http(addr, app_state_for_pki, inherited).await {
            tracing::error!(error = ?e, "PKI HTTP server error");
        }
    });
    bg.track_abort("pki-http", pki_http_handle);
}

#[expect(
    clippy::expect_used,
    reason = "infallible at startup: tokio runtime construction failures are unrecoverable and must abort process initialization"
)]
pub fn run(info: BuildInfo) -> std::process::ExitCode {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime")
        .block_on(async_main(info))
}
