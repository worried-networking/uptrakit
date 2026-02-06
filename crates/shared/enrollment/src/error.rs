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

impl EnrollmentError {
    /// `true` when the controller closed the connection (normal during merges or restarts).
    pub fn is_receive_closed(&self) -> bool {
        matches!(self, EnrollmentError::ReceiveClosed)
    }

    /// `true` when the TLS handshake failed because the server considers our
    /// client certificate expired.
    pub fn is_cert_expired(&self) -> bool {
        let msg = match self {
            EnrollmentError::WebSocket(e) => e.to_string(),
            EnrollmentError::Io(e) => e.to_string(),
            _ => return false,
        };
        msg.contains("CertificateExpired")
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
