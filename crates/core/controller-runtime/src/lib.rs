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
pub(crate) mod tasks;
#[cfg(feature = "test-support")]
pub mod test_support;
#[cfg(feature = "zeroconf")]
mod zeroconf;

use clap::Parser;
use sea_orm::DatabaseConnection;
use thiserror::Error;
use uptrakit_audit_log::AuditLogDispatcher;
use uptrakit_build_info::BuildInfo;
use uptrakit_shared_macros::impl_report_conversion;

use uptrakit_config_reload::{ReexecHook, ReexecOutcome};
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

/// Status-watch channels + config path consumed by `reload_audit_bridge`.
pub(crate) struct ReloadBridgeChannels {
    pub file_state_tx: tokio::sync::watch::Sender<uptrakit_config_reload::ConfigFileState>,
    pub last_reload_tx: tokio::sync::watch::Sender<Option<uptrakit_config_reload::LastReloadInfo>>,
    pub recent_events_tx: tokio::sync::watch::Sender<Vec<serde_json::Value>>,
    pub config_path: std::path::PathBuf,
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
