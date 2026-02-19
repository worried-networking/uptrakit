use rootcause::prelude::*;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] uptrakit_openapi_client::ReqwestError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("API error ({status}): {message}")]
    Api {
        status: uptrakit_openapi_client::StatusCode,
        message: String,
    },

    #[error("Not logged in. Run `uptrakit auth login` first.")]
    NotLoggedIn,

    #[error("directory error: {0}")]
    Directory(#[from] uptrakit_directories::DirectoryError),

    #[error("YAML serialization error: {0}")]
    Yaml(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Report<CliError>>;

impl_report_conversion! {
    uptrakit_openapi_client::ReqwestError => CliError::Http,
    std::io::Error                        => CliError::Io,
    serde_json::Error                     => CliError::Json,
    uptrakit_directories::DirectoryError  => CliError::Directory,
}

impl_report_conversion!(serde_yaml_ng::Error => CliError, |e| CliError::Yaml(e.to_string()));
impl_report_conversion!(uptrakit_openapi_client::types::mqtt_transport::ParseMqttTransportError => CliError, |e| CliError::Other(e.to_string()));

impl_report_conversion!(uptrakit_openapi_client::ClientError => CliError, |e| {
    match e {
        uptrakit_openapi_client::ClientError::Http(inner) => CliError::Http(inner),
        uptrakit_openapi_client::ClientError::Json(inner) => CliError::Json(inner),
        uptrakit_openapi_client::ClientError::Api { status, message } => CliError::Api { status, message },
        uptrakit_openapi_client::ClientError::RateLimited { retry_after_seconds } => CliError::Api {
            status: uptrakit_openapi_client::StatusCode::TOO_MANY_REQUESTS,
            message: match retry_after_seconds {
                Some(secs) => format!("Rate limited (retry after {secs}s)"),
                None => "Rate limited".to_string(),
            },
        },
        uptrakit_openapi_client::ClientError::NotFound(msg) => CliError::Api { status: uptrakit_openapi_client::StatusCode::NOT_FOUND, message: msg },
        uptrakit_openapi_client::ClientError::NotAuthenticated => CliError::NotLoggedIn,
        uptrakit_openapi_client::ClientError::InvalidMethod(msg) => CliError::Other(format!("Invalid HTTP method: {msg}")),
    }
});
