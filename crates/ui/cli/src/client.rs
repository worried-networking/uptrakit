use crate::config::{load_config, load_credentials};
use crate::error::{CliError, Result};
use rootcause::prelude::*;

pub use uptrakit_openapi_client::UptrakitClient;

/// Resolve server URL and API token from overrides or stored config/credentials.
///
/// Priority: explicit override > stored config/credentials. Returns
/// `CliError::NotLoggedIn` if either value is missing.
pub fn resolve_server_and_token(
    server_override: Option<&str>,
    token_override: Option<&str>,
) -> Result<(String, String)> {
    let config = load_config()?;
    let creds = load_credentials()?;

    let server = server_override
        .map(|s| s.to_string())
        .or(config.server)
        .ok_or_else(|| report!(CliError::NotLoggedIn))?;

    let token = token_override
        .map(|t| t.to_string())
        .or(creds.token)
        .ok_or_else(|| report!(CliError::NotLoggedIn))?;

    Ok((server, token))
}

/// Build an authenticated API client from stored config/credentials or overrides.
pub fn authenticated_client(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
) -> Result<UptrakitClient> {
    let (server, token) = resolve_server_and_token(server, token)?;
    UptrakitClient::with_token(&server, &token, insecure).context_to()
}
