use rootcause::prelude::*;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors that can occur when communicating with the Uptrakit API.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("rate limited{}", match .retry_after_seconds {
        Some(secs) => format!(" (retry after {secs}s)"),
        None => String::new(),
    })]
    RateLimited {
        retry_after_seconds: Option<u64>,
    },

    #[error("not found: {0}")]
    NotFound(String),

    #[error("not authenticated")]
    NotAuthenticated,

    #[error("invalid HTTP method: {0}")]
    InvalidMethod(String),
}

pub type Result<T> = std::result::Result<T, Report<ClientError>>;

impl_report_conversion! {
    reqwest::Error  => ClientError::Http,
    serde_json::Error => ClientError::Json,
}
