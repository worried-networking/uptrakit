use thiserror::Error;

use crate::controller_client::ControllerError;
use crate::identity::IdentityError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("connection error: {0}")]
    Connection(#[from] ControllerError),

    #[error("identity error: {0}")]
    Identity(#[from] IdentityError),

    #[error("protocol error: {0}")]
    Protocol(String),
}
