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

    // ── Agent-specific ───────────────────────────────────────────────
    #[error("update execution failed: {0}")]
    UpdateExecution(String),

    #[error("Pre-update hook failed: {0}")]
    PreUpdateHookFailed(String),

    #[error("Post-update hook failed: {0}")]
    PostUpdateHookFailed(String),
}

impl_report_conversion! {
    EnrollmentError => Error::Enrollment,
    std::io::Error => Error::Io,
    uptrakit_directories::DirectoryError => Error::Directory,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_execution_display() {
        let err = Error::UpdateExecution("timeout".to_string());
        assert_eq!(err.to_string(), "update execution failed: timeout");
    }

    #[test]
    fn pre_update_hook_failed_display() {
        let err = Error::PreUpdateHookFailed("exit code 1".to_string());
        assert_eq!(err.to_string(), "Pre-update hook failed: exit code 1");
    }

    #[test]
    fn post_update_hook_failed_display() {
        let err = Error::PostUpdateHookFailed("exit code 2".to_string());
        assert_eq!(err.to_string(), "Post-update hook failed: exit code 2");
    }
}
