mod cli;
mod host_cli;

use std::collections::BTreeSet;
use std::sync::Arc;

use clap::Parser;
use rootcause::prelude::*;
use uptrakit_agent_ssh::runtime_support::AgentSshRuntimeSupport;
use uptrakit_agent_ssh::{
    db, init_ssh_data_key_ring, reencrypt_ssh_to_v3, register_ssh_column_aad, ssh_pool,
};
use uptrakit_agent_ssh_runtime::{
    SshAgentEvent, SshAgentIdentity, SshAgentRuntime, SshAgentRuntimeConfig, SshAgentSettings,
    ssh_agent_capabilities,
};
use uptrakit_audit_log::RuntimeAuditEmitter;
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, default_resolve_shutdown,
};
use uptrakit_wire::Capability;

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

struct SshAgentHandler {
    runtime: SshAgentRuntime<AgentSshRuntimeSupport>,
}

#[async_trait::async_trait]
impl ServiceHandler for SshAgentHandler {
    const DIR_NAME: &'static str = "agent-ssh";
    const SERVICE_LABEL: &'static str = "uptrakit-agent-ssh service";
    const SERVICE_APP_NAME: &'static str = env!("CARGO_PKG_NAME");

    type ServiceEvent = SshAgentEvent;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        let encryption_public_key = identity.public_key_raw().map(|bytes| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        });

        self.runtime
            .on_connected(
                conn,
                SshAgentIdentity {
                    service_id: identity.service_id(),
                    private_key_der: identity.private_key_pkcs8_der(),
                    encryption_public_key,
                },
            )
            .await
            .map_err(|error| rootcause::Report::new(LoopError::Other(error.to_string())))
    }

    async fn on_message(
        &mut self,
        msg: uptrakit_wire::ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        self.runtime.handle_controller_message(msg, conn).await;
        Ok(None)
    }

    async fn on_settings(
        &mut self,
        settings: &uptrakit_wire::ServiceSettingsPayload,
        conn: &mut ControllerConnection,
    ) {
        if let Err(error) = self
            .runtime
            .apply_settings(
                SshAgentSettings {
                    tenant_id: settings.tenant_id,
                    ui_surfaces_enabled: conn
                        .agreed_capabilities()
                        .contains(&Capability::UiSurfaces),
                    persist_tenant_id: true,
                },
                conn,
            )
            .await
        {
            tracing::warn!(error = %error, "failed to apply SSH agent service settings");
        }
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        ssh_agent_capabilities()
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        self.runtime.poll_event().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        Ok(self.runtime.handle_event(event, conn).await)
    }

    fn on_surface_action_response(
        &mut self,
        response: uptrakit_wire::surfaces::SurfaceActionResponse,
    ) {
        self.runtime.handle_surface_action_response(response);
    }

    async fn on_surface_action_request(
        &mut self,
        request: uptrakit_wire::surfaces::SurfaceActionRequest,
        conn: &mut ControllerConnection,
    ) -> LoopResult<()> {
        self.runtime
            .handle_controller_message(
                uptrakit_wire::ControllerMessage::SurfaceActionRequest(request),
                conn,
            )
            .await;
        Ok(())
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        shutdown_timeout: std::time::Duration,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = default_resolve_shutdown(cause);
        self.runtime
            .shutdown(conn, shutdown_timeout, disconnect_reason, outcome)
            .await
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

    let infra_bundles = {
        let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
        #[expect(
            clippy::expect_used,
            reason = "infallible at startup: catalog construction failures are static configuration errors that must abort process initialization"
        )]
        let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(&catalog_config)
            .expect("plugin catalog must build successfully");
        Arc::new(catalog.create_infra_bundles(&catalog_config))
    };
    let surface_proxy = Arc::new(uptrakit_service_sdk::ServiceSurfaceProxy::new());
    let support = AgentSshRuntimeSupport::new(
        local_db,
        state_dir.clone(),
        ssh_pool::SshConnectionPool::new(),
        surface_proxy,
        infra_bundles,
        true,
    );

    let mut handler = SshAgentHandler {
        runtime: SshAgentRuntime::new(SshAgentRuntimeConfig::with_audit_emitter(
            support,
            state_dir.join("update-freeze"),
            RuntimeAuditEmitter::new(),
        )),
    };

    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-agent-ssh",
        &args.common,
        &mut handler,
    )
    .await;
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

async fn rotate_ssh_master_key(db: &sea_orm::DatabaseConnection, new_key_path: &std::path::Path) {
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, TransactionTrait};

    let new_key_hex = match std::fs::read_to_string(new_key_path) {
        Ok(contents) => contents,
        Err(error) => {
            tracing::error!(path = %new_key_path.display(), error = %error, "failed to read new master key file");
            return;
        }
    };

    let new_key_bytes = match uptrakit_shared_types::hex::decode(new_key_hex.trim()) {
        Ok(bytes) => {
            let array: [u8; 32] = match bytes.try_into() {
                Ok(array) => array,
                Err(_) => {
                    tracing::error!("new master key must be exactly 32 bytes (64 hex chars)");
                    return;
                }
            };
            array
        }
        Err(error) => {
            tracing::error!(error = %error, "failed to decode new master key hex");
            return;
        }
    };
    let new_kek = zeroize::Zeroizing::new(new_key_bytes);

    let new_kek_fp = {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(new_kek.as_slice());
        #[expect(
            clippy::indexing_slicing,
            reason = "infallible: `Sha256::digest` always returns 32 bytes, so `hash[..8]` is always in range"
        )]
        let prefix = &hash[..8];
        uptrakit_shared_types::hex::encode(prefix)
    };

    let current_kek_fp = match uptrakit_crypto::master_key_fingerprint() {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            tracing::error!(error = %error, "failed to compute KEK fingerprint");
            return;
        }
    };

    if new_kek_fp == current_kek_fp {
        tracing::warn!("new master key has same fingerprint as current — no rotation needed");
        return;
    }

    let txn = match db.begin().await {
        Ok(txn) => txn,
        Err(error) => {
            tracing::error!(error = %error, "failed to begin transaction for key rotation");
            return;
        }
    };

    let rows = match uptrakit_agent_ssh::db::entity::data_encryption_key::Entity::find()
        .all(&txn)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(error = %error, "failed to query DEKs for rotation");
            return;
        }
    };

    for row in &rows {
        let dek = match uptrakit_crypto::unwrap_data_key(&row.wrapped_key, &row.key_id) {
            Ok(dek) => dek,
            Err(error) => {
                tracing::error!(key_id = %row.key_id, error = %error, "failed to unwrap DEK");
                return;
            }
        };
        let new_wrapped = match uptrakit_crypto::wrap_data_key_with(&new_kek, &dek) {
            Ok(wrapped) => wrapped,
            Err(error) => {
                tracing::error!(key_id = %row.key_id, error = %error, "failed to re-wrap DEK");
                return;
            }
        };
        let mut active_model: uptrakit_agent_ssh::db::entity::data_encryption_key::ActiveModel =
            row.clone().into_active_model();
        active_model.wrapped_key = sea_orm::Set(new_wrapped);
        active_model.kek_fingerprint = sea_orm::Set(new_kek_fp.clone());
        if let Err(error) = active_model.update(&txn).await {
            tracing::error!(key_id = %row.key_id, error = %error, "failed to update DEK row");
            return;
        }
    }

    if let Err(error) = txn.commit().await {
        tracing::error!(error = %error, "failed to commit key rotation transaction");
        return;
    }

    tracing::info!(
        dek_count = rows.len(),
        new_kek_fp,
        "SSH agent master key rotation complete — restart with the new key file"
    );
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
