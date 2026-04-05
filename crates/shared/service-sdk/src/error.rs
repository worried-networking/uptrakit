use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

// ── Domain sub-enums ──────────────────────────────────────────────────

/// TLS and certificate parsing errors.
#[derive(Debug, Error)]
pub enum TlsError {
    #[error("TLS error: {0}")]
    Config(String),
    #[error("TLS name validation error: {0}")]
    InvalidDnsName(#[from] rustls::pki_types::InvalidDnsNameError),
    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("PEM parsing error: {0}")]
    Pem(#[from] rustls::pki_types::pem::Error),
    #[error("no certificates found in CA response")]
    NoCertificates,
    #[error("certificate parse error: {0}")]
    CertificateParse(String),
}

/// Identity and key management errors.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("keypair generation failed: {0}")]
    KeypairGeneration(String),
    #[error("CSR generation error: {0}")]
    CsrGeneration(String),
    #[error("identity not enrolled")]
    NotEnrolled,
    #[error("identity not certified (no certificate)")]
    NotCertified,
}

/// Enrollment protocol and connection errors.
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("initialization failed: {0}")]
    Init(String),
    #[error("connection closed by controller")]
    ReceiveClosed,
    #[error("unexpected message type")]
    UnexpectedMessage,
    #[error("enrollment failed: {0}")]
    Enrollment(String),
    #[error("enrollment rejected by controller")]
    EnrollmentRejected,
    #[error("timed out waiting for approval")]
    ApprovalTimeout,
    #[error("timed out waiting for response")]
    ResponseTimeout,
    #[error("TCP connection timed out")]
    ConnectionTimeout,
    #[error("timed out sending message to controller")]
    SendTimeout,
    #[error("protocol version mismatch: expected {expected}, received {received}")]
    VersionMismatch { expected: u32, received: u32 },
}

/// CA certificate fetch and bootstrap errors.
#[derive(Debug, Error)]
pub enum CaError {
    #[error("failed to fetch CA certificate: {0}")]
    Fetch(String),
    #[error("failed to read CA certificate file: {0}")]
    CertFile(String),
}

// ── Top-level error ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum EnrollmentError {
    #[error(transparent)]
    Tls(TlsError),
    #[error(transparent)]
    Identity(IdentityError),
    #[error(transparent)]
    Protocol(ProtocolError),
    #[error(transparent)]
    Ca(CaError),
    #[error("I/O error: {0}")]
    Io(std::io::Error),
    #[error("JSON serialization error: {0}")]
    Json(serde_json::Error),
    #[error("WebSocket error: {0}")]
    WebSocket(tokio_tungstenite::tungstenite::Error),
    #[error("HTTP URI parsing error: {0}")]
    HttpUri(http::uri::InvalidUri),
    #[error("directory operation failed")]
    Directory(uptrakit_directories::DirectoryError),
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

/// Check if an `std::io::Error` wraps a rustls `CertificateRevoked` alert.
pub fn is_rustls_cert_revoked(io_err: &std::io::Error) -> bool {
    if let Some(inner) = io_err.get_ref()
        && let Some(rustls_err) = inner.downcast_ref::<rustls::Error>()
    {
        return matches!(
            rustls_err,
            rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRevoked)
        );
    }
    false
}

impl EnrollmentError {
    /// `true` when the controller closed the connection (normal during merges or restarts).
    pub fn is_receive_closed(&self) -> bool {
        matches!(self, Self::Protocol(ProtocolError::ReceiveClosed))
    }

    /// `true` for transient transport-level failures that should be retried.
    ///
    /// The following are considered transient:
    /// - [`EnrollmentError::WebSocket`] — network-level connection error (TCP
    ///   refused, DNS failure, TLS handshake, etc.), except
    ///   `WebSocket(Io(_))` wrapping rustls certificate-expired/revoked alerts.
    /// - [`EnrollmentError::Io`] — TCP reset, connection refused, etc.,
    ///   **unless** the underlying error wraps rustls `CertificateExpired` or
    ///   `CertificateRevoked` alerts.
    /// - [`ProtocolError::ConnectionTimeout`] — explicit TCP connect timeout.
    /// - [`ProtocolError::SendTimeout`] — write blocked until the OS buffer filled
    ///   (controller stopped consuming); the connection is dead and should reconnect.
    ///
    /// The following are **not** retried:
    /// - `Protocol::EnrollmentRejected` — server explicitly refused the token.
    /// - `Protocol::VersionMismatch` — incompatible protocol version.
    /// - `Protocol::UnexpectedMessage` — protocol violation.
    /// - `Identity::*` — key / CSR generation failures (local misconfiguration).
    /// - `Json::*` — protocol-level serialization error.
    /// - `Directory::*` — filesystem access failure.
    /// - `Tls::InvalidDnsName` — hostname is malformed (permanent).
    ///
    /// Note: `EnrollmentError::Tls(TlsError::Rustls(...))` is also non-transient.
    /// There is no `Self::Tls(_)` match arm below, so TLS errors fall through
    /// to `_ => false`.
    pub fn is_transient_network(&self) -> bool {
        match self {
            Self::WebSocket(tokio_tungstenite::tungstenite::Error::Io(io_err)) => {
                !is_rustls_cert_expired(io_err) && !is_rustls_cert_revoked(io_err)
            }
            Self::WebSocket(_) => true,
            Self::Io(e) if !is_rustls_cert_expired(e) && !is_rustls_cert_revoked(e) => true,
            Self::Protocol(ProtocolError::ConnectionTimeout | ProtocolError::SendTimeout) => true,
            _ => false,
        }
    }

    /// `true` when the TLS handshake failed because the server considers our
    /// client certificate expired.
    pub fn is_cert_expired(&self) -> bool {
        match self {
            Self::Tls(TlsError::Rustls(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateExpired,
            ))) => true,
            Self::WebSocket(tokio_tungstenite::tungstenite::Error::Io(io_err)) => {
                is_rustls_cert_expired(io_err)
            }
            Self::Io(io_err) => is_rustls_cert_expired(io_err),
            _ => false,
        }
    }
}

// ── ReportConversion impls ───────────────────────────────────────────

impl_report_conversion! {
    std::io::Error                        => EnrollmentError::Io,
    serde_json::Error                     => EnrollmentError::Json,
    tokio_tungstenite::tungstenite::Error => EnrollmentError::WebSocket,
    http::uri::InvalidUri                 => EnrollmentError::HttpUri,
    uptrakit_directories::DirectoryError  => EnrollmentError::Directory,
    TlsError                              => EnrollmentError::Tls,
    IdentityError                         => EnrollmentError::Identity,
    ProtocolError                         => EnrollmentError::Protocol,
    CaError                               => EnrollmentError::Ca,
}

impl_report_conversion!(rustls::pki_types::InvalidDnsNameError => EnrollmentError,
    |e| EnrollmentError::Tls(TlsError::InvalidDnsName(e)));
impl_report_conversion!(rustls::Error => EnrollmentError,
    |e| EnrollmentError::Tls(TlsError::Rustls(e)));
impl_report_conversion!(rustls::pki_types::pem::Error => EnrollmentError,
    |e| EnrollmentError::Tls(TlsError::Pem(e)));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_cert_expired_rustls_direct() {
        let err = EnrollmentError::Tls(TlsError::Rustls(rustls::Error::AlertReceived(
            rustls::AlertDescription::CertificateExpired,
        )));
        assert!(err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_rustls_different_alert() {
        let err = EnrollmentError::Tls(TlsError::Rustls(rustls::Error::AlertReceived(
            rustls::AlertDescription::HandshakeFailure,
        )));
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
        let err = EnrollmentError::Protocol(ProtocolError::ReceiveClosed);
        assert!(!err.is_cert_expired());
    }

    #[test]
    fn is_cert_revoked_rustls_direct() {
        let err = EnrollmentError::Tls(TlsError::Rustls(rustls::Error::AlertReceived(
            rustls::AlertDescription::CertificateRevoked,
        )));
        assert!(!err.is_cert_expired());
        assert!(!err.is_transient_network());
    }

    #[test]
    fn is_cert_revoked_websocket_io() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRevoked);
        let io_err = std::io::Error::other(rustls_err);
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(io_err));
        assert!(!err.is_cert_expired());
        assert!(!err.is_transient_network());
    }

    #[test]
    fn is_cert_revoked_io_direct() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRevoked);
        let io_err = std::io::Error::other(rustls_err);
        let err = EnrollmentError::Io(io_err);
        assert!(!err.is_cert_expired());
        assert!(!err.is_transient_network());
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

    #[test]
    fn is_rustls_cert_revoked_helper_positive() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRevoked);
        let io_err = std::io::Error::other(rustls_err);
        assert!(is_rustls_cert_revoked(&io_err));
    }

    #[test]
    fn is_rustls_cert_revoked_helper_false_for_expired() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        assert!(!is_rustls_cert_revoked(&io_err));
    }

    #[test]
    fn is_rustls_cert_revoked_helper_false_for_plain_io() {
        let io_err = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
        assert!(!is_rustls_cert_revoked(&io_err));
    }

    // ── is_transient_network tests ────────────────────────────────────────

    #[test]
    fn is_transient_network_websocket_error() {
        let err =
            EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::ConnectionClosed);
        assert!(err.is_transient_network());
    }

    #[test]
    fn websocket_protocol_is_transient() {
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        ));
        assert!(err.is_transient_network());
    }

    #[test]
    fn plain_websocket_io_is_transient() {
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::from(std::io::ErrorKind::ConnectionReset),
        ));
        assert!(!err.is_cert_expired());
        assert!(err.is_transient_network());
    }

    #[test]
    fn is_transient_network_io_not_cert_expired() {
        let err = EnrollmentError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        ));
        assert!(err.is_transient_network());
    }

    #[test]
    fn is_transient_network_connection_timeout() {
        let err = EnrollmentError::Protocol(ProtocolError::ConnectionTimeout);
        assert!(err.is_transient_network());
    }

    #[test]
    fn is_transient_network_send_timeout() {
        let err = EnrollmentError::Protocol(ProtocolError::SendTimeout);
        assert!(err.is_transient_network());
    }

    #[test]
    fn is_transient_network_enrollment_rejected() {
        let err = EnrollmentError::Protocol(ProtocolError::EnrollmentRejected);
        assert!(!err.is_transient_network());
    }

    #[test]
    fn receive_closed_not_transient() {
        let err = EnrollmentError::Protocol(ProtocolError::ReceiveClosed);
        assert!(err.is_receive_closed());
        assert!(!err.is_transient_network());
    }

    #[test]
    fn is_transient_network_cert_expired_io() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        let err = EnrollmentError::Io(io_err);
        // Cert-expired IO errors must NOT be retried.
        assert!(err.is_cert_expired());
        assert!(!err.is_transient_network());
    }

    #[test]
    fn is_transient_network_websocket_cert_expired_io() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(io_err));
        assert!(err.is_cert_expired());
        assert!(!err.is_transient_network());
    }

    #[test]
    fn is_transient_network_websocket_cert_revoked_io() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRevoked);
        let io_err = std::io::Error::other(rustls_err);
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(io_err));
        assert!(!err.is_transient_network());
    }

    #[test]
    fn is_transient_network_cert_revoked_io() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRevoked);
        let io_err = std::io::Error::other(rustls_err);
        let err = EnrollmentError::Io(io_err);
        assert!(!err.is_transient_network());
    }
}
