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

    // ── I/O ───────────────────────────────────────────────────────────
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<AgentCoreError>>;

impl_report_conversion! {
    std::io::Error => AgentCoreError::Io,
}
