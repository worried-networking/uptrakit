use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_service_sdk::EnrollmentError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum Error {
    // ── Enrollment (delegates to enrollment crate) ────────────────────
    #[error(transparent)]
    Enrollment(#[from] EnrollmentError),

    // ── I/O (needed for file operations in authenticated loop) ────────
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    // ── Directory operations ────────────────────────────────────────
    #[error("directory operation failed")]
    Directory(#[from] uptrakit_directories::DirectoryError),

    // ── Database operations ─────────────────────────────────────────
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    // ── Host management ─────────────────────────────────────────────
    #[error("host not found: {0}")]
    HostNotFound(String),

    #[error("host name already exists: {0}")]
    HostNameConflict(String),

    #[error("unsupported SSH key type: {0}")]
    UnsupportedKeyType(String),

    #[error("crypto error: {0}")]
    Crypto(String),

    // ── SSH transport (bootstrap) ────────────────────────────────────
    #[error("SSH connection failed: {0}")]
    SshConnection(String),

    #[error("SSH authentication failed: {0}")]
    SshAuth(String),

    #[error("SSH remote command failed: {0}")]
    SshCommand(String),

    #[error("host key mismatch: expected {expected}, observed {observed}")]
    HostKeyMismatch { expected: String, observed: String },

    #[error("SSH key generation failed: {0}")]
    KeyGeneration(String),

    #[error("bootstrap verification failed: {0}")]
    BootstrapVerification(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

pub type Result<T> = std::result::Result<T, Report<Error>>;

impl_report_conversion! {
    EnrollmentError => Error::Enrollment,
    std::io::Error => Error::Io,
    uptrakit_directories::DirectoryError => Error::Directory,
    sea_orm::DbErr => Error::Database,
}

// russh::Error doesn't impl std::error::Error in a compatible way, so use String-based conversion.
impl_report_conversion!(russh::Error => Error, |e| Error::SshConnection(e.to_string()));

// russh agent auth errors use String-based conversion (AgentAuthError wraps SendError + keys::Error).
impl_report_conversion!(russh::AgentAuthError => Error, |e| Error::SshAuth(e.to_string()));

// CommandError from uptrakit-command (used by SSH remote command execution).
impl_report_conversion!(uptrakit_command::CommandError => Error, |e| Error::SshCommand(e.to_string()));

// PluginError from infra plugin `HostLifecycle` hooks (e.g. Proxmox PVE node
// detection) — surfaced through `collect_infra_results` so a transport error
// during infra detection propagates loudly instead of being conflated with a
// verified "not this infra" result.
impl_report_conversion!(
    uptrakit_plugin_infrastructure_registry::PluginError => Error,
    |e| Error::BootstrapVerification(e.to_string())
);

// SudoersError from the shared sudoers-provisioning helpers in uptrakit-agent-core
// (moved out of this crate; used by both bootstrap and host-sync sudoers writes).
impl_report_conversion!(uptrakit_agent_core::sudoers::SudoersError => Error, |e| Error::SshCommand(e.to_string()));
