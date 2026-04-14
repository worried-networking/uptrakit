#[cfg(feature = "embedded-agent")]
mod agent;
mod cert_signer;
mod cli;
mod crl_manager;
mod db;
mod db_migrate;
mod durations;
#[cfg_attr(
    not(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    )),
    allow(dead_code) // Infrastructure types used by follow-up service embeddings.
)]
mod embedded;
#[cfg(feature = "embed-frontend")]
mod embedded_frontend;
mod migration;
#[cfg(feature = "embedded-mqtt")]
mod mqtt;
mod mtls_acceptor;
mod pki;
mod reconcile;
mod reencrypt;
#[cfg(feature = "embedded-scheduler")]
mod scheduler;
mod server;
mod service_host;
#[cfg(feature = "embedded-ssh-agent")]
mod ssh_agent;
mod startup;
mod tasks;
#[cfg(feature = "zeroconf")]
mod zeroconf;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use thiserror::Error;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
#[cfg(feature = "journald")]
use tracing_subscriber::prelude::*;
use uptrakit_audit_log::{AuditFilter, AuditLogDispatcher};
use uptrakit_build_info::BuildInfo;
#[cfg(feature = "embedded-scheduler")]
use uptrakit_internal_wire::surfaces;
use uptrakit_plugin_infrastructure_registry::{PluginHttpClientConfig, build_plugin_http_client};
use uptrakit_shared_macros::impl_report_conversion;

use uptrakit_web_api::AppState;
use uptrakit_web_api::settings::Settings;
use uptrakit_web_api::{SurfaceRuntimeRolloutState, default_surface_runtime_requirements};

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

    #[allow(unused_mut)] // mutated inside #[cfg(feature = "journald")] block
    let mut builder = uptrakit_tracing_init::TracingBuilder::new()
        .verbosity(args.verbose)
        .max_verbosity(3)
        .directives_for_verbosity(
            0,
            &[
                ("uptrakit_controller", "info"),
                ("uptrakit_web_api", "info"),
            ],
        )
        .directives_for_verbosity(
            1,
            &[
                ("uptrakit_controller", "debug"),
                ("uptrakit_web_api", "debug"),
            ],
        )
        .directives_for_verbosity(2, &[("uptrakit", "debug")])
        .directives_for_verbosity(3, &[("uptrakit", "trace")]);

    // When the journald audit backend is selected, add a dedicated journald
    // tracing layer filtered to the `uptrakit_audit` target so that structured
    // audit events reach the system journal alongside normal stdout logging.
    #[cfg(feature = "journald")]
    if args
        .audit_log_backend
        .contains(&cli::AuditLogBackendArg::Journald)
    {
        let journald = tracing_journald::layer()
            .expect("failed to connect to journald")
            .with_filter(tracing_subscriber::EnvFilter::new("uptrakit_audit=info"));
        builder = builder.extra_layer(Box::new(journald));
    }

    builder.init();

    // Dispatch optional subcommands before entering the normal server path.
    if let Some(cli::ControllerCommand::DbMigrate(ref db_args)) = args.command {
        if let Err(report) = db_migrate::run(&args, db_args).await {
            eprintln!("db-migrate error: {report:?}");
            return std::process::ExitCode::FAILURE;
        }
        return std::process::ExitCode::SUCCESS;
    }

    if let Err(report) = run(args).await {
        eprintln!("Error:\n{report}");
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
    #[cfg_attr(
        not(any(feature = "embedded-scheduler", feature = "embedded-agent")),
        allow(unused_variables)
    )]
    let controller_installation_id = startup::init_installation_id(app_dirs.state_dir()).await?;

    // Phase 3: Database
    let db_init = startup::init_database(&args, app_dirs.state_dir()).await?;
    let db_conn = db_init.conn;
    let db_url = db_init.url;
    let default_tenant_id = db_init.default_tenant.id;
    tracing::info!(%default_tenant_id, "loaded default tenant");

    // Phase 4: Master key verification (HA safety)
    startup::verify_master_key(&db_conn).await?;

    // Phase 4b: Register column AAD mappings (enables ENC:v2/v3 read support)
    reencrypt::register_column_aad_mappings();

    // Phase 4c: Initialize data key ring (envelope encryption)
    startup::init_data_key_ring(&db_conn).await?;

    // Phase 4d: Migrate all encrypted values to ENC:v3 (automatic)
    reencrypt::reencrypt_to_v3(&db_conn).await;

    // Phase 4e: Master key rotation (operator-triggered)
    if let Some(ref new_key_path) = args.rotate_master_key_file {
        startup::rotate_master_key(&db_conn, new_key_path).await?;
    }

    // Phase 5: Load settings
    let (settings, global_raw, _tenant_raw, reg_token) =
        Settings::load(&db_conn, default_tenant_id)
            .await
            .context(AppError::Settings)?;
    if let Some(token) = reg_token {
        eprintln!("==========================================================");
        eprintln!("  No users found. Use this one-time registration token:");
        eprintln!("  {token}");
        eprintln!("==========================================================");
    }

    // Phase 6: Reconcile settings
    let reconciled =
        startup::reconcile_all_settings(&db_conn, &args, &settings, &global_raw).await?;

    // Phase 7: OIDC bootstrap
    startup::bootstrap_oidc(&db_conn, default_tenant_id, &args).await?;

    // Phase 7b: Enrollment token bootstrap
    startup::bootstrap_enrollment_tokens(&db_conn, default_tenant_id, &args).await?;

    // Phase 8: Validate configuration
    let validated = startup::validate_configuration(&args, &reconciled)?;

    // Phase 9: PKI + TLS
    let pki =
        startup::init_pki_runtime(&args, &db_conn, app_dirs.config_dir(), &reconciled).await?;

    // Phase 10: JWT
    let jwt_manager = startup::init_jwt(&db_conn, app_dirs.state_dir()).await?;

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
    let workload_claim_registry =
        Arc::new(uptrakit_web_api::workload_claims::WorkloadClaimRegistry::new());
    #[cfg_attr(not(feature = "nats"), allow(unused_mut))]
    let mut notification_service =
        uptrakit_web_api::notification_service::NotificationService::new(
            service_connections.clone(),
            controller_id,
        )
        .with_claim_registry(Arc::clone(&workload_claim_registry));

    // NATS transport (optional, feature-gated)
    // Uses the reconciled NATS URL (DB value wins over CLI; CLI seeds DB on first run).
    #[cfg(feature = "nats")]
    let nats_transport = if let Some(ref url) = reconciled.nats_url {
        let nats = uptrakit_web_api::nats_transport::NatsTransport::connect(url, controller_id)
            .await
            .context_transform(|e| {
                use uptrakit_web_api::nats_transport::NatsTransportError;
                match e {
                    NatsTransportError::Connection(msg) => {
                        AppError::Config(format!("NATS connection failed: {msg}"))
                    }
                    NatsTransportError::JetStream(msg) => AppError::Config(format!(
                        "NATS JetStream setup failed: {msg}\n\
                         Ensure JetStream is enabled on the NATS server: start with the \
                         -js flag, or add `jetstream: {{enabled: true}}` to nats-server.conf"
                    )),
                    _ => AppError::Config("NATS initialization failed".to_string()),
                }
            })?;
        notification_service = notification_service.with_nats(nats.clone());
        Some(nats)
    } else {
        None
    };

    // Build the batch progress broadcaster with NATS for cross-instance SSE.
    // When NATS is not configured the broadcaster operates in single-instance mode.
    let batch_progress_broadcaster =
        uptrakit_web_api::batch_progress_broadcaster::BatchProgressBroadcaster::new();
    #[cfg(feature = "nats")]
    let batch_progress_broadcaster = if let Some(ref nats) = nats_transport {
        batch_progress_broadcaster.with_nats(nats.nats_client())
    } else {
        batch_progress_broadcaster
    };

    // Build the admin event broadcaster with NATS for cross-instance SSE fan-out.
    // When NATS is not configured the broadcaster operates in single-instance mode.
    #[cfg_attr(not(feature = "nats"), allow(unused_mut))]
    let mut event_broadcaster = uptrakit_web_api::event_broadcaster::EventBroadcaster::new();
    #[cfg(feature = "nats")]
    if let Some(ref nats) = nats_transport {
        event_broadcaster = event_broadcaster.with_nats(nats.clone(), controller_id);
    }

    let token_denylist = Arc::new(
        uptrakit_web_api::auth::token_denylist::TokenDenylist::new_with_db(db_conn.clone()),
    );

    // Shared cancellation token: cancelled by BackgroundTasks::shutdown(), which
    // also signals open SSE streams in the web API to terminate cleanly.
    let shutdown_token = CancellationToken::new();

    // Build the plugin catalog from all compiled-in descriptors.
    // The catalog replaces the old PluginRegistry and provides PluginOps.
    let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig {
        allow_private_urls: args.allow_private_notification_urls,
        http_client: Some(
            build_plugin_http_client(PluginHttpClientConfig {
                user_agent: "uptrakit-controller",
                redirect_policy: reqwest::redirect::Policy::limited(5),
                ..Default::default()
            })
            .map_err(|e| report!(AppError::Config(format!("plugin catalog HTTP client: {e}"))))?,
        ),
        cancellation_token: Some(shutdown_token.clone()),
    };
    let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(&catalog_config)
        .context_transform(|_| AppError::Config("failed to build plugin catalog".to_string()))?;

    let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(catalog);

    let callback_base_url = format!("https://{}", reconciled.https_addr);
    let notification_dispatcher =
        uptrakit_web_api::notifications::dispatcher::NotificationDispatcher::new(
            db_conn.clone(),
            Arc::clone(&plugin_ops),
            callback_base_url,
        );

    // Build credential sources for external services that need direct infrastructure access.
    let credential_sources = {
        #[cfg_attr(not(feature = "nats"), allow(unused_mut))]
        let mut sources = uptrakit_web_api::ServiceCredentialSources {
            db_url: Some(db_url),
            nats_url: None,
            master_key_hex,
        };
        #[cfg(feature = "nats")]
        if let Some(ref url) = reconciled.nats_url {
            sources.nats_url = Some(url.clone());
        }
        sources
    };

    // Audit log backend and filter wiring.
    let (audit_filter, audit_dispatcher) = build_audit_logger(&args, &db_conn).await?;

    // Seed the extension registry with plugin-provided manifests paired with
    // their per-plugin action catalogues (including notification plugin
    // extensions aggregated by the unified plugin_ops). Using the paired form
    // ensures each extension resolves only its own actions so that
    // `resolveAction("create")` on notifications.telegram does not return
    // webhook's "Add Webhook" action.
    let extension_entries = plugin_ops.extension_manifests_and_actions();

    let surface_registry = Arc::new(uptrakit_web_api::surface_registry::SurfaceRegistry::new(
        uptrakit_web_api::surface_registry::SurfaceRegistryConfig::default(),
    ));
    surface_registry
        .bootstrap_builtin(build_controller_builtin_surface_registration())
        .map_err(|error| {
            report!(AppError::Config(format!(
                "failed to bootstrap built-in surfaces: {error}"
            )))
        })?;
    for registration in plugin_ops.surface_registrations() {
        let provider_id = registration.provider.provider_id.clone();
        surface_registry
            .bootstrap_plugin(registration)
            .map_err(|error| {
                report!(AppError::Config(format!(
                    "failed to bootstrap plugin surfaces for provider {provider_id}: {error}"
                )))
            })?;
    }
    let surface_proxy = Arc::new(
        uptrakit_web_api::surface_proxy::SurfaceProxy::new().with_local_executor(Arc::new(
            uptrakit_web_api::surface_proxy::PluginSurfaceLocalExecutor::new(
                Arc::new(db_conn.clone()),
                Arc::new(
                    uptrakit_web_api::surface_proxy::PluginOpsSurfaceActionInvoker::new(
                        Arc::clone(&plugin_ops),
                    ),
                ),
            ),
        )),
    );

    let surface_runtime_rollout =
        build_surface_runtime_rollout_state_for_phase0(args.surface_runtime_rollout);
    log_surface_runtime_rollout_state(&surface_runtime_rollout);

    // Create the embedded service host before AppState so it can be stored
    // in the state. The host's `add()` is called later in spawn_background_tasks.
    let embedded_host = Arc::new(embedded::EmbeddedServiceHost::new());
    let builtin_host = service_host::BuiltinServiceHost::new(Arc::clone(&embedded_host));

    let builder = AppState::builder()
        .ca_snapshot(ca_rx)
        .ca_key_store(ca_key_store)
        .db(db_conn)
        .settings(settings)
        .cert_signer(cert_signer)
        .service_connections(service_connections.clone())
        .revocation_notify(revocation_notify)
        .embedded_service_notifier(
            Arc::clone(&embedded_host) as Arc<dyn uptrakit_web_api::EmbeddedServiceNotifier>
        )
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
        .notification_dispatcher(notification_dispatcher)
        .token_denylist(token_denylist)
        .credential_sources(credential_sources)
        .event_broadcaster(event_broadcaster.clone())
        .batch_progress_broadcaster(batch_progress_broadcaster)
        .shutdown_token(shutdown_token.clone())
        .audit_log_filter(audit_filter)
        .audit_log_dispatcher(audit_dispatcher)
        .plugin_ops(plugin_ops)
        .surface_registry(surface_registry)
        .surface_proxy(surface_proxy)
        .workload_claim_registry(workload_claim_registry)
        .reject_dangerous_commands(!args.allow_dangerous_commands)
        .surface_runtime_rollout(surface_runtime_rollout);

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

    let recovered =
        uptrakit_web_api::queries::update_batches::mark_all_in_progress_as_failed_for_rollout(
            app_state.db(),
        )
        .await
        .map_err(|e| {
            report!(AppError::Config(format!(
                "failed to run owner-aware rollout cleanup: {e}"
            )))
        })?;

    if !recovered.is_empty() {
        tracing::warn!(
            count = recovered.len(),
            "marked pre-existing in-progress updates as failed during owner-aware rollout cleanup"
        );

        for record in &recovered {
            if let Some(batch_id) = record.batch_id {
                match uptrakit_web_api::queries::update_batches::dispatch_next_in_batch(
                    app_state.db(),
                    &app_state.notification.notification_service,
                    batch_id,
                    record.host_id,
                    record.tenant_id,
                )
                .await
                {
                    Ok(Some(completion)) => {
                        tracing::debug!(
                            %batch_id,
                            status = %completion.status.as_str(),
                            completed = completion.completed_count,
                            failed = completion.failed_count,
                            "startup rollout cleanup intentionally does not replay retroactive batch-completion notifications"
                        );
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            %batch_id,
                            host_id = %record.host_id,
                            "failed to promote next queued batch item after rollout cleanup"
                        );
                    }
                }
            } else if let Err(error) =
                uptrakit_web_api::queries::update_batches::dispatch_next_queued_for_host(
                    app_state.db(),
                    &app_state.notification.notification_service,
                    record.host_id,
                    record.tenant_id,
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    host_id = %record.host_id,
                    "failed to dispatch next queued update after rollout cleanup"
                );
            }
        }
    }

    // Seed the in-memory token denylist from DB before accepting traffic.
    // This ensures revocations made before a controller restart are honoured.
    app_state
        .auth
        .token_denylist
        .load_from_db()
        .await
        .map_err(|e| {
            report!(AppError::Config(format!(
                "failed to seed token denylist: {e}"
            )))
        })?;

    // Spawn background tasks
    let mut bg = tasks::BackgroundTasks::new(shutdown_token);
    spawn_background_tasks(
        &mut bg,
        &app_state,
        &crl_manager,
        ca_managed,
        ca_tx,
        initial_ca_version,
        controller_id,
        controller_installation_id,
        args.tls_cert.is_some(),
        &service_connections,
        &builtin_host,
        app_dirs.state_dir().to_path_buf(),
        #[cfg(feature = "nats")]
        &nats_transport,
    )
    .await;

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

    // Coordinate takeover from old process if requested
    handle_server_takeover(args.takeover_from, &server_handle).await;

    // Spawn zeroconf mDNS advertiser if enabled
    #[cfg(feature = "zeroconf")]
    spawn_zeroconf(&mut bg, &app_state, reconciled.https_addr);

    // Spawn PKI HTTP server if needed
    spawn_pki_http(&mut bg, &app_state, validated.pki_http_port);

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

// ---------------------------------------------------------------------------
// Extracted helper functions
// ---------------------------------------------------------------------------

fn build_surface_runtime_rollout_state_for_phase0(
    rollout_requested: bool,
) -> SurfaceRuntimeRolloutState {
    SurfaceRuntimeRolloutState::phase0(
        rollout_requested,
        default_surface_runtime_requirements(false),
        std::collections::BTreeMap::new(),
    )
}

fn log_surface_runtime_rollout_state(rollout: &SurfaceRuntimeRolloutState) {
    let snapshot = rollout.snapshot();
    let required_provider_apps: Vec<&str> = snapshot
        .required_providers
        .iter()
        .map(|provider| provider.app_name.as_str())
        .collect();
    let locally_satisfied_required_provider_apps: Vec<&str> = snapshot
        .required_providers
        .iter()
        .filter(|provider| provider.locally_satisfied)
        .map(|provider| provider.app_name.as_str())
        .collect();

    tracing::info!(
        rollout_requested = snapshot.rollout_requested,
        guard_satisfied = snapshot.guard_satisfied,
        active = snapshot.active,
        mode = ?snapshot.mode,
        required_provider_apps = ?required_provider_apps,
        locally_satisfied_required_provider_apps = ?locally_satisfied_required_provider_apps,
        reported_provider_count = snapshot.reported_provider_count,
        missing_required_providers = ?snapshot.missing_required_providers,
        incompatible_required_providers = ?snapshot.incompatible_required_providers,
        "surface runtime rollout evaluation",
    );

    if snapshot.rollout_requested && !snapshot.guard_satisfied {
        tracing::warn!(
            missing_required_providers = ?snapshot.missing_required_providers,
            incompatible_required_providers = ?snapshot.incompatible_required_providers,
            "surface runtime rollout requested but activation guard is not satisfied; keeping legacy runtime active",
        );
    }
}

fn build_controller_builtin_surface_registration() -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: "controller.builtin".to_string(),
            provider_kind: surfaces::ProviderKind::BuiltIn,
            provider_namespace: "controller".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("controller.builtin.info")
                    .expect("static built-in surface_id must be valid"),
                label: "Controller Built-in".to_string(),
                priority: 0,
                slot: surfaces::SLOT_SETTINGS_TABS.to_string(),
                scope: surfaces::Scope::Global,
                targeting: surfaces::Targeting::Universal,
                required_permission: None,
                provider_kind: surfaces::ProviderKind::BuiltIn,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::UniversalTargeting,
                ]),
                root_node: surfaces::SurfaceNode::TextBlock {
                    text: "Controller built-in surface runtime is active.".to_string(),
                },
            },
            interactions: Vec::new(),
            data_sources: Vec::new(),
        }],
        encryption_metadata: None,
    }
}

/// Build the audit log filter and dispatcher from CLI arguments.
///
/// Configures the audit backend (database, journald, or noop) and the
/// event filter (all, mutations-only, or none).
async fn build_audit_logger(
    args: &cli::Args,
    db_conn: &DatabaseConnection,
) -> Result<(AuditFilter, AuditLogDispatcher)> {
    use uptrakit_audit_log::{FilterMode, NoopBackend};

    let filter_mode = match args.audit_log_filter {
        cli::AuditLogFilterArg::All => FilterMode::All,
        cli::AuditLogFilterArg::Mutations => FilterMode::Mutations,
        cli::AuditLogFilterArg::None => FilterMode::None,
    };

    // Validate mutual exclusivity: `none` cannot be combined with other backends.
    let has_none = args
        .audit_log_backend
        .contains(&cli::AuditLogBackendArg::None);
    if has_none && args.audit_log_backend.len() > 1 {
        tracing::warn!(
            "--audit-log-backend=none is mutually exclusive with other backends; \
             disabling all audit logging"
        );
    }

    if has_none || filter_mode == FilterMode::None {
        let backend: std::sync::Arc<dyn uptrakit_audit_log::AuditLogBackend> =
            std::sync::Arc::new(NoopBackend);
        return Ok((
            AuditFilter::new(FilterMode::None),
            AuditLogDispatcher::new(backend),
        ));
    }

    // Build the database connection for the audit log backend.
    // Use the separate audit DB URL if provided, otherwise the main DB.
    let audit_db = if let Some(ref url) = args.audit_log_db_url {
        startup::init_audit_database(url, args.db_max_connections).await?
    } else {
        db_conn.clone()
    };

    let mut backends: Vec<std::sync::Arc<dyn uptrakit_audit_log::AuditLogBackend>> = Vec::new();

    for backend_arg in &args.audit_log_backend {
        match backend_arg {
            cli::AuditLogBackendArg::Db => {
                backends.push(std::sync::Arc::new(
                    uptrakit_audit_log::DatabaseBackend::new(audit_db.clone()),
                ));
            }
            #[cfg(feature = "journald")]
            cli::AuditLogBackendArg::Journald => {
                backends.push(std::sync::Arc::new(uptrakit_audit_log::JournaldBackend));
            }
            cli::AuditLogBackendArg::None => {
                // Already handled above (has_none guard).
            }
        }
    }

    let backend: std::sync::Arc<dyn uptrakit_audit_log::AuditLogBackend> = match backends.len() {
        0 => std::sync::Arc::new(NoopBackend),
        1 => backends.into_iter().next().expect("one backend"),
        _ => std::sync::Arc::new(uptrakit_audit_log::MultiplexBackend::new(backends)),
    };

    tracing::info!(
        filter = %filter_mode,
        backends = args.audit_log_backend.len(),
        "audit logging configured"
    );

    Ok((
        AuditFilter::new(filter_mode),
        AuditLogDispatcher::new(backend),
    ))
}

/// Spawn all background tasks: CRL manager, denylist cleanup, settings reload,
/// CA reload/rotation, scheduler, server cert renewal, and NATS consumer.
#[allow(clippy::too_many_arguments)]
// `controller_installation_id` is only used behind `embedded-scheduler`
// and `embedded-agent` feature flags; `has_external_tls_cert` is only used
// behind the `nats` feature flag.
#[allow(unused_variables)]
async fn spawn_background_tasks(
    bg: &mut tasks::BackgroundTasks,
    app_state: &Arc<AppState>,
    crl_manager: &Arc<crl_manager::CrlManager>,
    ca_managed: bool,
    ca_tx: tokio::sync::watch::Sender<pki::CaSnapshot>,
    initial_ca_version: i64,
    controller_id: uuid::Uuid,
    #[cfg_attr(
        not(any(
            feature = "embedded-scheduler",
            feature = "embedded-agent",
            feature = "embedded-mqtt"
        )),
        allow(unused_variables)
    )]
    controller_installation_id: uuid::Uuid,
    has_external_tls_cert: bool,
    service_connections: &uptrakit_web_api::service_connections::ServiceConnectionRegistry,
    builtin_host: &service_host::BuiltinServiceHost,
    state_dir: std::path::PathBuf,
    #[cfg(feature = "nats")] nats_transport: &Option<
        uptrakit_web_api::nats_transport::NatsTransport,
    >,
) {
    // Used only when embedded-scheduler or embedded-agent features are enabled.
    #[cfg(not(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-mqtt"
    )))]
    let _ = controller_installation_id;

    // CRL manager: uses the child cancellation token for cooperative shutdown.
    // Must use track() (not track_abort()) so the manager finishes its current
    // cycle and writes the final TLS config before the process exits.
    let crl_handle = tokio::spawn(Arc::clone(crl_manager).run(Some(bg.child_token())));
    bg.track("crl-manager", crl_handle);

    // Token denylist cleanup (in-memory, per-instance — not in scheduler)
    let h =
        tasks::spawn_denylist_cleanup(bg.child_token(), Arc::clone(&app_state.auth.token_denylist));
    bg.track("denylist-cleanup", h);

    let h = tasks::spawn_settings_reload(bg.child_token(), Arc::clone(app_state));
    bg.track("settings-reload", h);

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

    // Centralised task scheduler (HA-safe via optimistic locking).
    // Only compiled when the `embedded-scheduler` feature is enabled.
    // When an external scheduler service is deployed, the controller does NOT
    // need this feature — the external scheduler handles all scheduled tasks.
    #[cfg(feature = "embedded-scheduler")]
    {
        if let Err(e) = service_host::builtins::register_scheduler(
            builtin_host,
            app_state,
            bg,
            controller_id,
            controller_installation_id,
            ca_managed,
            &ca_tx,
        )
        .await
        {
            tracing::error!(error = %e, "failed to start embedded scheduler");
        }
    }

    // Embedded agent: run a local agent inside the controller process.
    // Only available in single-tenant deployments (uses default_tenant_id).
    #[cfg(feature = "embedded-agent")]
    {
        if let Err(e) = service_host::builtins::register_agent(
            builtin_host,
            app_state,
            bg,
            controller_installation_id,
            state_dir.clone(),
        )
        .await
        {
            tracing::error!(error = %e, "failed to start embedded agent");
        }
    }

    // Embedded SSH agent: manage remote hosts over SSH from within the controller.
    // Only available in single-tenant deployments (uses default_tenant_id).
    #[cfg(feature = "embedded-ssh-agent")]
    {
        if let Err(e) = service_host::builtins::register_agent_ssh(
            builtin_host,
            app_state,
            bg,
            controller_installation_id,
            state_dir.clone(),
        )
        .await
        {
            tracing::error!(error = %e, "failed to start embedded SSH agent");
        }
    }

    #[cfg(feature = "embedded-mqtt")]
    {
        if let Err(e) = service_host::builtins::register_mqtt(
            builtin_host,
            app_state,
            bg,
            controller_installation_id,
        )
        .await
        {
            tracing::error!(error = %e, "failed to start embedded mqtt");
        }
    }

    // Suppress unused-variable warnings when embedded features are disabled.
    let _ = controller_id;
    let _ = controller_installation_id;
    let _ = &state_dir;

    if ca_managed {
        let h = tasks::spawn_ca_rotation(
            bg.child_token(),
            Arc::clone(app_state),
            ca_tx,
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

    // Suppress unused-variable warnings when nats feature is disabled.
    let _ = &service_connections;
}

/// Wait for the HTTPS server to become ready, then signal the old controller
/// process via SIGUSR1 to begin its graceful shutdown.
async fn handle_server_takeover(
    old_pid: Option<u32>,
    server_handle: &axum_server::Handle<SocketAddr>,
) {
    let Some(old_pid) = old_pid else {
        return;
    };

    // Wait until the new server is actually listening before signaling the old
    // process. A 10-second timeout guards against a port conflict or slow TLS
    // init keeping us blocked indefinitely.
    match tokio::time::timeout(Duration::from_secs(10), server_handle.listening()).await {
        Ok(_) => tracing::info!("server is listening; signaling old process"),
        Err(_) => tracing::warn!(
            "timed out waiting for server to become ready (10s); \
             signaling old process anyway"
        ),
    }
    match nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(old_pid as i32),
        nix::sys::signal::Signal::SIGUSR1,
    ) {
        Ok(()) => tracing::info!(pid = old_pid, "sent SIGUSR1 to old process"),
        Err(e) => tracing::warn!(pid = old_pid, error = %e, "failed to signal old process"),
    }
}

/// Spawn the zeroconf mDNS advertiser if the feature is enabled and configured.
#[cfg(feature = "zeroconf")]
fn spawn_zeroconf(
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

/// Spawn the optional plain-HTTP PKI server on the given port.
fn spawn_pki_http(
    bg: &mut tasks::BackgroundTasks,
    app_state: &Arc<AppState>,
    pki_http_port: Option<u16>,
) {
    let Some(port) = pki_http_port else {
        return;
    };
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let app_state_for_pki = Arc::clone(app_state);
    let pki_http_handle = tokio::spawn(async move {
        if let Err(e) = server::run_pki_http(addr, app_state_for_pki).await {
            tracing::error!(error = ?e, "PKI HTTP server error");
        }
    });
    bg.track_abort("pki-http", pki_http_handle);
}

#[cfg(test)]
mod surface_rollout_tests {
    #[test]
    fn surface_rollout_phase0_blocks_activation_without_reports() {
        let rollout = super::build_surface_runtime_rollout_state_for_phase0(true);
        let snapshot = rollout.snapshot();
        assert!(snapshot.rollout_requested);
        assert!(!snapshot.guard_satisfied);
        assert!(!snapshot.active);
        assert_eq!(
            snapshot.missing_required_providers,
            vec![
                "uptrakit-agent-ssh".to_string(),
                "uptrakit-mqtt".to_string()
            ]
        );
    }

    #[test]
    fn surface_rollout_phase0_marks_embedded_ssh_only_after_runtime_success() {
        let rollout = super::build_surface_runtime_rollout_state_for_phase0(true);
        assert_eq!(
            rollout.snapshot().missing_required_providers,
            vec![
                "uptrakit-agent-ssh".to_string(),
                "uptrakit-mqtt".to_string()
            ]
        );

        rollout.set_local_requirement_satisfied("uptrakit-agent-ssh", true);
        let snapshot = rollout.snapshot();
        assert!(!snapshot.active);
        assert_eq!(
            snapshot.missing_required_providers,
            vec!["uptrakit-mqtt".to_string()]
        );
    }
}
