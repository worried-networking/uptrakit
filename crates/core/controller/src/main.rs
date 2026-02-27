mod cert_signer;
mod cli;
mod crl_manager;
mod db;
mod db_migrate;
mod durations;
#[cfg(feature = "embed-frontend")]
mod embedded_frontend;
mod migration;
mod mtls_acceptor;
mod pki;
mod reconcile;
mod reencrypt;
#[cfg(feature = "embedded-scheduler")]
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
    uptrakit_crypto::CryptoError => AppError,
    |e| AppError::Config(e.to_string())
);

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = cli::Args::parse();
    if args.version {
        print_build_info();
        return std::process::ExitCode::SUCCESS;
    }

    if args.verbose > 3 {
        eprintln!(
            "warning: -vvvv or more has no additional effect; maximum verbosity is -vvv (trace)"
        );
    }
    let directive = match args.verbose {
        0 => "uptrakit_controller=info",
        1 => "uptrakit_controller=debug",
        2 => "uptrakit=debug",
        _ => "uptrakit=trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(directive.parse().expect("valid directive")),
        )
        .init();

    // Dispatch optional subcommands before entering the normal server path.
    if let Some(cli::ControllerCommand::DbMigrate(ref db_args)) = args.command {
        if let Err(report) = db_migrate::run(&args, db_args).await {
            eprintln!("db-migrate error: {report:?}");
            return std::process::ExitCode::FAILURE;
        }
        return std::process::ExitCode::SUCCESS;
    }

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
    let master_key_hex = startup::init_master_key(&args)?;

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
    let db_init = startup::init_database(&args, app_dirs.state_dir()).await?;
    let db_conn = db_init.conn;
    let db_url = db_init.url;
    let default_tenant_id = db_init.default_tenant.id;
    tracing::info!(%default_tenant_id, "loaded default tenant");

    // Phase 4: Master key verification (HA safety)
    startup::verify_master_key(&db_conn, default_tenant_id).await?;

    // Phase 4b: Re-encrypt legacy plaintext secrets
    reencrypt::reencrypt_legacy_plaintext(&db_conn).await;

    // Phase 5: Load settings
    let (settings, raw_settings, reg_token) = Settings::load(&db_conn, default_tenant_id)
        .await
        .context(AppError::Settings)?;
    if let Some(token) = reg_token {
        eprintln!("==========================================================");
        eprintln!("  No users found. Use this one-time registration token:");
        eprintln!("  {token}");
        eprintln!("==========================================================");
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
    #[cfg_attr(not(feature = "nats"), allow(unused_mut))]
    let mut notification_service = uptrakit_web_api::notification_service::NotificationService::new(
        service_connections.clone(),
        controller_id,
    );

    // NATS transport (optional, feature-gated)
    #[cfg(feature = "nats")]
    let nats_transport = if let Some(ref url) = args.nats_url {
        let nats = uptrakit_web_api::nats_transport::NatsTransport::connect(url, controller_id)
            .await
            .context_transform(|_| AppError::Config("NATS connection failed".to_string()))?;
        notification_service = notification_service.with_nats(nats.clone());
        Some(nats)
    } else {
        None
    };

    let token_denylist = Arc::new(uptrakit_web_api::auth::token_denylist::TokenDenylist::new());

    // Build credential sources for external services that need direct infrastructure access.
    let credential_sources = {
        #[cfg_attr(not(feature = "nats"), allow(unused_mut))]
        let mut sources = uptrakit_web_api::ServiceCredentialSources {
            db_url: Some(db_url),
            nats_url: None,
            master_key_hex: master_key_hex.map(uptrakit_internal_wire::SecretString::new),
        };
        #[cfg(feature = "nats")]
        if let Some(ref url) = args.nats_url {
            sources.nats_url = Some(url.clone());
        }
        sources
    };

    let builder = AppState::builder()
        .ca_snapshot(ca_rx)
        .ca_key_store(ca_key_store)
        .db(db_conn)
        .settings(settings)
        .cert_signer(cert_signer)
        .service_connections(service_connections.clone())
        .revocation_notify(revocation_notify)
        .jwt(Arc::new(jwt_manager))
        .device_flow_store(device_flow_store)
        .rate_limit_store(rate_limit_store)
        .pki_path(pki_path)
        .rustls_config(rustls_config.clone())
        .crl_pem_cache(crl_pem_cache)
        .ca_rotation_trigger(ca_rotation_trigger)
        .default_tenant_id(default_tenant_id)
        .controller_id(controller_id)
        .notification_service(notification_service)
        .token_denylist(token_denylist)
        .credential_sources(credential_sources);

    #[cfg(feature = "oidc")]
    let builder = builder
        .oidc_flow_store(oidc_flow_store)
        .account_link_store(account_link_store)
        .oidc_token_exchange_store(oidc_token_exchange_store)
        .oidc_registration_store(oidc_registration_store);

    let app_state = Arc::new(
        builder
            .build()
            .map_err(|e| report!(AppError::Config(format!("failed to build AppState: {e}"))))?,
    );

    // Spawn background tasks
    let mut bg = tasks::BackgroundTasks::new(CancellationToken::new());

    // CRL manager: uses the child cancellation token for cooperative shutdown.
    // Must use track() (not track_abort()) so the manager finishes its current
    // cycle and writes the final TLS config before the process exits.
    let crl_handle = tokio::spawn(Arc::clone(&crl_manager).run(Some(bg.child_token())));
    bg.track("crl-manager", crl_handle);

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

    // Centralised task scheduler (HA-safe via optimistic locking).
    // Only compiled when the `embedded-scheduler` feature is enabled.
    // When an external scheduler service is deployed, the controller does NOT
    // need this feature — the external scheduler handles all scheduled tasks.
    #[cfg(feature = "embedded-scheduler")]
    {
        use scheduler::ControllerSchedulerNotifier;
        use uptrakit_scheduler_engine::executors::*;
        use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;

        let notifier: std::sync::Arc<dyn uptrakit_scheduler_engine::SchedulerNotifier> =
            std::sync::Arc::new(ControllerSchedulerNotifier::new(
                app_state.notification_service.clone(),
                Arc::clone(&app_state.ca_rotation_trigger),
            ));

        let mut sched = uptrakit_scheduler_engine::Scheduler::new(
            app_state.db().clone(),
            uptrakit_scheduler_engine::SchedulerConfig::new(controller_id, default_tenant_id),
        );

        sched.register(
            ScheduledTaskType::AuthCleanup,
            Box::new(auth_cleanup::AuthCleanupExecutor::new(
                app_state.db().clone(),
            )),
        );
        sched.register(
            ScheduledTaskType::StaleLeaseCleanup,
            Box::new(stale_lease_cleanup::StaleLeaseCleanupExecutor::new(
                app_state.db().clone(),
            )),
        );
        if ca_managed {
            sched.register(
                ScheduledTaskType::CaRotationCheck,
                Box::new(scheduler::CaRotationCheckExecutor::new(
                    ca_tx.subscribe(),
                    Arc::clone(&app_state.ca_rotation_trigger),
                )),
            );
        }
        sched.register(
            ScheduledTaskType::VersionCheck,
            Box::new(version_check::VersionCheckExecutor::new(
                app_state.db().clone(),
                Arc::clone(&notifier),
            )),
        );
        sched.register(
            ScheduledTaskType::ServiceCertCheck,
            Box::new(service_cert_check::ServiceCertCheckExecutor::new(
                app_state.db().clone(),
                Arc::clone(&notifier),
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

    // NATS consumer (cross-controller event delivery)
    #[cfg(feature = "nats")]
    if let Some(ref nats) = nats_transport {
        let h = tasks::spawn_nats_consumer(
            bg.child_token(),
            nats.clone(),
            service_connections.clone(),
            app_state.db().clone(),
            Some(Arc::clone(&app_state.ca_rotation_trigger)),
        );
        bg.track("nats-consumer", h);
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
