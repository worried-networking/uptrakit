mod cli;
mod client;
mod commands;
pub(crate) mod db;
mod error;
mod host_ops;
mod ssh_config;
mod ssh_executor;
mod ssh_key;
mod ssh_target;
mod ssh_transport;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_build_info::BuildInfo;
use uptrakit_service_sdk::{
    AuthenticatedContext, LoopOutcome, ServiceConfig, ServiceEnrollmentInfo, ServiceHandler,
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
}

impl ServiceHandler for SshAgentHandler {
    fn config(&self) -> ServiceConfig {
        ServiceConfig {
            dir_name: "agent-ssh",
            service_label: "uptrakit-agent-ssh service",
        }
    }

    fn enrollment_info(&self) -> ServiceEnrollmentInfo {
        ServiceEnrollmentInfo {
            service_type: uptrakit_internal_wire::ServiceType::SshAgent,
        }
    }

    fn run_authenticated_loop<'a>(
        &'a mut self,
        ctx: AuthenticatedContext<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = uptrakit_service_sdk::Result<LoopOutcome>> + Send + 'a,
        >,
    > {
        Box::pin(async move {
            let cert_not_after_ts = ctx.identity.cert_not_after_ms();

            match client::run_authenticated_loop(client::AuthenticatedLoopParams {
                host: ctx.host,
                port: ctx.port,
                base_url: ctx.base_url,
                pki_addr: ctx.pki_addr,
                ca_pem: ctx.ca_pem,
                tls_connector: ctx.tls_connector,
                cert_not_after_ts,
                identity: ctx.identity,
                state_dir: &self.state_dir,
            })
            .await
            {
                Ok(outcome) => Ok(outcome),
                Err(e) => {
                    let ctx = e.current_context();
                    if ctx.is_cert_expired() {
                        Err(report!(uptrakit_service_sdk::EnrollmentError::Tls(
                            uptrakit_service_sdk::TlsError::Rustls(rustls::Error::AlertReceived(
                                rustls::AlertDescription::CertificateExpired,
                            ))
                        )))
                    } else if ctx.is_receive_closed() {
                        Err(report!(uptrakit_service_sdk::EnrollmentError::Protocol(
                            uptrakit_service_sdk::ProtocolError::ReceiveClosed
                        )))
                    } else {
                        Err(report!(uptrakit_service_sdk::EnrollmentError::Protocol(
                            uptrakit_service_sdk::ProtocolError::Enrollment(e.to_string())
                        )))
                    }
                }
            }
        })
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.common.version {
        print_build_info();
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

        let state_dir = match resolve_state_dir_from_common(&args.common) {
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

    let filter = match "uptrakit_agent_ssh=info".parse() {
        Ok(directive) => EnvFilter::from_default_env().add_directive(directive),
        Err(_) => EnvFilter::from_default_env(),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Install the default crypto provider for rustls.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Initialize master encryption key for local SSH credential storage.
    if let Err(e) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
        tracing::error!("{e}");
        std::process::exit(1);
    }

    // Resolve state directory early so we can pass it to the handler.
    let state_dir = match resolve_state_dir_from_common(&args.common) {
        Ok(dir) => dir,
        Err(e) => {
            tracing::error!("{e}");
            std::process::exit(1);
        }
    };

    let mut handler = SshAgentHandler { state_dir };
    if let Err(e) = uptrakit_service_sdk::run_service_lifecycle(&args.common, &mut handler).await {
        if e.current_context().is_receive_closed() {
            tracing::info!("disconnected by controller");
        } else {
            tracing::error!(error = %e, "agent-ssh failed");
            std::process::exit(1);
        }
    }
}

/// Resolve the state directory for this service.
fn resolve_state_dir_from_common(
    common: &uptrakit_service_sdk::cli::CommonServiceArgs,
) -> InitResult<std::path::PathBuf> {
    let dirs = common.resolve_dirs("agent-ssh").map_err(|e| {
        report!(InitError::Directory(format!(
            "failed to resolve directories: {e}"
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
            uptrakit_shared_db::crypto::init_master_key(key_bytes).map_err(|e| {
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

fn print_build_info() {
    let build_info = BuildInfo::current(
        "uptrakit-agent-ssh",
        env!("CARGO_PKG_VERSION"),
        option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
    );
    let output = build_info.render_human();
    print!("{output}");
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
