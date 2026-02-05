use rootcause::{Report, ReportConversion, markers};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnrollmentError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("keypair generation failed: {0}")]
    KeypairGeneration(String),

    #[error("CSR generation failed: {0}")]
    CsrGeneration(String),

    #[error("certificate parse error: {0}")]
    CertificateParse(String),

    #[error("identity not enrolled")]
    NotEnrolled,

    #[error("identity not certified (no certificate)")]
    NotCertified,
}

pub type Result<T> = std::result::Result<T, Report<EnrollmentError>>;

impl<T> ReportConversion<std::io::Error, markers::Mutable, T> for EnrollmentError
where
    EnrollmentError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<std::io::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(EnrollmentError::Io)
    }
}

impl<T> ReportConversion<serde_json::Error, markers::Mutable, T> for EnrollmentError
where
    EnrollmentError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<serde_json::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(EnrollmentError::Json)
    }
}
