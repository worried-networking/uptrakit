use thiserror::Error;

/// Errors that can occur in the audit log subsystem.
#[derive(Debug, Error)]
pub enum AuditLogError {
    #[error("audit log backend error: {0}")]
    Backend(String),

    #[error("audit log database error: {0}")]
    #[cfg(feature = "db")]
    Database(#[from] sea_orm::DbErr),
}

/// Convenience alias for audit log results.
pub type Result<T> = std::result::Result<T, rootcause::Report<AuditLogError>>;
