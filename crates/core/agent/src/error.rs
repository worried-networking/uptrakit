use rootcause::{Report, ReportConversion, markers};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    // External error types with #[from]
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("TLS name validation error: {0}")]
    TlsName(#[from] rustls::pki_types::InvalidDnsNameError),

    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),

    #[error("PEM parsing error: {0}")]
    Pem(#[from] rustls::pki_types::pem::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP URI parsing error: {0}")]
    HttpUri(#[from] http::uri::InvalidUri),

    // Context-specific variants (keep these - they add semantic meaning)
    #[error("failed to fetch CA certificate: {0}")]
    FetchCa(String),

    #[error("failed to read CA certificate file: {0}")]
    CaCertFile(String),

    #[error("no certificates found in CA response")]
    NoCertificates,

    #[error("connection closed by controller")]
    ReceiveClosed,

    #[error("unexpected message type")]
    UnexpectedMessage,

    #[error("enrollment failed: {0}")]
    Enrollment(String),

    #[error("enrollment rejected by controller")]
    EnrollmentRejected,
}

impl Error {
    pub fn is_receive_closed(&self) -> bool {
        matches!(self, Error::ReceiveClosed)
    }

    /// Returns `true` when the TLS handshake failed because the server
    /// considers our client certificate expired.
    pub fn is_cert_expired(&self) -> bool {
        let msg = match self {
            Error::WebSocket(e) => e.to_string(),
            Error::Io(e) => e.to_string(),
            _ => return false,
        };
        msg.contains("CertificateExpired")
    }
}

pub type Result<T> = std::result::Result<T, Report<Error>>;

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

impl<T> ReportConversion<rustls::pki_types::InvalidDnsNameError, markers::Mutable, T> for Error
where
    Error: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<rustls::pki_types::InvalidDnsNameError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(Error::TlsName)
    }
}

impl<T> ReportConversion<rustls::Error, markers::Mutable, T> for Error
where
    Error: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<rustls::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(Error::Rustls)
    }
}

impl<T> ReportConversion<rustls::pki_types::pem::Error, markers::Mutable, T> for Error
where
    Error: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<rustls::pki_types::pem::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(Error::Pem)
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

impl<T> ReportConversion<http::uri::InvalidUri, markers::Mutable, T> for Error
where
    Error: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<http::uri::InvalidUri, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(Error::HttpUri)
    }
}
