use rootcause::{Report, ReportConversion, markers};
use thiserror::Error;

use crate::controller_client::ControllerError;
use crate::identity::IdentityError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("connection error: {0}")]
    Connection(#[from] ControllerError),

    #[error("identity error: {0}")]
    Identity(#[from] IdentityError),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("directory error: {0}")]
    Directory(#[from] uptrakit_directories::DirectoryError),
}

pub type Result<T> = std::result::Result<T, Report<AppError>>;

impl<T> ReportConversion<ControllerError, markers::Mutable, T> for AppError
where
    AppError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<ControllerError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(AppError::Connection)
    }
}

impl<T> ReportConversion<IdentityError, markers::Mutable, T> for AppError
where
    AppError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<IdentityError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(AppError::Identity)
    }
}

impl<T> ReportConversion<uptrakit_directories::DirectoryError, markers::Mutable, T> for AppError
where
    AppError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<uptrakit_directories::DirectoryError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(AppError::Directory)
    }
}
