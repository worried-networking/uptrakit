//! Boot phase: serve — background tasks, embedded registration, signal loop,
//! HTTP server, and graceful shutdown.
//!
//! [`run`] is the final phase of `run_server`.  It receives all long-lived
//! handles via [`ServeDeps`] and drives the process until a shutdown signal or
//! server exit, then tears everything down in order.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_build_info::BuildInfo;
use uptrakit_web_api::AppState;
use uptrakit_web_api::oauth::boot::deregister_oauth_instance;

use crate::AppError;
use crate::boot::listeners::Listeners;

// ---------------------------------------------------------------------------
// ServeDeps — handles separated before assemble consumes identity/components
// ---------------------------------------------------------------------------

/// Handles that the server-tail code (background tasks, embedded registration,
/// signal loop, graceful shutdown, OAuth deregister) needs after
/// [`super::app_state::assemble`] has consumed `identity` and `components` by
/// move.
///
/// Build this struct *before* calling `assemble`, by cloning `Arc`/`Copy`
/// fields from `identity.pki` and `components`, and by moving
/// `oauth_instance_for_shutdown` out of `identity` (the SIGTERM path owns it).
pub(crate) struct ServeDeps {
    // ---- PKI / TLS ----
    pub crl_manager: Arc<crate::crl_manager::CrlManager>,
    /// Cloned watch sender — `ca_tx.clone()` is cheap (Arc-backed).
    pub ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    pub ca_managed: bool,
    pub initial_ca_version: i64,
    pub has_external_tls_cert: bool,

    // ---- Service connections, NATS, and shutdown ----
    /// Cloned unconditionally — `bg.shutdown` needs it on every build.
    pub service_connections: uptrakit_web_api::service_connections::ServiceConnectionRegistry,
    /// Cloned (Arc-cheap) when the `nats` feature is enabled.
    #[cfg(feature = "nats")]
    pub nats_transport: Option<uptrakit_web_api::nats_transport::NatsTransport>,
    /// Cloned for `BackgroundTasks::new`; components are moved into assemble.
    pub shutdown_token: tokio_util::sync::CancellationToken,

    // ---- OAuth shutdown ----
    /// Moved (not cloned) from `identity.oauth_instance_for_shutdown`.
    /// The SIGTERM/SIGINT deregister path is the single owner.
    pub oauth_instance_for_shutdown: Option<(uuid::Uuid, sea_orm::DatabaseConnection)>,

    // ---- Controller identity ----
    /// Copied before components is moved into assemble.
    /// Used by `register_scheduler` under `#[cfg(feature = "embedded-scheduler")]`.
    #[cfg_attr(
        not(feature = "embedded-scheduler"),
        expect(
            dead_code,
            reason = "controller_id is only passed to register_scheduler when the embedded-scheduler feature is enabled"
        )
    )]
    pub controller_id: uuid::Uuid,

    // ---- Embedded-service fields ----
    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    pub builtin_host: crate::service_host::BuiltinServiceHost,
    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    pub controller_installation_id: uuid::Uuid,
    #[cfg(any(feature = "embedded-agent", feature = "embedded-ssh-agent"))]
    pub state_dir: PathBuf,

    // ---- Server options ----
    /// Resolved HTTPS bind address; consumed by `ServerOptions`.
    pub https_addr: SocketAddr,
    /// Optional static asset directory for the embedded frontend.
    pub static_dir: Option<PathBuf>,
    /// Port for the plain-HTTP PKI endpoint; `None` when disabled.
    pub pki_http_port: Option<u16>,
}

// ---------------------------------------------------------------------------
// CaTaskDeps — owned parameter struct for spawn_background_tasks
// ---------------------------------------------------------------------------

/// Owned CA-related parameters for [`spawn_background_tasks`].
///
/// Replaces the previous 8-argument signature that required a
/// `#[expect(clippy::too_many_arguments)]` suppression.  All fields carry
/// owned values (`Arc` clones or `Copy` scalars) — no lifetime parameter.
pub(super) struct CaTaskDeps {
    pub crl_manager: Arc<crate::crl_manager::CrlManager>,
    pub ca_managed: bool,
    pub ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    pub initial_ca_version: i64,
    pub has_external_tls_cert: bool,
}

// ---------------------------------------------------------------------------
// Helper: spawn_background_tasks
// ---------------------------------------------------------------------------

/// Spawn background tasks: CRL manager, denylist cleanup, CA reload/rotation,
/// server cert renewal, and NATS consumer. Embedded service registration is
/// handled by the caller after this function returns.
pub(super) async fn spawn_background_tasks(
    bg: &mut crate::tasks::BackgroundTasks,
    app_state: &Arc<AppState>,
    ca: CaTaskDeps,
    #[cfg(feature = "nats")]
    service_connections: &uptrakit_web_api::service_connections::ServiceConnectionRegistry,
    #[cfg(feature = "nats")] nats_transport: &Option<
        uptrakit_web_api::nats_transport::NatsTransport,
    >,
) {
    // CRL manager: uses the child cancellation token for cooperative shutdown.
    // Must use track() (not track_abort()) so the manager finishes its current
    // cycle and writes the final TLS config before the process exits.
    let crl_handle = tokio::spawn(Arc::clone(&ca.crl_manager).run(Some(bg.child_token())));
    bg.track("crl-manager", crl_handle);

    // Token denylist cleanup (in-memory, per-instance — not in scheduler)
    let h = crate::tasks::spawn_denylist_cleanup(
        bg.child_token(),
        Arc::clone(&app_state.auth.token_denylist),
    );
    bg.track("denylist-cleanup", h);

    if ca.ca_managed {
        let h = crate::tasks::spawn_ca_reload(
            bg.child_token(),
            Arc::clone(app_state),
            ca.ca_tx.clone(),
            Arc::clone(&ca.crl_manager),
            ca.initial_ca_version,
        );
        bg.track("ca-reload", h);
    }

    if ca.ca_managed {
        let h = crate::tasks::spawn_ca_rotation(
            bg.child_token(),
            Arc::clone(app_state),
            ca.ca_tx.clone(),
            Arc::clone(&ca.crl_manager),
        );
        bg.track("ca-rotation", h);
    }

    if !ca.has_external_tls_cert {
        let h = crate::tasks::spawn_server_cert_renewal(
            bg.child_token(),
            Arc::clone(app_state),
            Arc::clone(&ca.crl_manager),
        );
        bg.track("server-cert-renewal", h);
    }

    // NATS consumer (cross-controller event delivery)
    #[cfg(feature = "nats")]
    if let Some(ref nats) = *nats_transport {
        let h = crate::tasks::spawn_nats_consumer(
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
                access_engine: Some(Arc::clone(&app_state.access_engine)),
            },
        );
        bg.track("nats-consumer", h);
    }

    // Update reaper: absolute-deadline backstop that forces overdue in_progress
    // updates to a terminal Interrupted state. Detached (fire-and-forget),
    // mirroring `spawn_heartbeat`; the loop has no cancellation token.
    uptrakit_web_api::update_reaper::spawn_update_reaper(Arc::clone(app_state));
}

// ---------------------------------------------------------------------------
// Helper: spawn_zeroconf
// ---------------------------------------------------------------------------

/// Spawn the zeroconf mDNS advertiser if the feature is enabled and configured.
#[cfg(feature = "zeroconf")]
pub(super) fn spawn_zeroconf(
    bg: &mut crate::tasks::BackgroundTasks,
    app_state: &Arc<AppState>,
    https_addr: SocketAddr,
) {
    let zeroconf_settings = app_state.settings.zeroconf();
    if zeroconf_settings.enabled {
        let ca_snap = app_state.cert.ca_snapshot.borrow().clone();
        let zc_cancel = bg.child_token();
        let handle = tokio::spawn(crate::zeroconf::run_advertiser(
            zc_cancel,
            https_addr,
            ca_snap,
            zeroconf_settings,
        ));
        bg.track("zeroconf-advertiser", handle);
    }
}

// ---------------------------------------------------------------------------
// Helper: spawn_pki_http
// ---------------------------------------------------------------------------

/// Spawn the optional plain-HTTP PKI server on the given port.
///
/// `inherited` is a pre-bound socket to reuse on the reexec path; `None` on
/// cold start causes a fresh `bind(addr)`.
pub(super) fn spawn_pki_http(
    bg: &mut crate::tasks::BackgroundTasks,
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
        if let Err(e) = crate::server::run_pki_http(addr, app_state_for_pki, inherited).await {
            tracing::error!(error = ?e, "PKI HTTP server error");
        }
    });
    bg.track_abort("pki-http", pki_http_handle);
}

// ---------------------------------------------------------------------------
// serve::run — final boot phase
// ---------------------------------------------------------------------------

/// Final boot phase: spawn background tasks, register embedded services,
/// install signal handlers, start servers, and drive the shutdown loop.
// `info` is consumed only by the embedded agent / ssh-agent registration calls below, which are
// feature-gated. A `_info` rename is not viable: `clippy::used_underscore_binding` (workspace-deny)
// fires when those features ARE enabled and the underscored binding is used. The function-level
// `expect(unused_variables)`, gated to the builds where `info` is genuinely unused, is the
// idiomatic resolution under this lint set.
#[cfg_attr(
    not(any(feature = "embedded-agent", feature = "embedded-ssh-agent")),
    expect(
        unused_variables,
        reason = "info is only consumed by embedded agent and ssh-agent registration calls"
    )
)]
pub(crate) async fn run(
    state: Arc<AppState>,
    listeners: Listeners,
    deps: ServeDeps,
    info: &BuildInfo,
) -> crate::Result<()> {
    // Destructure listeners — each socket is consumed exactly once.
    let Listeners {
        https_std,
        pki_std_for_spawn,
        listener_count: _,
        first_listener_fd: _,
    } = listeners;

    // Spawn background tasks (reads crl_manager, ca_managed, ca_tx, etc. from deps).
    let mut bg = crate::tasks::BackgroundTasks::new(deps.shutdown_token);
    let ca_deps = CaTaskDeps {
        crl_manager: Arc::clone(&deps.crl_manager),
        ca_managed: deps.ca_managed,
        ca_tx: deps.ca_tx.clone(),
        initial_ca_version: deps.initial_ca_version,
        has_external_tls_cert: deps.has_external_tls_cert,
    };
    spawn_background_tasks(
        &mut bg,
        &state,
        ca_deps,
        #[cfg(feature = "nats")]
        &deps.service_connections,
        #[cfg(feature = "nats")]
        &deps.nats_transport,
    )
    .await;

    // Embedded service registration. Failures are fatal — a broken embedded
    // service should not leave the deployment in an indeterminate state.
    #[cfg(feature = "embedded-scheduler")]
    crate::service_host::builtins::register_scheduler(
        &deps.builtin_host,
        &state,
        &mut bg,
        deps.controller_id,
        deps.controller_installation_id,
        deps.ca_managed,
        &deps.ca_tx,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded scheduler: {e}"
        )))
    })?;

    #[cfg(feature = "embedded-agent")]
    crate::service_host::builtins::register_agent(
        &deps.builtin_host,
        &state,
        &mut bg,
        deps.controller_installation_id,
        deps.state_dir.clone(),
        None,
        info,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded agent: {e}"
        )))
    })?;

    #[cfg(feature = "embedded-ssh-agent")]
    crate::service_host::builtins::register_agent_ssh(
        &deps.builtin_host,
        &state,
        &mut bg,
        deps.controller_installation_id,
        deps.state_dir.clone(),
        info,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded SSH agent: {e}"
        )))
    })?;

    #[cfg(feature = "embedded-mqtt")]
    crate::service_host::builtins::register_mqtt(
        &deps.builtin_host,
        &state,
        &mut bg,
        deps.controller_installation_id,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded MQTT service: {e}"
        )))
    })?;

    // Set up signal handlers
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context_transform(|e| {
            AppError::Config(format!("failed to set up SIGTERM handler: {e}"))
        })?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context_transform(|e| AppError::Config(format!("failed to set up SIGINT handler: {e}")))?;

    // Spawn HTTPS server
    let server_handle = axum_server::Handle::new();
    let server_options = crate::server::ServerOptions {
        https_addr: deps.https_addr,
        rustls_config: state.server.rustls_config.clone(),
        app_state: Arc::clone(&state),
        static_dir: deps.static_dir,
        handle: server_handle.clone(),
        inherited_listener: Some(https_std),
    };
    let server_task = tokio::spawn(crate::server::run(server_options));

    // Spawn zeroconf mDNS advertiser if enabled
    #[cfg(feature = "zeroconf")]
    spawn_zeroconf(&mut bg, &state, deps.https_addr);

    // Spawn PKI HTTP server if needed
    spawn_pki_http(&mut bg, &state, deps.pki_http_port, pki_std_for_spawn);

    // Notify the service manager (and stdout-based supervisors) that all
    // servers are bound and the controller is ready to accept connections.
    crate::reexec::sd_notify::signal_ready();

    // Main event loop — wait for shutdown signal or server exit
    let mut server_task = server_task;
    let shutdown_reason = tokio::select! {
        result = &mut server_task => {
            match result {
                Ok(Ok(())) => {
                    tracing::info!("server task exited normally");
                    "server exit"
                }
                Ok(Err(e)) => {
                    tracing::error!(error = ?e, "server error");
                    return Err(e).context(AppError::Server)?;
                }
                Err(e) => {
                    tracing::error!(error = %e, "server task panicked");
                    "server panic"
                }
            }
        }
        _ = sigterm.recv() => {
            tracing::info!("received SIGTERM, initiating graceful shutdown");
            "SIGTERM"
        }
        _ = sigint.recv() => {
            tracing::info!("received SIGINT, initiating graceful shutdown");
            "SIGINT"
        }
    };

    // Graceful shutdown — 30 second default timeout
    tracing::info!(reason = shutdown_reason, "shutdown signal received");
    let shutdown_timeout = Duration::from_secs(30);
    bg.shutdown(server_handle, deps.service_connections, shutdown_timeout)
        .await;

    if let Some((instance_id, ref db)) = deps.oauth_instance_for_shutdown {
        deregister_oauth_instance(db, instance_id).await;
    }

    Ok(())
}
