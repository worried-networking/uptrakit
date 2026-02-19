mod cert_signer;
mod cli;
mod crl_manager;
mod db;
mod durations;
#[cfg(feature = "embed-frontend")]
mod embedded_frontend;
mod migration;
mod mtls_acceptor;
mod pki;
mod reconcile;
mod scheduler;
mod server;
mod startup;
mod tasks;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use thiserror::Error;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use uptrakit_build_info::BuildInfo;
use uptrakit_shared_macros::impl_report_conversion;

use uptrakit_web_api::AppState;
use uptrakit_web_api::settings::Settings;

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
    uptrakit_shared_db::crypto::CryptoError => AppError,
    |e| AppError::Config(e.to_string())
);

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = cli::Args::parse();
    if args.version {
        print_build_info();
        return std::process::ExitCode::SUCCESS;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    if let Err(report) = run(args).await {
        eprintln!("Error: {report:?}");
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

fn print_build_info() {
    let build_info = BuildInfo::current(
        "uptrakit-controller",
        env!("CARGO_PKG_VERSION"),
        option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
    );
    let output = build_info.render_human();
    print!("{output}");
}

async fn run(args: cli::Args) -> Result<()> {
    // Phase 1: Master key initialization
    startup::init_master_key(&args)?;

    // Phase 2: Application directories
    let app_dirs = args.resolve_dirs().map_err(|e| {
        report!(AppError::Config(format!(
            "failed to resolve directories: {e}"
        )))
    })?;
    app_dirs.ensure_dirs().await.map_err(|e| {
        report!(AppError::Config(format!(
            "failed to create directories: {e}"
        )))
    })?;
    tracing::info!("config directory: {}", app_dirs.config_dir().display());
    tracing::info!("state directory: {}", app_dirs.state_dir().display());

    // Phase 3: Database
    let (db_conn, default_tenant) = startup::init_database(&args, app_dirs.state_dir()).await?;
    let default_tenant_id = default_tenant.id;
    tracing::info!(%default_tenant_id, "loaded default tenant");

    // Phase 4: Master key verification (HA safety)
    startup::verify_master_key(&db_conn, default_tenant_id).await?;

    // Phase 5: Load settings
    let (settings, raw_settings, reg_token) = Settings::load(&db_conn, default_tenant_id)
        .await
        .context(AppError::Settings)?;
    if let Some(token) = reg_token {
        tracing::info!("==========================================================");
        tracing::info!("  No users found. Use this one-time registration token:");
        tracing::info!("  {}", token);
        tracing::info!("==========================================================");
    }

    // Phase 6: Reconcile settings
    let reconciled = startup::reconcile_all_settings(
        &db_conn,
        default_tenant_id,
        &args,
        &settings,
        &raw_settings,
    )
    .await?;

    // Phase 7: OIDC bootstrap
    startup::bootstrap_oidc(&db_conn, default_tenant_id, &args).await?;

    // Phase 8: Validate configuration
    let validated = startup::validate_configuration(&args, &reconciled)?;

    // Phase 9: PKI + TLS
    let pki = startup::init_pki_runtime(
        &args,
        &db_conn,
        default_tenant_id,
        app_dirs.config_dir(),
        &reconciled,
    )
    .await?;

    // Phase 10: JWT
    let jwt_manager = startup::init_jwt(&db_conn, default_tenant_id, app_dirs.state_dir()).await?;

    // Destructure PKI runtime for distribution across AppState and tasks
    let startup::PkiRuntime {
        ca_managed,
        pki_path,
        ca_tx,
        ca_rx,
        ca_key_store,
        rustls_config,
        revocation_notify,
        ca_rotation_trigger,
        crl_pem_cache,
        crl_manager,
        initial_ca_version,
    } = pki;

    // Build shared application state
    let cert_signer = Arc::new(cert_signer::RcgenAgentCertSigner::new(
        ca_rx.clone(),
        Arc::clone(&ca_key_store),
    ));

    #[cfg(feature = "oidc")]
    let oidc_flow_store = uptrakit_web_api::auth::oidc_state::OidcFlowStore::new(db_conn.clone());
    #[cfg(feature = "oidc")]
    let account_link_store =
        uptrakit_web_api::auth::oidc_state::AccountLinkStore::new(db_conn.clone());
    #[cfg(feature = "oidc")]
    let oidc_token_exchange_store =
        uptrakit_web_api::auth::oidc_state::OidcTokenExchangeStore::new(db_conn.clone());
    #[cfg(feature = "oidc")]
    let oidc_registration_store =
        uptrakit_web_api::auth::oidc_state::OidcRegistrationStore::new(db_conn.clone());
    let device_flow_store =
        uptrakit_web_api::auth::device_flow::DeviceFlowStore::new(db_conn.clone());
    let rate_limit_store = uptrakit_web_api::auth::rate_limit::RateLimitStore::new(db_conn.clone());

    let service_connections =
        uptrakit_web_api::service_connections::ServiceConnectionRegistry::new();
    let controller_id = uuid::Uuid::now_v7();
    let notification_service = uptrakit_web_api::notification_service::NotificationService::new(
        db_conn.clone(),
        service_connections.clone(),
        controller_id,
    );
    let token_denylist = Arc::new(uptrakit_web_api::auth::token_denylist::TokenDenylist::new());

    let app_state = Arc::new(AppState {
        ca_snapshot: ca_rx,
        ca_key_store,
        db: db_conn,
        settings,
        cert_signer,
        service_connections: service_connections.clone(),
        revocation_notify,
        #[cfg(feature = "oidc")]
        oidc_flow_store,
        #[cfg(feature = "oidc")]
        account_link_store,
        jwt: Arc::new(jwt_manager),
        #[cfg(feature = "oidc")]
        oidc_token_exchange_store,
        #[cfg(feature = "oidc")]
        oidc_registration_store,
        device_flow_store,
        rate_limit_store,
        pki_path,
        rustls_config: rustls_config.clone(),
        crl_pem_cache,
        ca_rotation_trigger,
        default_tenant_id,
        controller_id,
        notification_service,
        token_denylist,
    });

    // Spawn background tasks
    let mut bg = tasks::BackgroundTasks::new(CancellationToken::new());

    // CRL manager (uses child token for graceful exit, abort as safety net)
    let crl_handle = tokio::spawn(Arc::clone(&crl_manager).run(Some(bg.child_token())));
    bg.track_abort("crl-manager", crl_handle);

    // Token denylist cleanup (in-memory, per-instance — not in scheduler)
    let h = tasks::spawn_denylist_cleanup(bg.child_token(), Arc::clone(&app_state.token_denylist));
    bg.track("denylist-cleanup", h);

    let h = tasks::spawn_settings_reload(bg.child_token(), Arc::clone(&app_state));
    bg.track("settings-reload", h);

    if ca_managed {
        let h = tasks::spawn_ca_reload(
            bg.child_token(),
            Arc::clone(&app_state),
            ca_tx.clone(),
            Arc::clone(&crl_manager),
            initial_ca_version,
        );
        bg.track("ca-reload", h);
    }

    let h = tasks::spawn_event_poller(bg.child_token(), Arc::clone(&app_state));
    bg.track("event-poller", h);

    // Centralised task scheduler (HA-safe via optimistic locking)
    {
        use scheduler::executors::*;
        use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;

        let mut sched = scheduler::Scheduler::new(
            app_state.db.clone(),
            scheduler::SchedulerConfig::new(controller_id, default_tenant_id),
        );

        sched.register(
            ScheduledTaskType::AuthCleanup,
            Box::new(auth_cleanup::AuthCleanupExecutor::new(
                #[cfg(feature = "oidc")]
                app_state.oidc_flow_store.clone(),
                #[cfg(feature = "oidc")]
                app_state.account_link_store.clone(),
                #[cfg(feature = "oidc")]
                app_state.oidc_token_exchange_store.clone(),
                #[cfg(feature = "oidc")]
                app_state.oidc_registration_store.clone(),
                app_state.device_flow_store.clone(),
                app_state.rate_limit_store.clone(),
            )),
        );
        sched.register(
            ScheduledTaskType::StaleLeaseCleanup,
            Box::new(stale_lease_cleanup::StaleLeaseCleanupExecutor::new(
                app_state.db.clone(),
                service_connections.clone(),
            )),
        );
        sched.register(
            ScheduledTaskType::EventCleanup,
            Box::new(event_cleanup::EventCleanupExecutor::new(
                app_state.db.clone(),
            )),
        );
        if ca_managed {
            sched.register(
                ScheduledTaskType::CaRotationCheck,
                Box::new(ca_rotation_check::CaRotationCheckExecutor::new(
                    ca_tx.subscribe(),
                    Arc::clone(&app_state.ca_rotation_trigger),
                )),
            );
        }
        sched.register(
            ScheduledTaskType::VersionCheck,
            Box::new(version_check::VersionCheckExecutor::new(
                app_state.db.clone(),
                app_state.notification_service.clone(),
            )),
        );
        sched.register(
            ScheduledTaskType::ServiceCertCheck,
            Box::new(service_cert_check::ServiceCertCheckExecutor::new(
                app_state.db.clone(),
                app_state.notification_service.clone(),
            )),
        );

        let h = tokio::spawn(sched.run(bg.child_token()));
        bg.track("scheduler", h);
    }

    if ca_managed {
        let h = tasks::spawn_ca_rotation(
            bg.child_token(),
            Arc::clone(&app_state),
            ca_tx,
            Arc::clone(&crl_manager),
        );
        bg.track("ca-rotation", h);
    }

    if args.tls_cert.is_none() {
        let h = tasks::spawn_server_cert_renewal(
            bg.child_token(),
            Arc::clone(&app_state),
            Arc::clone(&crl_manager),
        );
        bg.track("server-cert-renewal", h);
    }

    // Set up signal handlers
    let mut sigterm = signal(SignalKind::terminate()).context_transform(|e| {
        AppError::Config(format!("failed to set up SIGTERM handler: {e}"))
    })?;
    let mut sigint = signal(SignalKind::interrupt())
        .context_transform(|e| AppError::Config(format!("failed to set up SIGINT handler: {e}")))?;
    let mut sigusr1 = signal(SignalKind::user_defined1()).context_transform(|e| {
        AppError::Config(format!("failed to set up SIGUSR1 handler: {e}"))
    })?;

    // Spawn HTTPS server
    let server_handle = axum_server::Handle::new();
    let server_options = server::ServerOptions {
        https_addr: reconciled.https_addr,
        rustls_config,
        app_state: Arc::clone(&app_state),
        static_dir: validated.static_dir,
        handle: server_handle.clone(),
        enable_reuseport: args.reuseport,
    };
    let server_task = tokio::spawn(server::run(server_options));

    // If taking over, signal old process after server is ready
    if let Some(old_pid) = args.takeover_from {
        tokio::time::sleep(Duration::from_millis(100)).await;
        match nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(old_pid as i32),
            nix::sys::signal::Signal::SIGUSR1,
        ) {
            Ok(()) => tracing::info!(pid = old_pid, "sent SIGUSR1 to old process"),
            Err(e) => tracing::warn!(pid = old_pid, error = %e, "failed to signal old process"),
        }
    }

    // Spawn PKI HTTP server if needed
    if let Some(port) = validated.pki_http_port {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let app_state_for_pki = Arc::clone(&app_state);
        let pki_http_handle = tokio::spawn(async move {
            if let Err(e) = server::run_pki_http(addr, app_state_for_pki).await {
                tracing::error!(error = ?e, "PKI HTTP server error");
            }
        });
        bg.track_abort("pki-http", pki_http_handle);
    }

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
        _ = sigusr1.recv() => {
            tracing::info!("received SIGUSR1 (new process ready), initiating graceful shutdown");
            "SIGUSR1 (takeover)"
        }
    };

    // Graceful shutdown
    tracing::info!(reason = shutdown_reason, "shutdown signal received");
    let shutdown_timeout = Duration::from_secs(args.shutdown_timeout_secs);
    bg.shutdown(server_handle, service_connections, shutdown_timeout)
        .await;

    Ok(())
}
