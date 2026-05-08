mod cli;
mod host_cli;

use clap::Parser;
use rootcause::prelude::*;

use uptrakit_agent_ssh_runtime::{
    AgentSshHandler, AgentSshMode, db, init_ssh_data_key_ring, reencrypt_ssh_to_v3,
    register_ssh_column_aad, rotate_ssh_master_key,
};
use uptrakit_service_sdk::run_lifecycle_and_handle_errors;

use cli::{Args, Commands};

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

    if let Some(Commands::Host { command }) = args.command {
        uptrakit_service_sdk::TracingBuilder::new()
            .verbosity(args.common.verbose)
            .init();

        if let Err(error) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
        register_ssh_column_aad();

        let state_dir = match resolve_state_dir_from_common(&args.common).await {
            Ok(dir) => dir,
            Err(error) => {
                eprintln!("error: {error}");
                std::process::exit(1);
            }
        };

        match db::init_db(&state_dir).await {
            Ok(host_db) => {
                init_ssh_data_key_ring(&host_db).await;
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "best-effort cleanup on subcommand exit; failures here are non-actionable"
                )]
                let _ = host_db.close().await;
            }
            Err(error) => {
                tracing::error!(error = %error, "could not init DEK ring for host subcommand");
            }
        }

        if let Err(error) = host_cli::run(&state_dir, command).await {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
        return;
    }

    if args.common.url.is_none() {
        eprintln!("error: --url is required for daemon mode");
        std::process::exit(1);
    }

    uptrakit_service_sdk::TracingBuilder::new()
        .verbosity(args.common.verbose)
        .init();
    uptrakit_service_sdk::init_crypto();

    if let Err(error) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
        tracing::error!("{error}");
        std::process::exit(1);
    }
    register_ssh_column_aad();

    let state_dir = match resolve_state_dir_from_common(&args.common).await {
        Ok(dir) => dir,
        Err(error) => {
            tracing::error!("{error}");
            std::process::exit(1);
        }
    };

    let local_db = match db::init_db(&state_dir).await {
        Ok(db) => db,
        Err(error) => {
            tracing::error!("failed to initialize local database: {error}");
            std::process::exit(1);
        }
    };

    init_ssh_data_key_ring(&local_db).await;
    reencrypt_ssh_to_v3(&local_db).await;

    if let Some(ref new_key_path) = args.rotate_master_key_file {
        rotate_ssh_master_key(&local_db, new_key_path).await;
    }

    let mut handler = AgentSshHandler::new(local_db, state_dir, AgentSshMode::Binary, None);

    run_lifecycle_and_handle_errors("uptrakit-agent-ssh", &args.common, &mut handler).await;
}

async fn resolve_state_dir_from_common(
    common: &uptrakit_service_sdk::cli::CommonServiceArgs,
) -> InitResult<std::path::PathBuf> {
    let dirs = common.resolve_dirs("agent-ssh").map_err(|error| {
        report!(InitError::Directory(format!(
            "failed to resolve directories: {error}"
        )))
    })?;
    dirs.ensure_state_dir().await.map_err(|error| {
        report!(InitError::Directory(format!(
            "failed to ensure state directory: {error}"
        )))
    })?;
    Ok(dirs.state_dir().to_path_buf())
}

fn init_master_key(
    master_key_file: &Option<std::path::PathBuf>,
    allow_plaintext_secrets: bool,
) -> InitResult<()> {
    let env_val = std::env::var("UPTRAKIT_MASTER_KEY").ok();
    // SAFETY: called early in `main` before any other thread is spawned, so no concurrent
    // reads or writes to the process environment can race with this removal.
    unsafe { std::env::remove_var("UPTRAKIT_MASTER_KEY") };
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
            uptrakit_crypto::init_master_key(zeroize::Zeroizing::new(key_bytes)).map_err(
                |error| {
                    report!(InitError::MasterKey(format!(
                        "failed to initialize master key: {error}"
                    )))
                },
            )?;
            tracing::info!("master encryption key initialized");
        }
        None => {
            if allow_plaintext_secrets {
                tracing::warn!(
                    "master encryption key not set; encryption at rest is disabled. \
                    This is for development only and is NOT safe for production."
                );
                uptrakit_crypto::enable_plaintext_mode();
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
        let contents =
            std::fs::read_to_string(key_file).map_err(|error| report!(InitError::Io(error)))?;
        return Ok(Some(contents.trim().to_string()));
    }

    if let Some(env_val) = env_val {
        return Ok(Some(env_val.trim().to_string()));
    }

    Ok(None)
}

fn parse_master_key_hex(key_hex: &str) -> InitResult<[u8; 32]> {
    let bytes = uptrakit_shared_types::hex::decode(key_hex).map_err(|error| {
        report!(InitError::Hex(format!(
            "master key must be a 64-character hex string: {error}"
        )))
    })?;
    let key_bytes: [u8; 32] = bytes.try_into().map_err(|value: Vec<u8>| {
        report!(InitError::Hex(format!(
            "master key must be exactly 32 bytes (64 hex chars), got {} bytes",
            value.len()
        )))
    })?;
    Ok(key_bytes)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test code: `assert!(r.is_err())` is idiomatic in tests where the error variant is not inspected"
    )]

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
            Ok(file) => file,
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
