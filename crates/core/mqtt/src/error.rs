use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("database error")]
    Database,
}
