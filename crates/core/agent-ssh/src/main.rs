mod cli;
mod client;
mod commands;
pub(crate) mod db;
mod error;
mod host_info;
mod host_ops;
mod ssh_config;
mod ssh_key;
mod ssh_target;
mod ssh_transport;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_internal_wire::{
    ControllerMessage, DisconnectReason, DisconnectingPayload, ServiceMessage, ServiceType,
};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    Signal,
};

use cli::{Args, Commands};

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
}

#[async_trait::async_trait]
impl ServiceHandler for SshAgentHandler {
    const DIR_NAME: &'static str = "agent-ssh";
    const SERVICE_LABEL: &'static str = "uptrakit-agent-ssh service";
    const SERVICE_TYPE: ServiceType = ServiceType::SshAgent;

    type ServiceEvent = std::convert::Infallible;

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
        client::report_enrolled_hosts(&local_db, conn).await;

        self.local_db = Some(local_db);
        Ok(())
    }

    async fn on_message(
        &mut self,
        _msg: ControllerMessage,
        _conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        tracing::debug!("ignoring unrecognized message in authenticated loop");
        Ok(None)
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        std::future::pending().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        _conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match event {}
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        _signal: Signal,
        _shutdown_timeout_seconds: u32,
    ) -> LoopOutcome {
        let disconnecting_msg =
            ServiceMessage::Disconnecting(DisconnectingPayload::new(DisconnectReason::Shutdown));
        if let Err(e) = conn.send(disconnecting_msg).await {
            tracing::debug!(error = %e, "failed to send Disconnecting message");
        } else {
            tracing::debug!("sent Disconnecting message to controller");
        }
        LoopOutcome::Shutdown
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
        // Minimal tracing for CLI subcommands.
        let filter = EnvFilter::from_default_env();
        tracing_subscriber::fmt().with_env_filter(filter).init();

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

    uptrakit_service_sdk::init_tracing("uptrakit_agent_ssh=info");
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
}
