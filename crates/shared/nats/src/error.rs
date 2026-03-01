use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum NatsError {
    #[error("NATS connection failed")]
    Connection,
    #[error("JetStream setup failed")]
    JetStream,
}

impl_report_conversion!(async_nats::ConnectError => NatsError,
    |_e| NatsError::Connection
);
