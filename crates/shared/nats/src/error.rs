use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum NatsError {
    #[error("{0}")]
    Connection(String),
    #[error("{0}")]
    JetStream(String),
}

impl_report_conversion!(async_nats::ConnectError => NatsError,
    |e| NatsError::Connection(e.to_string())
);
