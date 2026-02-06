use rootcause::ReportConversion;
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_enrollment::EnrollmentError;

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

// ── ReportConversion impls ───────────────────────────────────────────

impl<T> ReportConversion<EnrollmentError, markers::Mutable, T> for Error
where
    Error: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<EnrollmentError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(Error::Enrollment)
    }
}

impl<T> ReportConversion<std::io::Error, markers::Mutable, T> for Error
where
    Error: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<std::io::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(Error::Io)
    }
}

impl<T> ReportConversion<tokio_tungstenite::tungstenite::Error, markers::Mutable, T> for Error
where
    Error: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<tokio_tungstenite::tungstenite::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(Error::WebSocket)
    }
}

impl<T> ReportConversion<serde_json::Error, markers::Mutable, T> for Error
where
    Error: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<serde_json::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(Error::Json)
    }
}
