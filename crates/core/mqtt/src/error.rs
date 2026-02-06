use rootcause::ReportConversion;
use rootcause::prelude::*;
use thiserror::Error;

use crate::controller_client::ControllerError;
use uptrakit_enrollment::EnrollmentError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("enrollment error: {0}")]
    Enrollment(#[from] EnrollmentError),

    #[error("connection error: {0}")]
    Connection(#[from] ControllerError),

    #[error("directory error: {0}")]
    Directory(#[from] uptrakit_directories::DirectoryError),
}

pub type Result<T> = std::result::Result<T, Report<AppError>>;

impl<T> ReportConversion<EnrollmentError, markers::Mutable, T> for AppError
where
    AppError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<EnrollmentError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(AppError::Enrollment)
    }
}

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
