use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_enrollment::EnrollmentError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum Error {
    // ── Enrollment (delegates to enrollment crate) ────────────────────
    #[error(transparent)]
    Enrollment(#[from] EnrollmentError),

    // ── I/O and serialization (needed in authenticated loop) ─────────
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    // ── Agent-specific ───────────────────────────────────────────────
    #[error("update execution failed: {0}")]
    UpdateExecution(String),

    #[error("Pre-update hook failed: {0}")]
    PreUpdateHookFailed(String),

    #[error("Post-update hook failed: {0}")]
    PostUpdateHookFailed(String),
}

impl Error {
    /// `true` when the controller closed the connection.
    pub fn is_receive_closed(&self) -> bool {
        match self {
            Error::Enrollment(e) => e.is_receive_closed(),
            _ => false,
        }
    }

    /// `true` when the TLS handshake failed because the server considers
    /// our client certificate expired.
    pub fn is_cert_expired(&self) -> bool {
        match self {
            Error::Enrollment(e) => e.is_cert_expired(),
            Error::WebSocket(e) => e.to_string().contains("CertificateExpired"),
            Error::Io(e) => e.to_string().contains("CertificateExpired"),
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, Report<Error>>;

impl_report_conversion! {
    EnrollmentError => Error::Enrollment,
    std::io::Error => Error::Io,
    tokio_tungstenite::tungstenite::Error => Error::WebSocket,
    serde_json::Error => Error::Json,
}
