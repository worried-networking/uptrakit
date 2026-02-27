use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("{0}")]
    Execution(String),
    #[error("task timed out: {0:?}")]
    TaskTimedOut(ScheduledTaskType),
}

pub type Result<T> = std::result::Result<T, Report<SchedulerError>>;

impl_report_conversion!(sea_orm::DbErr => SchedulerError::Database);
