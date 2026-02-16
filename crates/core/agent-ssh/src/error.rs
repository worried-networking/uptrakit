use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_service_sdk::{EnrollmentError, is_rustls_cert_expired};
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
    Database(String),

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
            Error::Io(io_err) => is_rustls_cert_expired(io_err),
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, Report<Error>>;

impl_report_conversion! {
    EnrollmentError => Error::Enrollment,
    std::io::Error => Error::Io,
    uptrakit_directories::DirectoryError => Error::Directory,
}

// sea_orm::DbErr uses a String-based variant, so we use a closure conversion.
impl_report_conversion!(sea_orm::DbErr => Error, |e| Error::Database(e.to_string()));

// russh::Error doesn't impl std::error::Error in a compatible way, so use String-based conversion.
impl_report_conversion!(russh::Error => Error, |e| Error::SshConnection(e.to_string()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_receive_closed_enrollment_error() {
        let err = Error::Enrollment(EnrollmentError::Protocol(
            uptrakit_service_sdk::ProtocolError::ReceiveClosed,
        ));
        assert!(err.is_receive_closed());
    }

    #[test]
    fn is_receive_closed_other_error() {
        let err = Error::Database("test".to_string());
        assert!(!err.is_receive_closed());
    }

    #[test]
    fn is_receive_closed_io_error() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset",
        ));
        assert!(!err.is_receive_closed());
    }

    #[test]
    fn is_cert_expired_enrollment_delegates() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        let err = Error::Enrollment(EnrollmentError::Io(io_err));
        assert!(err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_enrollment_websocket_wrapping_rustls() {
        let rustls_err = rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired);
        let io_err = std::io::Error::other(rustls_err);
        let ws_err = tokio_tungstenite::tungstenite::Error::Io(io_err);
        let err = Error::Enrollment(EnrollmentError::WebSocket(ws_err));
        assert!(err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_unrelated_io_error() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert!(!err.is_cert_expired());
    }

    #[test]
    fn is_cert_expired_database_error() {
        let err = Error::Database("CertificateExpired".to_string());
        assert!(
            !err.is_cert_expired(),
            "should not match CertificateExpired in unrelated variant"
        );
    }

    #[test]
    fn host_not_found_display() {
        let err = Error::HostNotFound("test".to_string());
        assert_eq!(err.to_string(), "host not found: test");
    }

    #[test]
    fn host_name_conflict_display() {
        let err = Error::HostNameConflict("dup".to_string());
        assert_eq!(err.to_string(), "host name already exists: dup");
    }

    #[test]
    fn ssh_connection_display() {
        let err = Error::SshConnection("timeout".to_string());
        assert_eq!(err.to_string(), "SSH connection failed: timeout");
    }

    #[test]
    fn ssh_auth_display() {
        let err = Error::SshAuth("bad password".to_string());
        assert_eq!(err.to_string(), "SSH authentication failed: bad password");
    }

    #[test]
    fn ssh_command_display() {
        let err = Error::SshCommand("exit code 1".to_string());
        assert_eq!(err.to_string(), "SSH remote command failed: exit code 1");
    }

    #[test]
    fn host_key_mismatch_display() {
        let err = Error::HostKeyMismatch {
            expected: "SHA256:abc".to_string(),
            observed: "SHA256:xyz".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "host key mismatch: expected SHA256:abc, observed SHA256:xyz"
        );
    }

    #[test]
    fn key_generation_display() {
        let err = Error::KeyGeneration("RNG failure".to_string());
        assert_eq!(err.to_string(), "SSH key generation failed: RNG failure");
    }

    #[test]
    fn bootstrap_verification_display() {
        let err = Error::BootstrapVerification("whoami mismatch".to_string());
        assert_eq!(
            err.to_string(),
            "bootstrap verification failed: whoami mismatch"
        );
    }

    #[test]
    fn invalid_input_display() {
        let err = Error::InvalidInput("bad username".to_string());
        assert_eq!(err.to_string(), "invalid input: bad username");
    }
}
