use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

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

impl_report_conversion! {
    EnrollmentError => AppError::Enrollment,
    ControllerError => AppError::Connection,
    uptrakit_directories::DirectoryError => AppError::Directory,
}
