//! Error types for notification plugins.

use rootcause::prelude::*;
use thiserror::Error;

/// Errors originating from notification plugin operations.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum NotificationPluginError {
    /// The channel-specific configuration is invalid.
    #[error("invalid channel config: {0}")]
    InvalidConfig(String),

    /// SMTP is not configured; no host has been provided.
    #[error("SMTP is not configured")]
    SmtpNotConfigured,

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

/// Convenience type alias for notification plugin results.
pub type Result<T> = std::result::Result<T, Report<NotificationPluginError>>;
