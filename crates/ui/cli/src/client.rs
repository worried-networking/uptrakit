use crate::config::{load_config, load_credentials};
use crate::error::{CliError, Result};
use rootcause::prelude::*;

pub use uptrakit_openapi_client::UptrakitClient;

/// Build an authenticated API client from stored config/credentials or overrides.
pub fn authenticated_client(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
) -> Result<UptrakitClient> {
    let config = load_config()?;
    let creds = load_credentials()?;

    let server = server
        .map(|s| s.to_string())
        .or(config.server)
        .ok_or_else(|| report!(CliError::NotLoggedIn))?;

    let token = token
        .map(|t| t.to_string())
        .or(creds.token)
        .ok_or_else(|| report!(CliError::NotLoggedIn))?;

    UptrakitClient::with_token(&server, &token, insecure).context_to()
}
