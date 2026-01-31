use rootcause::{Report, ReportConversion, markers};

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Not logged in. Run `uptrakit-cli auth login` first.")]
    NotLoggedIn,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Report<CliError>>;

impl<T> ReportConversion<reqwest::Error, markers::Mutable, T> for CliError
where
    CliError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<reqwest::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(CliError::Http)
    }
}

impl<T> ReportConversion<std::io::Error, markers::Mutable, T> for CliError
where
    CliError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<std::io::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(CliError::Io)
    }
}

impl<T> ReportConversion<serde_json::Error, markers::Mutable, T> for CliError
where
    CliError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<serde_json::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(CliError::Json)
    }
}
