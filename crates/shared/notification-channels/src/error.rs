//! Error types for notification channels.

use rootcause::prelude::*;
use thiserror::Error;

/// Errors originating from notification channel operations.
#[derive(Debug, Error)]
pub enum ChannelError {
    /// The channel-specific configuration is invalid.
    #[error("invalid channel config: {0}")]
    InvalidConfig(String),

    /// The notification delivery failed.
    #[error("delivery failed: {0}")]
    DeliveryFailed(String),

    /// An HTTP request to the channel's backend failed.
    #[error("HTTP request failed: {0}")]
    HttpRequest(String),

    /// Failed to build the HTTP client.
    #[error("failed to build HTTP client: {0}")]
    HttpClientBuild(String),

    /// Failed to serialize the notification payload.
    #[error("serialization failed: {0}")]
    Serialization(String),

    /// HMAC key construction failed.
    #[error("HMAC key error: {0}")]
    HmacKey(String),
}

/// Convenience type alias for channel results.
pub type Result<T> = std::result::Result<T, Report<ChannelError>>;
