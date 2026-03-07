use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum AgentCoreError {
    // ── Update execution ──────────────────────────────────────────────
    #[error("update execution failed: {0}")]
    UpdateExecution(String),

    #[error("Pre-update hook failed: {0}")]
    PreUpdateHookFailed(String),

    #[error("Post-update hook failed: {0}")]
    PostUpdateHookFailed(String),

    /// Plugin's `pre_update_hook` signalled that the update should not proceed.
    #[error("plugin pre-update hook aborted the update: {0}")]
    PluginPreUpdateHookAborted(String),

    /// Plugin's `post_update_hook` failed (logged as warning; non-fatal).
    #[error("plugin post-update hook failed (non-fatal): {0}")]
    PluginPostUpdateHookFailed(String),

    // ── Attestation ───────────────────────────────────────────────────
    /// GitHub Actions attestation check failed; update aborted by policy.
    #[error("attestation verification failed: {0}")]
    AttestationFailed(String),

    // ── I/O ───────────────────────────────────────────────────────────
    #[error("I/O error: {0}")]
    Io(std::io::Error),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<AgentCoreError>>;

impl_report_conversion! {
    std::io::Error => AgentCoreError::Io,
}
