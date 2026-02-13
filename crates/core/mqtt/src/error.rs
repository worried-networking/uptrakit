use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_service_sdk::{EnrollmentError, is_rustls_cert_expired};
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum AppError {
    // ── Enrollment (delegates to SDK crate) ─────────────────────────────
    #[error(transparent)]
    Enrollment(#[from] EnrollmentError),

    // ── I/O (needed for signal setup) ───────────────────────────────────
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl AppError {
    /// `true` when the controller closed the connection.
    pub fn is_receive_closed(&self) -> bool {
        match self {
            AppError::Enrollment(e) => e.is_receive_closed(),
            _ => false,
        }
    }

    /// `true` when the TLS handshake failed because the server considers
    /// our client certificate expired.
    pub fn is_cert_expired(&self) -> bool {
        match self {
            AppError::Enrollment(e) => e.is_cert_expired(),
            AppError::Io(io_err) => is_rustls_cert_expired(io_err),
        }
    }
}

pub type Result<T> = std::result::Result<T, Report<AppError>>;

impl_report_conversion! {
    EnrollmentError => AppError::Enrollment,
    std::io::Error => AppError::Io,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_receive_closed_enrollment_error() {
        let err = AppError::Enrollment(EnrollmentError::ReceiveClosed);
        assert!(err.is_receive_closed());
    }

    #[test]
    fn is_receive_closed_io_error() {
        let err = AppError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset",
        ));
        assert!(!err.is_receive_closed());
    }

    #[test]
    fn is_cert_expired_enrollment_delegates() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        let err = AppError::Enrollment(EnrollmentError::Io(io_err));
        assert!(err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_enrollment_websocket_wrapping_rustls() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        let ws_err = tokio_tungstenite::tungstenite::Error::Io(io_err);
        let err = AppError::Enrollment(EnrollmentError::WebSocket(ws_err));
        assert!(err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_unrelated_io_error() {
        let err = AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert!(!err.is_cert_expired());
    }
}
