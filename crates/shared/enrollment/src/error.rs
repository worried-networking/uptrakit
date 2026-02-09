use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum EnrollmentError {
    // ── I/O and serialization ────────────────────────────────────────
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    // ── TLS / certificate parsing ────────────────────────────────────
    #[error("TLS error: {0}")]
    Tls(String),

    #[error("TLS name validation error: {0}")]
    TlsName(#[from] rustls::pki_types::InvalidDnsNameError),

    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),

    #[error("PEM parsing error: {0}")]
    Pem(#[from] rustls::pki_types::pem::Error),

    #[error("no certificates found in CA response")]
    NoCertificates,

    #[error("certificate parse error: {0}")]
    CertificateParse(String),

    // ── Key / CSR generation ─────────────────────────────────────────
    #[error("keypair generation failed: {0}")]
    KeypairGeneration(String),

    #[error("CSR generation error: {0}")]
    CsrGeneration(String),

    // ── WebSocket / HTTP ─────────────────────────────────────────────
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("HTTP URI parsing error: {0}")]
    HttpUri(#[from] http::uri::InvalidUri),

    // ── Enrollment protocol ──────────────────────────────────────────
    #[error("connection closed by controller")]
    ReceiveClosed,

    #[error("unexpected message type")]
    UnexpectedMessage,

    #[error("enrollment failed: {0}")]
    Enrollment(String),

    #[error("enrollment rejected by controller")]
    EnrollmentRejected,

    // ── Identity state ───────────────────────────────────────────────
    #[error("identity not enrolled")]
    NotEnrolled,

    #[error("identity not certified (no certificate)")]
    NotCertified,

    // ── CA fetch / bootstrap ─────────────────────────────────────────
    #[error("failed to fetch CA certificate: {0}")]
    FetchCa(String),

    #[error("failed to read CA certificate file: {0}")]
    CaCertFile(String),
}

pub type Result<T> = std::result::Result<T, Report<EnrollmentError>>;

/// Check if an `std::io::Error` wraps a rustls `CertificateExpired` alert.
///
/// When the TLS handshake fails with a `CertificateExpired` alert, `rustls`
/// produces `rustls::Error::AlertReceived(AlertDescription::CertificateExpired)`
/// which is wrapped in `std::io::Error` (custom kind). This function downcasts
/// through the error chain to detect that specific case without relying on
/// string representations.
pub fn is_rustls_cert_expired(io_err: &std::io::Error) -> bool {
    if let Some(inner) = io_err.get_ref()
        && let Some(rustls_err) = inner.downcast_ref::<rustls::Error>()
    {
        return matches!(
            rustls_err,
            rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired)
        );
    }
    false
}

impl EnrollmentError {
    /// `true` when the controller closed the connection (normal during merges or restarts).
    pub fn is_receive_closed(&self) -> bool {
        matches!(self, EnrollmentError::ReceiveClosed)
    }

    /// `true` when the TLS handshake failed because the server considers our
    /// client certificate expired.
    pub fn is_cert_expired(&self) -> bool {
        match self {
            EnrollmentError::Rustls(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateExpired,
            )) => true,
            EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(io_err)) => {
                is_rustls_cert_expired(io_err)
            }
            EnrollmentError::Io(io_err) => is_rustls_cert_expired(io_err),
            _ => false,
        }
    }
}

// ── ReportConversion impls ───────────────────────────────────────────

impl_report_conversion! {
    std::io::Error                          => EnrollmentError::Io,
    serde_json::Error                       => EnrollmentError::Json,
    rustls::pki_types::InvalidDnsNameError  => EnrollmentError::TlsName,
    rustls::Error                           => EnrollmentError::Rustls,
    rustls::pki_types::pem::Error           => EnrollmentError::Pem,
    tokio_tungstenite::tungstenite::Error   => EnrollmentError::WebSocket,
    http::uri::InvalidUri                   => EnrollmentError::HttpUri,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_cert_expired_rustls_direct() {
        let err = EnrollmentError::Rustls(rustls::Error::AlertReceived(
            rustls::AlertDescription::CertificateExpired,
        ));
        assert!(err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_rustls_different_alert() {
        let err = EnrollmentError::Rustls(rustls::Error::AlertReceived(
            rustls::AlertDescription::HandshakeFailure,
        ));
        assert!(!err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_io_wrapping_rustls() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        let err = EnrollmentError::Io(io_err);
        assert!(err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_ws_wrapping_rustls() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        let ws_err = tokio_tungstenite::tungstenite::Error::Io(io_err);
        let err = EnrollmentError::WebSocket(ws_err);
        assert!(err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_plain_io_error() {
        let err = EnrollmentError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert!(!err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_unrelated_variant() {
        let err = EnrollmentError::ReceiveClosed;
        assert!(!err.is_cert_expired());
    }

    #[test]
    fn is_rustls_cert_expired_helper_positive() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        assert!(is_rustls_cert_expired(&io_err));
    }

    #[test]
    fn is_rustls_cert_expired_helper_different_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
        assert!(!is_rustls_cert_expired(&io_err));
    }
}
