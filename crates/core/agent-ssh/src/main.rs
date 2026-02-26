mod cli;
mod client;
mod commands;
pub(crate) mod db;
mod error;
mod host_info;
mod host_ops;
mod ssh_config;
mod ssh_executor;
mod ssh_key;
mod ssh_pool;
mod ssh_target;
mod ssh_transport;

use clap::Parser;
use rootcause::prelude::*;
use std::collections::BTreeSet;

use uptrakit_internal_wire::{Capability, ControllerMessage, DisconnectReason};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, Signal,
};

use cli::{Args, Commands};

/// Initialize `tracing_subscriber` with a verbosity-aware filter.
///
/// Verbosity levels expand scope progressively, keeping third-party crates
/// silent unless `RUST_LOG` explicitly enables them:
///
/// - `verbosity == 0`: `{own_module}=info`
/// - `verbosity == 1`: `{own_module}=debug`
/// - `verbosity == 2`: `uptrakit=debug`
/// - `verbosity >= 3`: `uptrakit=trace`
fn init_tracing(own_module: &str, verbosity: u8) {
    use tracing_subscriber::EnvFilter;

    if verbosity > 3 {
        eprintln!(
            "warning: -vvvv or more has no additional effect; maximum verbosity is -vvv (trace)"
        );
    }

    let directive = match verbosity {
        0 => format!("{own_module}=info"),
        1 => format!("{own_module}=debug"),
        2 => "uptrakit=debug".to_string(),
        _ => "uptrakit=trace".to_string(),
    };
    let mut filter = EnvFilter::from_default_env();
    if let Ok(d) = directive.parse() {
        filter = filter.add_directive(d);
    }
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

// ---------------------------------------------------------------------------
// Typed error for initialization helpers
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum InitError {
    #[error("{0}")]
    Directory(String),
    #[error("{0}")]
    MasterKey(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Hex(String),
}

type InitResult<T> = std::result::Result<T, rootcause::Report<InitError>>;

struct SshAgentHandler {
    state_dir: std::path::PathBuf,
    local_db: Option<sea_orm::DatabaseConnection>,
    in_flight_update: Option<client::InFlightUpdate>,
    pool: ssh_pool::SshConnectionPool,
}

#[async_trait::async_trait]
impl ServiceHandler for SshAgentHandler {
    const DIR_NAME: &'static str = "agent-ssh";
    const SERVICE_LABEL: &'static str = "uptrakit-agent-ssh service";

    type ServiceEvent = client::UpdateEvent;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        // Open (or create) the local SSH host database.
        let local_db = crate::db::init_db(&self.state_dir).await.map_err(|e| {
            report!(LoopError::Other(format!(
                "failed to initialize local database: {e}"
            )))
        })?;
        tracing::debug!("local SSH host database initialized");

        // Report enrolled hosts to controller.
        client::report_enrolled_hosts(&local_db, conn, &self.pool).await;

        self.local_db = Some(local_db);
        Ok(())
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        let db = self.local_db.as_ref().ok_or_else(|| {
            report!(LoopError::Other(
                "local_db not initialized: on_connected must be called before on_message"
                    .to_string()
            ))
        })?;
        match msg {
            ControllerMessage::CheckVersions(payload) => {
                Ok(client::handle_check_versions_ssh(payload, db, conn, &self.pool).await)
            }
            ControllerMessage::ExecuteUpdate(payload) => {
                client::handle_execute_update_ssh(
                    *payload,
                    db,
                    &mut self.in_flight_update,
                    conn,
                    &self.pool,
                )
                .await;
                Ok(None)
            }
            ControllerMessage::DiscoverSoftware(payload) => {
                Ok(client::handle_discover_software_ssh(payload, db, conn, &self.pool).await)
            }
            _ => {
                tracing::debug!("ignoring unrecognized message in authenticated loop");
                Ok(None)
            }
        }
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        if let Some(ref mut update) = self.in_flight_update {
            tokio::select! {
                biased;
                Some(output_msg) = update.output_rx.recv() => {
                    client::UpdateEvent::Output(output_msg)
                }
                result = &mut update.handle => {
                    client::UpdateEvent::Completed(result)
                }
            }
        } else {
            std::future::pending::<Self::ServiceEvent>().await
        }
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        let Some(ref update) = self.in_flight_update else {
            tracing::error!("received update event but no in-flight update exists");
            return Ok(None);
        };
        let update_history_id = update.update_history_id;

        match event {
            client::UpdateEvent::Output(output_msg) => {
                client::send_update_output(conn, update_history_id, output_msg).await;
            }
            client::UpdateEvent::Completed(result) => {
                client::send_update_result(conn, update_history_id, result).await;
                self.in_flight_update = None;
            }
        }
        Ok(None)
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        client::ssh_agent_capabilities()
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        shutdown_timeout_seconds: u32,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = resolve_shutdown(cause);
        let outcome = client::handle_graceful_shutdown(
            conn,
            self.in_flight_update.take(),
            shutdown_timeout_seconds,
            disconnect_reason,
            outcome,
        )
        .await;

        // Gracefully close all pooled SSH connections so remote hosts receive
        // a clean disconnect rather than a silent socket drop.
        self.pool.disconnect_all().await;

        outcome
    }
}

/// Map a [`ShutdownCause`] to the appropriate [`DisconnectReason`] and
/// [`LoopOutcome`] for this service.
///
/// | Cause | `DisconnectReason` | `LoopOutcome` |
/// | --- | --- | --- |
/// | `Signal(Hangup)` | `Restart` | `Restart` |
/// | `Signal(_)` | `Shutdown` | `Shutdown` |
/// | `ServerRestarting` | `Restart` | `Disconnected` |
fn resolve_shutdown(cause: ShutdownCause) -> (DisconnectReason, LoopOutcome) {
    match cause {
        ShutdownCause::Signal(Signal::Hangup) => (DisconnectReason::Restart, LoopOutcome::Restart),
        ShutdownCause::Signal(_) => (DisconnectReason::Shutdown, LoopOutcome::Shutdown),
        ShutdownCause::ServerRestarting => (DisconnectReason::Restart, LoopOutcome::Disconnected),
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            "uptrakit-agent-ssh",
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    // Host subcommands run with minimal tracing and no rustls provider.
    if let Some(Commands::Host { command }) = args.command {
        // Verbosity-aware tracing for CLI subcommands.
        init_tracing("uptrakit_agent_ssh", args.common.verbose);

        if let Err(e) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }

        let state_dir = match resolve_state_dir_from_common(&args.common).await {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        };

        if let Err(e) = commands::host::run(&state_dir, command).await {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // ── Daemon mode ─────────────────────────────────────────────────
    // Validate that --url is provided for daemon mode.
    if args.common.url.is_none() {
        eprintln!("error: --url is required for daemon mode");
        std::process::exit(1);
    }

    init_tracing("uptrakit_agent_ssh", args.common.verbose);
    uptrakit_service_sdk::init_crypto();

    // Initialize master encryption key for local SSH credential storage.
    if let Err(e) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
        tracing::error!("{e}");
        std::process::exit(1);
    }

    // Resolve state directory early so we can pass it to the handler.
    let state_dir = match resolve_state_dir_from_common(&args.common).await {
        Ok(dir) => dir,
        Err(e) => {
            tracing::error!("{e}");
            std::process::exit(1);
        }
    };

    let mut handler = SshAgentHandler {
        state_dir,
        local_db: None,
        in_flight_update: None,
        pool: ssh_pool::SshConnectionPool::new(),
    };
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-agent-ssh",
        &args.common,
        &mut handler,
    )
    .await;
}

/// Resolve the state directory for this service.
async fn resolve_state_dir_from_common(
    common: &uptrakit_service_sdk::cli::CommonServiceArgs,
) -> InitResult<std::path::PathBuf> {
    let dirs = common.resolve_dirs("agent-ssh").map_err(|e| {
        report!(InitError::Directory(format!(
            "failed to resolve directories: {e}"
        )))
    })?;
    dirs.ensure_state_dir().await.map_err(|e| {
        report!(InitError::Directory(format!(
            "failed to ensure state directory: {e}"
        )))
    })?;
    Ok(dirs.state_dir().to_path_buf())
}

/// Initialize the master encryption key from CLI args or environment.
fn init_master_key(
    master_key_file: &Option<std::path::PathBuf>,
    allow_plaintext_secrets: bool,
) -> InitResult<()> {
    let env_val = std::env::var("UPTRAKIT_MASTER_KEY").ok();
    let key_hex = read_master_key_hex(master_key_file.as_deref(), env_val.as_deref())?;

    match key_hex {
        Some(key_hex) => {
            if allow_plaintext_secrets {
                tracing::warn!(
                    "--allow-plaintext-secrets is enabled. This flag is for development only; \
                    encryption remains enabled because a master key was provided."
                );
            }
            let key_bytes = parse_master_key_hex(&key_hex)?;
            uptrakit_crypto::init_master_key(zeroize::Zeroizing::new(key_bytes)).map_err(|e| {
                report!(InitError::MasterKey(format!(
                    "failed to initialize master key: {e}"
                )))
            })?;
            tracing::info!("master encryption key initialized");
        }
        None => {
            if allow_plaintext_secrets {
                tracing::warn!(
                    "master encryption key not set; encryption at rest is disabled. \
                    This is for development only and is NOT safe for production."
                );
            } else {
                bail!(InitError::MasterKey(
                    "master encryption key is required: set UPTRAKIT_MASTER_KEY env var \
                     (64-char hex string) or pass --master-key-file <path>. \
                     For development only, pass --allow-plaintext-secrets to run without \
                     encryption at rest."
                        .into(),
                ));
            }
        }
    }
    Ok(())
}

fn read_master_key_hex(
    master_key_file: Option<&std::path::Path>,
    env_val: Option<&str>,
) -> InitResult<Option<String>> {
    if let Some(key_file) = master_key_file {
        let contents = std::fs::read_to_string(key_file).map_err(|e| report!(InitError::Io(e)))?;
        return Ok(Some(contents.trim().to_string()));
    }

    if let Some(env_val) = env_val {
        return Ok(Some(env_val.trim().to_string()));
    }

    Ok(None)
}

fn parse_master_key_hex(key_hex: &str) -> InitResult<[u8; 32]> {
    let bytes = uptrakit_shared_types::hex::decode(key_hex).map_err(|e| {
        report!(InitError::Hex(format!(
            "master key must be a 64-character hex string: {e}"
        )))
    })?;
    let key_bytes: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
        report!(InitError::Hex(format!(
            "master key must be exactly 32 bytes (64 hex chars), got {} bytes",
            v.len()
        )))
    })?;
    Ok(key_bytes)
}

#[cfg(test)]
mod tests {
    use super::{parse_master_key_hex, read_master_key_hex};
    use std::io::Write;

    #[test]
    fn missing_key_returns_none() {
        let result = read_master_key_hex(None, None);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn env_key_is_trimmed() {
        let result = read_master_key_hex(None, Some("  deadbeef  "));
        assert!(matches!(result, Ok(Some(ref value)) if value == "deadbeef"));
    }

    #[test]
    fn file_key_is_trimmed() {
        let mut file = match tempfile::NamedTempFile::new() {
            Ok(f) => f,
            Err(_) => return,
        };
        assert!(file.write_all(b"  0123  ").is_ok());
        let result = read_master_key_hex(Some(file.path()), None);
        assert!(matches!(result, Ok(Some(ref value)) if value == "0123"));
    }

    #[test]
    fn parse_master_key_rejects_invalid_hex() {
        let result = parse_master_key_hex("not-hex");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_rejects_invalid_length() {
        let result = parse_master_key_hex("aa");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_accepts_valid_length() {
        let key_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = parse_master_key_hex(key_hex);
        assert!(matches!(result, Ok(bytes) if bytes.len() == 32));
    }

    #[test]
    fn resolve_shutdown_hangup() {
        use super::{LoopOutcome, ShutdownCause, Signal, resolve_shutdown};
        use uptrakit_internal_wire::DisconnectReason;
        let (reason, outcome) = resolve_shutdown(ShutdownCause::Signal(Signal::Hangup));
        assert_eq!(reason, DisconnectReason::Restart);
        assert_eq!(outcome, LoopOutcome::Restart);
    }

    #[test]
    fn resolve_shutdown_terminate() {
        use super::{LoopOutcome, ShutdownCause, Signal, resolve_shutdown};
        use uptrakit_internal_wire::DisconnectReason;
        let (reason, outcome) = resolve_shutdown(ShutdownCause::Signal(Signal::Terminate));
        assert_eq!(reason, DisconnectReason::Shutdown);
        assert_eq!(outcome, LoopOutcome::Shutdown);
    }

    #[test]
    fn resolve_shutdown_server_restarting() {
        use super::{LoopOutcome, ShutdownCause, resolve_shutdown};
        use uptrakit_internal_wire::DisconnectReason;
        let (reason, outcome) = resolve_shutdown(ShutdownCause::ServerRestarting);
        assert_eq!(reason, DisconnectReason::Restart);
        assert_eq!(outcome, LoopOutcome::Disconnected);
    }
}
