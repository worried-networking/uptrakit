//! Controller boot sequence.
//!
//! This module owns the top-level `run_server` entry point that drives
//! every phase of controller startup, from config loading through to the
//! main event loop.  Helper functions (`build_audit_logger`,
//! `spawn_background_tasks`, etc.) remain in the crate root so that later
//! per-phase extraction tasks can move them independently.

pub(crate) mod app_state;
pub(crate) mod components;
pub(crate) mod config;
pub(crate) mod crypto;
pub(crate) mod directories;
pub(crate) mod identity;
pub(crate) mod listeners;
#[cfg(feature = "nats")]
pub(crate) mod nats;
pub(crate) mod persistence;
pub(crate) mod recovery;
pub(crate) mod reload;
pub(crate) mod settings;

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_build_info::BuildInfo;
use uptrakit_web_api::oauth::boot::deregister_oauth_instance;

use crate::AppError;

pub(crate) async fn run_server(args: crate::cli::Args, info: BuildInfo) -> crate::Result<()> {
    // Phase 0: Load TOML config, parse bootstrap env args, initialise tracing.
    let cfg = config::load(args, &info).await?;

    // Phase 1: Master key initialization — reads from --master-key-from or TOML
    // master_key as a fallback. The TOML value already carries the full source
    // string (file:, env:, or inline hex) so no prefix injection is needed.
    let crypto = crypto::init(&cfg)?;

    let config_path_for_coord = cfg.config_path.clone();

    // Phase 2: Application directories — use platform defaults (no CLI overrides).
    let layout = directories::resolve().await?;
    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    let controller_installation_id = layout.installation_id;

    // Phase 3: Database — URL and pool size from TOML [db].
    let db = persistence::open(&cfg, &layout).await?;

    // Phases 4/4b/4c/4d: master key verify, column AAD mappings, data key ring, ENC:v3 migration
    crypto::verify_and_migrate(&db.db).await?;

    // Phases 5, 6, 7, 7b, 7c, 8: load settings, reconcile, seed, validate
    let settings_bundle = settings::load_and_seed(&cfg, &db).await?;

    // Phase 8b: Claim inherited TCP sockets and pre-bind listeners (FD-atomic).
    //
    // This must happen before the coordinator block so that `listener_count` and
    // `https_std`/`pki_std_for_spawn` are in scope when the reexec hook is
    // constructed and when the server task is spawned.
    let listeners::Listeners {
        https_std,
        pki_std_for_spawn,
        listener_count,
        first_listener_fd,
    } = listeners::claim(&settings_bundle)?;

    // Phases 7d, 9, 10: OAuth boot, PKI/TLS init, cert_signer construction, JWT init.
    // identity::init borrows cfg (via runtime), db.db, and settings_bundle.reconciled,
    // so it runs before any of those are destructured.
    //
    // `mut` is needed so that `oauth_instance_for_shutdown` can be taken by
    // move into `ServeDeps` via `Option::take` after `reload::wire` has borrowed
    // `&identity` to clone it into the reexec hook.
    let mut identity = identity::init(
        &cfg.booted.runtime,
        &db.db,
        layout.app_dirs.config_dir(),
        layout.app_dirs.state_dir(),
        &settings_bundle.reconciled,
    )
    .await?;

    // Build all web-API components (stores, plugin catalog, audit, broadcasters, etc.).
    // Borrows cfg, db, settings_bundle, and crypto by reference so none of them
    // need to be destructured yet.
    let components = components::build(&cfg, &db, &settings_bundle, &crypto).await?;

    // Destructure cfg and db now that components::build has finished borrowing them.
    let booted = cfg.booted;
    let args = cfg.args;
    let persistence::Persistence {
        db: db_conn,
        url: db_url,
        default_tenant_id,
    } = db;

    // Destructure the settings bundle now that listener FDs are claimed and
    // components have been built.
    let settings::SettingsBundle {
        settings,
        reconciled,
        validated,
    } = settings_bundle;

    // Wire config-reload coordinator, spawn reconciler + audit bridge, and
    // return non-optional ReloadWiring.  This replaces the old 7-tuple of
    // Options and the trailing `match (Some(…), …)` block.
    //
    // reload::wire borrows &identity so it can clone oauth_instance_for_shutdown
    // into the ControllerReexecHook; the move into ServeDeps happens after.
    let reload = reload::wire(
        booted,
        config_path_for_coord.clone(),
        args.master_key_from.clone(),
        &components,
        listener_count,
        first_listener_fd,
        &identity,
        db_conn.clone(),
        db_url.clone(),
        #[cfg(feature = "nats")]
        &reconciled,
    )
    .await?;

    // Separate serve-needed handles BEFORE assemble consumes identity and
    // components by move.  Arc/Copy fields are cloned cheaply; the
    // oauth_instance_for_shutdown is MOVED (not cloned) via Option::take — the
    // SIGTERM/SIGINT deregister path is its single owner.
    let serve_deps = {
        #[cfg(any(
            feature = "embedded-scheduler",
            feature = "embedded-agent",
            feature = "embedded-ssh-agent",
            feature = "embedded-mqtt"
        ))]
        let builtin_host = crate::service_host::BuiltinServiceHost::new(Arc::clone(
            &components.plugins.embedded_host,
        ));

        app_state::ServeDeps {
            crl_manager: Arc::clone(&identity.pki.crl_manager),
            ca_tx: identity.pki.ca_tx.clone(),
            ca_managed: identity.pki.ca_managed,
            initial_ca_version: identity.pki.initial_ca_version,
            has_external_tls_cert: identity.pki.has_external_tls_cert,
            service_connections: components.service_connections.clone(),
            #[cfg(feature = "nats")]
            nats_transport: components.nats_transport.clone(),
            // MOVE (not clone): the SIGTERM/SIGINT deregister path is the
            // single owner.  reload::wire already ran (borrowing &identity),
            // so we can now take the value via Option::take.
            oauth_instance_for_shutdown: identity.oauth_instance_for_shutdown.take(),
            shutdown_token: components.shutdown_token.clone(),
            controller_id: components.controller_id,
            #[cfg(any(
                feature = "embedded-scheduler",
                feature = "embedded-agent",
                feature = "embedded-ssh-agent",
                feature = "embedded-mqtt"
            ))]
            builtin_host,
            #[cfg(any(
                feature = "embedded-scheduler",
                feature = "embedded-agent",
                feature = "embedded-ssh-agent",
                feature = "embedded-mqtt"
            ))]
            controller_installation_id,
            #[cfg(any(feature = "embedded-agent", feature = "embedded-ssh-agent"))]
            state_dir: layout.app_dirs.state_dir().to_path_buf(),
        }
    };

    // Assemble AppState — consumes identity (oauth_instance_for_shutdown is
    // now None after the take() above) and components by move.
    let app_state = app_state::assemble(
        settings,
        identity,
        components,
        reload,
        db_conn.clone(),
        default_tenant_id,
        #[cfg(feature = "test-utils")]
        config_path_for_coord.clone(),
        #[cfg(feature = "test-utils")]
        args.master_key_from.clone(),
        #[cfg(feature = "test-utils")]
        listener_count,
        #[cfg(feature = "test-utils")]
        first_listener_fd,
    )
    .await?;

    // Startup recovery: GitHub diagnostic, rollout cleanup, denylist seed.
    recovery::run(&app_state).await?;

    // Spawn background tasks (reads crl_manager, ca_managed, ca_tx, etc. from serve_deps).
    let mut bg = crate::tasks::BackgroundTasks::new(serve_deps.shutdown_token);
    crate::spawn_background_tasks(
        &mut bg,
        &app_state,
        &serve_deps.crl_manager,
        serve_deps.ca_managed,
        &serve_deps.ca_tx,
        serve_deps.initial_ca_version,
        serve_deps.has_external_tls_cert,
        #[cfg(feature = "nats")]
        &serve_deps.service_connections,
        #[cfg(feature = "nats")]
        &serve_deps.nats_transport,
    )
    .await;

    // Embedded service registration. Failures are fatal — a broken embedded
    // service should not leave the deployment in an indeterminate state.
    #[cfg(feature = "embedded-scheduler")]
    crate::service_host::builtins::register_scheduler(
        &serve_deps.builtin_host,
        &app_state,
        &mut bg,
        serve_deps.controller_id,
        serve_deps.controller_installation_id,
        serve_deps.ca_managed,
        &serve_deps.ca_tx,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded scheduler: {e}"
        )))
    })?;

    #[cfg(feature = "embedded-agent")]
    crate::service_host::builtins::register_agent(
        &serve_deps.builtin_host,
        &app_state,
        &mut bg,
        serve_deps.controller_installation_id,
        serve_deps.state_dir.clone(),
        None,
        &info,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded agent: {e}"
        )))
    })?;

    #[cfg(feature = "embedded-ssh-agent")]
    crate::service_host::builtins::register_agent_ssh(
        &serve_deps.builtin_host,
        &app_state,
        &mut bg,
        serve_deps.controller_installation_id,
        serve_deps.state_dir.clone(),
        &info,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded SSH agent: {e}"
        )))
    })?;

    #[cfg(feature = "embedded-mqtt")]
    crate::service_host::builtins::register_mqtt(
        &serve_deps.builtin_host,
        &app_state,
        &mut bg,
        serve_deps.controller_installation_id,
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
        https_addr: reconciled.https_addr,
        rustls_config: app_state.server.rustls_config.clone(),
        app_state: Arc::clone(&app_state),
        static_dir: validated.static_dir,
        handle: server_handle.clone(),
        inherited_listener: Some(https_std),
    };
    let server_task = tokio::spawn(crate::server::run(server_options));

    // Spawn zeroconf mDNS advertiser if enabled
    #[cfg(feature = "zeroconf")]
    crate::spawn_zeroconf(&mut bg, &app_state, reconciled.https_addr);

    // Spawn PKI HTTP server if needed
    crate::spawn_pki_http(
        &mut bg,
        &app_state,
        validated.pki_http_port,
        pki_std_for_spawn,
    );

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
    bg.shutdown(
        server_handle,
        serve_deps.service_connections,
        shutdown_timeout,
    )
    .await;

    if let Some((instance_id, ref db)) = serve_deps.oauth_instance_for_shutdown {
        deregister_oauth_instance(db, instance_id).await;
    }

    Ok(())
}
