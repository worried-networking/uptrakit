use rootcause::ReportConversion;
use rootcause::prelude::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database connection error: {0}")]
    Connection(String),

    #[error("database migration error: {0}")]
    Migration(String),

    #[error("database configuration error: {0}")]
    Configuration(String),

    #[error("database error: {0}")]
    SeaOrm(#[from] sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, Report<DbError>>;

// ReportConversion for SeaORM errors
impl<T> ReportConversion<sea_orm::DbErr, markers::Mutable, T> for DbError
where
    DbError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<sea_orm::DbErr, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(DbError::SeaOrm)
    }
}
