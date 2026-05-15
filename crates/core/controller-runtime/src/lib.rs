#[cfg(feature = "embedded-agent")]
mod agent;
mod audit_enricher;
mod cert_signer;
mod cli;
mod crl_manager;
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
mod embedded;
#[cfg(feature = "embedded-frontend")]
mod embedded_frontend;
mod migration;
#[cfg(feature = "embedded-mqtt")]
mod mqtt;
mod mtls_acceptor;
mod pki;
mod reconcile;
mod reencrypt;
pub(crate) mod reexec;
mod reload;
#[cfg(feature = "embedded-scheduler")]
mod scheduler;
mod server;
mod server_cert_resolver;
mod service_host;
#[cfg(feature = "embedded-ssh-agent")]
mod ssh_agent;
mod startup;
mod tasks;
#[cfg(feature = "zeroconf")]
mod zeroconf;

use std::net::SocketAddr;
use std::os::unix::io::AsRawFd as _;
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
use uptrakit_plugin_infrastructure_registry::{PluginHttpClientConfig, build_plugin_http_client};
use uptrakit_shared_macros::impl_report_conversion;

use uptrakit_config_reload::{ReexecHook, ReexecOutcome};
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

/// Reexec hook implementation for the controller process.
///
/// Captures listener FDs and exec arguments at startup so that
/// [`reexec::perform_reexec`] can reconstruct the command line without
/// re-reading the process environment after a signal.
struct ControllerReexecHook {
    /// Resolved from `std::env::current_exe()` at startup.
    current_exe: std::path::PathBuf,
    config_path: std::path::PathBuf,
    master_key_file: Option<String>,
    generation: u64,
    /// Raw listener FDs cleared of `FD_CLOEXEC` before `exec()`.
    /// Empty when PKI HTTP is disabled; the child re-binds in that case.
    listener_fds: Vec<std::os::unix::io::RawFd>,
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
            master_key_file: self.master_key_file.clone(),
            listener_count: self.listener_fds.len(),
            generation: self.generation,
        };

        match reexec::perform_reexec(&plan, &self.listener_fds) {
            Ok(infallible) => match infallible {},
            Err(e) => ReexecOutcome::ExecFailed(e),
        }
    }
}

async fn async_main() -> std::process::ExitCode {
    let args = cli::Args::parse();

    if args.version {
        print_build_info();
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

    if let Err(report) = Box::pin(run_server(args)).await {
        eprintln!("Error:\n{report}");
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

async fn run_server(args: cli::Args) -> Result<()> {
    // Phase 0: Load TOML config — must happen before all other phases so that
    // all configuration comes from the file rather than CLI flags.
    let config_path = args.find_config_path().map_err(|e| {
        report!(AppError::Config(format!(
            "failed to resolve config path: {e}"
        )))
    })?;
    tracing::info!("toml config path: {}", config_path.display());
    let config_path_for_coord = config_path.clone();
    let booted = startup::boot_config(config_path)
        .await
        .map_err(|e| report!(AppError::Config(format!("failed to load TOML config: {e}"))))?;
    let runtime = &booted.runtime;

    // Parse bootstrap args from environment variables (no CLI flags; env only).
    let oidc_bootstrap = cli::OidcBootstrapArgs::try_parse_from(["uptrakit-controller"])
        .unwrap_or_else(|_| {
            // Fallback: construct with all None/default values.
            // env vars are picked up by clap's env attribute when try_parse_from
            // is called with a minimal argv — the env attributes on each field
            // still apply, so env vars take effect here.
            cli::OidcBootstrapArgs {
                oidc_issuer_url: std::env::var("UPTRAKIT_OIDC_ISSUER_URL").ok(),
                oidc_client_id: std::env::var("UPTRAKIT_OIDC_CLIENT_ID").ok(),
                oidc_client_secret: std::env::var("UPTRAKIT_OIDC_CLIENT_SECRET").ok(),
                oidc_provider_name: std::env::var("UPTRAKIT_OIDC_PROVIDER_NAME")
                    .ok()
                    .or_else(|| Some("SSO".to_string())),
                oidc_provider_slug: std::env::var("UPTRAKIT_OIDC_PROVIDER_SLUG")
                    .ok()
                    .or_else(|| Some("sso".to_string())),
                oidc_scopes: std::env::var("UPTRAKIT_OIDC_SCOPES")
                    .ok()
                    .or_else(|| Some("openid email profile groups".to_string())),
                oidc_allow_private_network_issuers: std::env::var(
                    "UPTRAKIT_OIDC_ALLOW_PRIVATE_NETWORK_ISSUERS",
                )
                .ok()
                .and_then(|v| v.parse().ok()),
            }
        });

    let enrollment_bootstrap =
        cli::EnrollmentBootstrapArgs::try_parse_from(["uptrakit-controller"]).unwrap_or_else(
            |_| cli::EnrollmentBootstrapArgs {
                bootstrap_enrollment_token: std::env::var("UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN")
                    .ok(),
                bootstrap_enrollment_token_max_uses: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN_MAX_USES",
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
                bootstrap_enrollment_token_ttl: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN_TTL",
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
                bootstrap_system_enrollment_token: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN",
                )
                .ok(),
                bootstrap_system_enrollment_token_max_uses: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN_MAX_USES",
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
                bootstrap_system_enrollment_token_ttl: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN_TTL",
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            },
        );

    // Initialise tracing. Log level from runtime.log in TOML; -v/-vv/-vvv on CLI overrides.
    #[expect(
        clippy::allow_attributes,
        clippy::allow_attributes_without_reason,
        reason = "feature-conditional: unused_mut fires only when the journald feature is disabled"
    )]
    #[allow(unused_mut)]
    let mut builder = uptrakit_tracing_init::TracingBuilder::new()
        .verbosity(args.verbose)
        .max_verbosity(3)
        .directives_for_verbosity(
            0,
            &[
                ("uptrakit_controller_runtime", "info"),
                ("uptrakit_web_api", "info"),
            ],
        )
        .directives_for_verbosity(
            1,
            &[
                ("uptrakit_controller_runtime", "debug"),
                ("uptrakit_web_api", "debug"),
            ],
        )
        .directives_for_verbosity(2, &[("uptrakit", "debug")])
        .directives_for_verbosity(3, &[("uptrakit", "trace")]);

    // When the journald audit backend is selected, add a dedicated journald
    // tracing layer filtered to the `uptrakit_audit` target so that structured
    // audit events reach the system journal alongside normal stdout logging.
    #[cfg(feature = "journald")]
    {
        #[expect(
            clippy::expect_used,
            reason = "infallible at startup: journald layer construction failures are unrecoverable for the requested audit backend and must abort initialization"
        )]
        let journald = tracing_journald::layer()
            .expect("failed to connect to journald")
            .with_filter(tracing_subscriber::EnvFilter::new("uptrakit_audit=info"));
        builder = builder.extra_layer(Box::new(journald));
    }

    builder.init();

    // Phase 1: Master key initialization — reads from --master-key-from or TOML
    // master_key.path as a fallback.
    let master_key_source = args.master_key_from.as_deref().or_else(|| {
        let p = runtime.master_key.path.as_str();
        if p.is_empty() { None } else { Some(p) }
    });
    // Build a `file:` prefixed source if we got a bare path from TOML.
    let master_key_from_toml_buf;
    let master_key_source = if let Some(src) = master_key_source {
        if !src.starts_with("file:")
            && !src.starts_with("env:")
            && !runtime.master_key.path.is_empty()
            && src == runtime.master_key.path.as_str()
        {
            master_key_from_toml_buf = format!("file:{src}");
            Some(master_key_from_toml_buf.as_str())
        } else {
            Some(src)
        }
    } else {
        None
    };
    let master_key_hex = startup::init_master_key(master_key_source)?;

    // Phase 2: Application directories — use platform defaults (no CLI overrides).
    let app_dirs =
        uptrakit_directories::AppDirs::resolve("controller", None, None).map_err(|e| {
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
    let controller_installation_id = startup::init_installation_id(app_dirs.state_dir()).await?;

    // Phase 3: Database — URL and pool size from TOML [db].
    let db_init =
        startup::init_database(&runtime.db.url, runtime.db.pool_size, app_dirs.state_dir()).await?;
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

    // Phase 6: Reconcile settings — use TOML values as seeds
    let reconciled =
        startup::reconcile_all_settings(&db_conn, runtime, &settings, &global_raw).await?;

    // Phase 7: OIDC bootstrap
    startup::bootstrap_oidc(&db_conn, default_tenant_id, &oidc_bootstrap).await?;

    // Phase 7b: Enrollment token bootstrap
    startup::bootstrap_enrollment_tokens(&db_conn, default_tenant_id, &enrollment_bootstrap)
        .await?;

    // Phase 7c: OAuth settings defaults
    startup::seed_oauth_defaults(&db_conn).await?;

    // Phase 8: Validate configuration
    let validated = startup::validate_configuration(runtime, &reconciled)?;

    // Phase 8b: Claim inherited TCP sockets and pre-bind listeners.
    //
    // This must happen before the coordinator block so that `listener_fds` and
    // `https_std`/`pki_std_for_spawn` are in scope when the reexec hook is
    // constructed and when the server task is spawned.
    let inherited = reexec::listenfd::take_inherited_listeners().unwrap_or_else(|e| {
        tracing::warn!("LISTEN_FDS claim failed: {e}; falling back to fresh bind");
        None
    });
    let (inherited_https, inherited_pki) = match inherited {
        Some(s) => {
            let https_std = s.https.into_std().map_err(|e| {
                rootcause::report!(AppError::Config(format!(
                    "into_std failed for inherited HTTPS socket: {e}"
                )))
            })?;
            let pki_std = s.pki.into_std().map_err(|e| {
                rootcause::report!(AppError::Config(format!(
                    "into_std failed for inherited PKI socket: {e}"
                )))
            })?;
            (Some(https_std), Some(pki_std))
        }
        None => (None, None),
    };

    // Pre-bind HTTPS socket so we have the raw FD for reexec listener inheritance.
    let https_std = match inherited_https {
        Some(l) => l,
        None => {
            let l = std::net::TcpListener::bind(reconciled.https_addr).map_err(|e| {
                report!(AppError::Config(format!(
                    "bind HTTPS {}: {e}",
                    reconciled.https_addr
                )))
            })?;
            l.set_nonblocking(true)
                .map_err(|e| report!(AppError::Config(format!("set_nonblocking HTTPS: {e}"))))?;
            l
        }
    };
    let https_raw_fd = https_std.as_raw_fd();

    // Pre-bind PKI socket and collect listener FDs for reexec inheritance.
    let (listener_fds, pki_std_for_spawn): (
        Vec<std::os::unix::io::RawFd>,
        Option<std::net::TcpListener>,
    ) = if let Some(pki_port) = validated.pki_http_port {
        let pki_std = match inherited_pki {
            Some(l) => l,
            None => {
                let addr = std::net::SocketAddr::from(([0, 0, 0, 0], pki_port));
                let l = std::net::TcpListener::bind(addr)
                    .map_err(|e| report!(AppError::Config(format!("bind PKI HTTP {addr}: {e}"))))?;
                l.set_nonblocking(true)
                    .map_err(|e| report!(AppError::Config(format!("set_nonblocking PKI: {e}"))))?;
                l
            }
        };
        let pki_fd = pki_std.as_raw_fd();
        (vec![https_raw_fd, pki_fd], Some(pki_std))
    } else {
        (vec![], None)
    };

    // Phase 9: PKI + TLS — cert/key paths from TOML [tls]
    let pki =
        startup::init_pki_runtime(runtime, &db_conn, app_dirs.config_dir(), &reconciled).await?;

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
        server_cert_resolver,
        revocation_notify,
        ca_rotation_trigger,
        crl_pem_cache,
        crl_manager,
        initial_ca_version,
        has_external_tls_cert,
    } = pki;

    // Build shared application state
    // Two-step: clone as concrete type then coerce to Arc<dyn IssuerSource>.
    // Arc::clone resolves its argument type from the return annotation, so
    // we cannot pass &Arc<CrlManager> when the binding expects Arc<dyn Trait>.
    let issuer_source: Arc<dyn cert_signer::IssuerSource> = {
        let concrete: Arc<crl_manager::CrlManager> = Arc::clone(&crl_manager);
        concrete
    };
    // Resolve the effective trust domain: explicit tls.trust_domain wins;
    // falls back to tls.sans[0] (legacy derivation); empty = SPIFFE disabled.
    let effective_trust_domain = runtime
        .tls
        .effective_trust_domain(&runtime.tls.sans)
        .to_owned();
    let cert_signer = {
        let signer = cert_signer::RcgenAgentCertSigner::new(ca_rx.clone(), issuer_source);
        if effective_trust_domain.is_empty() {
            Arc::new(signer) as Arc<dyn uptrakit_web_api::cert_signer::AgentCertSigner>
        } else {
            Arc::new(signer.with_trust_domain(effective_trust_domain))
                as Arc<dyn uptrakit_web_api::cert_signer::AgentCertSigner>
        }
    };

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
    #[cfg_attr(
        not(feature = "nats"),
        expect(
            unused_mut,
            reason = "only mutated inside the #[cfg(feature = \"nats\")] block below"
        )
    )]
    let mut notification_service =
        uptrakit_web_api::notification_service::NotificationService::new(
            service_connections.clone(),
            controller_id,
        )
        .with_claim_registry(Arc::clone(&workload_claim_registry));

    // NATS transport (optional, feature-gated)
    // Uses the reconciled NATS URL (DB value wins over TOML; TOML seeds DB on first run).
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
        notification_service = notification_service.with_nats(Arc::new(nats.clone()));
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
    #[cfg_attr(
        not(feature = "nats"),
        expect(
            unused_mut,
            reason = "only mutated inside the #[cfg(feature = \"nats\")] block below"
        )
    )]
    let mut event_broadcaster = uptrakit_web_api::event_broadcaster::EventBroadcaster::new();
    #[cfg(feature = "nats")]
    if let Some(ref nats) = nats_transport {
        event_broadcaster = event_broadcaster.with_nats(Arc::new(nats.clone()), controller_id);
    }

    let token_denylist = Arc::new(
        uptrakit_web_api::auth::token_denylist::TokenDenylist::new_with_db(db_conn.clone()),
    );
    let global_providers = Arc::new(uptrakit_web_api::global_providers::GlobalProviders::new(
        db_conn.clone(),
    ));

    // Shared cancellation token: cancelled by BackgroundTasks::shutdown(), which
    // also signals open SSE streams in the web API to terminate cleanly.
    let shutdown_token = CancellationToken::new();

    // Load instance-scoped plugin state from DB before catalog construction so
    // that instance-gated plugins reflect their persisted enabled/disabled state
    // from first request rather than requiring a restart after toggling.
    let instance_plugin_snapshot =
        uptrakit_web_api_queries::instance_plugin_settings::load_at_boot(&db_conn)
            .await
            .map_err(|e| {
                report!(AppError::Config(format!(
                    "failed to load instance plugin snapshot: {e}"
                )))
            })?;
    tracing::info!(
        plugin_count = instance_plugin_snapshot.iter().count(),
        "instance plugin snapshot loaded"
    );

    // Build InstancePluginStates by intersecting the snapshot with all
    // compiled-in instance-scoped descriptors.
    let all_descriptors = uptrakit_plugin_infrastructure_registry::all_descriptors();
    let instance_states = uptrakit_plugin_infrastructure_registry::InstancePluginStates::from_pairs(
        all_descriptors
            .iter()
            .filter(|d| d.scope == uptrakit_plugin_infrastructure_registry::PluginScope::Instance)
            .map(|d| (d.type_id, instance_plugin_snapshot.enabled(d.type_id))),
    );

    // Wrap the snapshot in Arc<ArcSwap<>> so AppState can serve lock-free reads
    // on the hot path and routes can atomically publish upserts.
    let instance_plugin_snapshot_handle =
        std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(instance_plugin_snapshot));

    // Build the plugin catalog from all compiled-in descriptors.
    // The catalog replaces the old PluginRegistry and provides PluginOps.
    // allow_private_urls defaults to false (SSRF-safe by default).
    let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig {
        allow_private_urls: false,
        http_client: Some(
            build_plugin_http_client(PluginHttpClientConfig {
                user_agent: "uptrakit-controller",
                redirect_policy: reqwest::redirect::Policy::limited(5),
                ..Default::default()
            })
            .map_err(|e| report!(AppError::Config(format!("plugin catalog HTTP client: {e}"))))?,
        ),
        cancellation_token: Some(shutdown_token.clone()),
        global_provider_lookup: Some(global_providers.clone()),
    };
    let catalog =
        uptrakit_plugin_infrastructure_registry::build_catalog(&catalog_config, instance_states)
            .context_transform(|_| {
                AppError::Config("failed to build plugin catalog".to_string())
            })?;

    let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(catalog);

    tracing::info!(
        update_protection = plugin_ops.controller_update_protection().is_some(),
        "plugin catalog ready"
    );

    let callback_base_url = format!("https://{}", reconciled.https_addr);
    let notification_dispatcher =
        uptrakit_web_api::notifications::dispatcher::NotificationDispatcher::new(
            db_conn.clone(),
            Arc::clone(&plugin_ops),
            callback_base_url,
        );

    // Build credential sources for external services that need direct infrastructure access.
    let credential_sources = {
        #[cfg_attr(
            not(feature = "nats"),
            expect(
                unused_mut,
                reason = "only mutated inside the #[cfg(feature = \"nats\")] block below"
            )
        )]
        let mut sources = uptrakit_web_api::ServiceCredentialSources::new(
            Some(db_url.clone()),
            None,
            master_key_hex,
        );
        #[cfg(feature = "nats")]
        if let Some(ref url) = reconciled.nats_url {
            sources.nats_url = Some(url.clone());
        }
        sources
    };

    // Audit log backend and filter wiring.
    let (_audit_filter, audit_dispatcher) = build_audit_logger(runtime, &db_conn).await?;

    let surface_registry = Arc::new(uptrakit_web_api::surface_registry::SurfaceRegistry::new(
        uptrakit_web_api::surface_registry::SurfaceRegistryConfig::default(),
    ));
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
    let audit_emitter = uptrakit_audit_log::AuditEmitter::new(audit_dispatcher.clone());
    let surface_proxy = Arc::new(
        uptrakit_web_api::surface_proxy::SurfaceProxy::new().with_local_executor(Arc::new(
            uptrakit_web_api::surface_proxy::PluginSurfaceLocalExecutor::new(
                Arc::new(db_conn.clone()),
                Arc::clone(&plugin_ops),
            )
            .with_audit_emitter(audit_emitter.clone()),
        )),
    );

    // Create the embedded service host before AppState so it can be stored
    // in the state. The host's `add()` is called later in spawn_background_tasks.
    let embedded_host = Arc::new(embedded::EmbeddedServiceHost::new());
    let builtin_host = service_host::BuiltinServiceHost::new(Arc::clone(&embedded_host));

    let builder = AppState::builder()
        .ca_snapshot(ca_rx)
        .ca_key_store(ca_key_store)
        .db(db_conn.clone())
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
        .server_cert_resolver(std::sync::Arc::clone(&server_cert_resolver)
            as std::sync::Arc<dyn uptrakit_web_api::server_cert_swap::ServerCertSwap>)
        .crl_pem_cache(crl_pem_cache)
        .ca_rotation_trigger(ca_rotation_trigger)
        .default_tenant_id(default_tenant_id)
        .controller_id(controller_id)
        .notification_service(notification_service)
        .notification_dispatcher(notification_dispatcher)
        .token_denylist(token_denylist)
        .credential_sources(credential_sources)
        .global_providers(global_providers)
        .event_broadcaster(event_broadcaster.clone())
        .batch_progress_broadcaster(batch_progress_broadcaster)
        .shutdown_token(shutdown_token.clone())
        .audit_log_dispatcher(audit_dispatcher.clone())
        .audit_emitter(audit_emitter.clone())
        .plugin_ops(plugin_ops)
        .surface_registry(surface_registry)
        .surface_proxy(surface_proxy)
        .workload_claim_registry(workload_claim_registry)
        .instance_plugin_snapshot(std::sync::Arc::clone(&instance_plugin_snapshot_handle))
        .reject_dangerous_commands(true);

    // Wire config-reload coordinator and receivers.
    //
    // Build each Reloadable from the loaded RuntimeConfig + available subsystem
    // handles, extend the coordinator (which was not yet spawned), extract a
    // handle, then spawn coordinator + reconciler.
    let (
        coordinator_handle_opt,
        settings_version_cache_opt,
        receivers_opt,
        reload_file_state_rx_opt,
        reload_last_reload_rx_opt,
        reload_recent_events_rx_opt,
        audit_log_filter_rx_opt,
    ) = {
        let mut b = booted;
        // DB → TLS → Listeners → NATS → Audit → Zeroconf → Plugins → Embedded
        let db_reloadable = reload::db_pool::DbPoolReloadable::new(db_conn.clone(), db_url.clone());
        let (tls_reloadable, _tls_rx) =
            reload::tls_snapshot::TlsSnapshotReloadable::new(b.runtime.tls.clone());
        let (https_reloadable, _https_rx) =
            reload::https_listener::HttpsListenerReloadable::new(b.runtime.network.https.clone());
        let (pki_reloadable, _pki_rx) =
            reload::pki_listener::PkiListenerReloadable::new(b.runtime.network.pki.clone());
        let (audit_reloadable, audit_log_filter_rx) = reload::audit::AuditDispatcherReloadable::new(
            audit_dispatcher.clone(),
            b.runtime.audit.clone(),
        );
        let (zeroconf_reloadable, _zeroconf_rx) =
            reload::zeroconf::ZeroconfReloadable::new(b.runtime.zeroconf.clone());
        let (plugin_reloadable, _plugin_rx) =
            uptrakit_web_api_queries::reload::plugin_registry::PluginCatalogReloadable::new(
                uptrakit_config_reload::config::PluginsConfig::default(),
            );
        let (embedded_reloadable, _embedded_rx) =
            reload::embedded::EmbeddedServicesReloadable::new(b.runtime.embedded_services.clone());

        #[cfg_attr(
            not(feature = "nats"),
            expect(
                unused_mut,
                reason = "only pushed inside the #[cfg(feature = \"nats\")] block below"
            )
        )]
        let mut reloadables: Vec<
            std::sync::Arc<dyn uptrakit_config_reload::ReloadableErased>,
        > = vec![
            Arc::new(db_reloadable),
            Arc::new(tls_reloadable),
            Arc::new(https_reloadable),
            Arc::new(pki_reloadable),
            Arc::new(audit_reloadable),
            Arc::new(zeroconf_reloadable),
            Arc::new(plugin_reloadable),
            Arc::new(embedded_reloadable),
        ];

        #[cfg(feature = "nats")]
        if let (Some(nats), Some(url)) = (&nats_transport, &reconciled.nats_url) {
            reloadables.push(Arc::new(reload::nats::NatsReloadable::new(
                nats.nats_client(),
                url.clone(),
            )));
        }

        b.coordinator.extend_reloadables(reloadables);
        b.coordinator
            .set_alert_writer(std::sync::Arc::new(reload::audit::AuditAlertWriter::new(
                audit_emitter.clone(),
            )));

        let current_exe = std::env::current_exe()
            .map_err(|e| report!(AppError::Config(format!("resolve current_exe: {e}"))))?;
        b.coordinator.set_config_path(config_path_for_coord.clone());
        b.coordinator
            .set_current_config(Arc::new(b.runtime.clone()));
        b.coordinator
            .set_reexec_hook(Box::new(ControllerReexecHook {
                current_exe,
                config_path: config_path_for_coord.clone(),
                master_key_file: args.master_key_from.clone(),
                generation: reexec::listenfd::current_generation(),
                listener_fds: listener_fds.clone(),
            }));

        let coordinator_handle = b.coordinator.handle();

        let _reconciler = reload::reconciler::spawn_config_reconciler(
            db_conn.clone(),
            coordinator_handle.sender(),
            b.settings_version_cache.clone(),
            shutdown_token.clone(),
        );

        tokio::spawn(b.coordinator.run());

        let audit_rx = b.audit_rx;
        let reload_file_state_tx = b.reload_file_state_tx;
        let reload_file_state_rx = b.reload_file_state_rx;
        let reload_last_reload_tx = b.reload_last_reload_tx;
        let reload_last_reload_rx = b.reload_last_reload_rx;
        let reload_recent_events_tx = b.reload_recent_events_tx;
        let reload_recent_events_rx = b.reload_recent_events_rx;
        tokio::spawn(reload_audit_bridge(
            audit_rx,
            audit_emitter,
            reload_file_state_tx,
            reload_last_reload_tx,
            reload_recent_events_tx,
            config_path_for_coord.clone(),
        ));

        (
            Some(coordinator_handle),
            Some(b.settings_version_cache),
            Some(b.receivers),
            Some(reload_file_state_rx),
            Some(reload_last_reload_rx),
            Some(reload_recent_events_rx),
            Some(audit_log_filter_rx),
        )
    };

    let builder = match (
        coordinator_handle_opt,
        settings_version_cache_opt,
        receivers_opt,
        reload_file_state_rx_opt,
        reload_last_reload_rx_opt,
        reload_recent_events_rx_opt,
        audit_log_filter_rx_opt,
    ) {
        (
            Some(handle),
            Some(cache),
            Some(receivers),
            Some(fs_rx),
            Some(lr_rx),
            Some(re_rx),
            Some(audit_filter_rx),
        ) => builder
            .coordinator_handle(handle)
            .settings_version_cache(cache)
            .config_receivers(receivers)
            .config_reload_status_receivers(fs_rx, lr_rx, re_rx)
            .audit_log_filter_rx(audit_filter_rx),
        _ => builder,
    };

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

    uptrakit_web_api::global_providers::github::emit_global_github_provider_diagnostic_if_needed(
        app_state.db(),
        &app_state.notification.event_broadcaster,
    )
    .await;

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
            #[cfg(feature = "plugin-ops")]
            if let Err(error) =
                uptrakit_web_api::queries::update_dispatch::finalize_post_update_hook(
                    app_state.db(),
                    app_state.controller_update_hook(),
                    app_state.plugin.plugin_ops.as_ref(),
                    record,
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    update_id = %record.id,
                    "post-update hook (resource restore) failed during startup cleanup"
                );
            }

            if let Err(error) =
                uptrakit_web_api::queries::update_dispatch::finalize_post_update_with_timeout(
                    app_state.db(),
                    app_state.controller_update_protection(),
                    record,
                    Duration::from_secs(2),
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    update_id = %record.id,
                    "post-update finalization failed during startup cleanup"
                );
            }

            if let Some(batch_id) = record.batch_id {
                match uptrakit_web_api::queries::update_batches::dispatch_next_in_batch(
                    app_state.db(),
                    uptrakit_web_api::queries::update_dispatch::DispatchContext {
                        notifier: &app_state.notification.notification_service,
                        protection: app_state.controller_update_protection(),
                        #[cfg(feature = "plugin-ops")]
                        hook: app_state.controller_update_hook(),
                        #[cfg(feature = "plugin-ops")]
                        notification_ops: Some(app_state.plugin.plugin_ops.as_ref()),
                    },
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
                    uptrakit_web_api::queries::update_dispatch::DispatchContext {
                        notifier: &app_state.notification.notification_service,
                        protection: app_state.controller_update_protection(),
                        #[cfg(feature = "plugin-ops")]
                        hook: app_state.controller_update_hook(),
                        #[cfg(feature = "plugin-ops")]
                        notification_ops: Some(app_state.plugin.plugin_ops.as_ref()),
                    },
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
        has_external_tls_cert,
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

    // Spawn HTTPS server
    let server_handle = axum_server::Handle::new();
    let server_options = server::ServerOptions {
        https_addr: reconciled.https_addr,
        rustls_config,
        app_state: Arc::clone(&app_state),
        static_dir: validated.static_dir,
        handle: server_handle.clone(),
        inherited_listener: Some(https_std),
    };
    let server_task = tokio::spawn(server::run(server_options));

    // Spawn zeroconf mDNS advertiser if enabled
    #[cfg(feature = "zeroconf")]
    spawn_zeroconf(&mut bg, &app_state, reconciled.https_addr);

    // Spawn PKI HTTP server if needed
    spawn_pki_http(
        &mut bg,
        &app_state,
        validated.pki_http_port,
        pki_std_for_spawn,
    );

    // Notify the service manager (and stdout-based supervisors) that all
    // servers are bound and the controller is ready to accept connections.
    reexec::sd_notify::signal_ready();

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
    bg.shutdown(server_handle, service_connections, shutdown_timeout)
        .await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Extracted helper functions
// ---------------------------------------------------------------------------

/// Build the audit log filter and dispatcher from TOML configuration.
///
/// Configures the audit backend (database or noop) and the event filter
/// (all, mutations-only, or none). Backend defaults to database; filter
/// comes from `runtime.audit.filter`.
async fn build_audit_logger(
    runtime: &uptrakit_config_reload::RuntimeConfig,
    db_conn: &DatabaseConnection,
) -> Result<(AuditFilter, AuditLogDispatcher)> {
    use uptrakit_audit_log::{FilterMode, NoopBackend};

    let filter_mode = match runtime.audit.filter.as_str() {
        "mutations" => FilterMode::Mutations,
        "none" => FilterMode::None,
        _ => FilterMode::All,
    };

    if filter_mode == FilterMode::None {
        let backend: std::sync::Arc<dyn uptrakit_audit_log::AuditLogBackend> =
            std::sync::Arc::new(NoopBackend);
        return Ok((
            AuditFilter::new(FilterMode::None),
            AuditLogDispatcher::new(backend),
        ));
    }

    // Default: database backend using the main DB connection.
    let backend: std::sync::Arc<dyn uptrakit_audit_log::AuditLogBackend> =
        std::sync::Arc::new(uptrakit_audit_log::DatabaseBackend::new(db_conn.clone()));

    tracing::info!(
        filter = %filter_mode,
        "audit logging configured"
    );

    let enricher = std::sync::Arc::new(audit_enricher::DbActorEnricher::new(db_conn.clone()));

    Ok((
        AuditFilter::new(filter_mode),
        AuditLogDispatcher::with_enricher(backend, enricher),
    ))
}

/// Spawn all background tasks: CRL manager, denylist cleanup, settings reload,
/// CA reload/rotation, scheduler, server cert renewal, and NATS consumer.
#[expect(
    clippy::too_many_arguments,
    reason = "spawns all background service tasks; each parameter drives a distinct lifecycle phase"
)]
// `controller_installation_id` is only used behind `embedded-scheduler`
// and `embedded-agent` feature flags; `has_external_tls_cert` is only used
// behind the `nats` feature flag.
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: some parameters only used inside embedded-scheduler/embedded-agent feature blocks"
)]
#[allow(unused_variables)]
async fn spawn_background_tasks(
    bg: &mut tasks::BackgroundTasks,
    app_state: &Arc<AppState>,
    crl_manager: &Arc<crl_manager::CrlManager>,
    ca_managed: bool,
    ca_tx: tokio::sync::watch::Sender<pki::CaSnapshot>,
    initial_ca_version: i64,
    controller_id: uuid::Uuid,
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
            None, // pid_file removed from CLI
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

    // Suppress unused-variable warnings in feature combinations where the
    // embedded service blocks above do not consume these values.
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

/// Bridge task: receive [`ReloadAuditEvent`]s from the coordinator and emit them as
/// system-scoped [`AuditEntry`] rows via [`AuditEmitter::emit_event`].
///
/// Also maintains the three status watch channels consumed by the
/// `GET /api/v1/instance/config-state` endpoint.
async fn reload_audit_bridge(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<uptrakit_config_reload::ReloadAuditEvent>,
    emitter: uptrakit_audit_log::AuditEmitter,
    file_state_tx: tokio::sync::watch::Sender<uptrakit_config_reload::ConfigFileState>,
    last_reload_tx: tokio::sync::watch::Sender<Option<uptrakit_config_reload::LastReloadInfo>>,
    recent_events_tx: tokio::sync::watch::Sender<Vec<serde_json::Value>>,
    config_path: std::path::PathBuf,
) {
    use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome, Event};
    use uptrakit_config_reload::ReloadAuditEvent;

    while let Some(event) = rx.recv().await {
        // Update status watch channels.
        match &event {
            ReloadAuditEvent::FileChanged { path } => {
                let pending_digest = file_digest(path);
                file_state_tx.send_modify(|s| {
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
                drop(last_reload_tx.send(Some(info)));

                match source {
                    uptrakit_config_reload::ReloadSource::Sighup
                    | uptrakit_config_reload::ReloadSource::FileWatch { .. } => {
                        let new_digest = file_digest(&config_path);
                        file_state_tx.send_modify(|s| {
                            s.digest = new_digest;
                            s.loaded_at = time::OffsetDateTime::now_utc();
                            s.pending_digest = None;
                            s.pending_detected_at = None;
                        });
                    }
                    _ => {}
                }

                let event_json = serde_json::json!({
                    "type": "applied",
                    "at": time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| String::new()),
                    "sections": sections,
                });
                recent_events_tx.send_modify(|v| {
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
                file_state_tx.send_modify(|s| {
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
                recent_events_tx.send_modify(|v| {
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
                recent_events_tx.send_modify(|v| {
                    v.push(event_json);
                    if v.len() > 20 {
                        v.remove(0);
                    }
                });
            }
            _ => {}
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
fn spawn_pki_http(
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

#[doc(hidden)]
fn print_build_info() {
    let build_info = BuildInfo::current(
        env!("UPTRAKIT_RELEASE_NAME"),
        env!("CARGO_PKG_VERSION"),
        option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
    );
    print!("{}", build_info.render_human());
}

#[expect(
    clippy::expect_used,
    reason = "infallible at startup: tokio runtime construction failures are unrecoverable and must abort process initialization"
)]
pub fn run() -> std::process::ExitCode {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime")
        .block_on(async_main())
}
