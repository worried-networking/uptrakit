#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Not logged in. Run `uptrakit-cli auth login` first.")]
    NotLoggedIn,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CliError>;
