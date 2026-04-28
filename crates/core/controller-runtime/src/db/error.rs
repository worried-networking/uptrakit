use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub(crate) enum DbError {
    #[error("database connection error: {0}")]
    Connection(String),

    #[error("database migration error: {0}")]
    Migration(String),

    #[error("database configuration error: {0}")]
    Configuration(String),

    #[error("database error: {0}")]
    SeaOrm(#[from] sea_orm::DbErr),
}

pub(crate) type Result<T> = std::result::Result<T, Report<DbError>>;

impl_report_conversion!(sea_orm::DbErr => DbError::SeaOrm);
