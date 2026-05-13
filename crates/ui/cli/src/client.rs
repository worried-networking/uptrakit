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
///
/// Loads `ca_pem` from the stored [`Config`] so that connections to controllers
/// whose TLS certificate was pinned with `uptrakit auth ca trust` succeed.
/// When the connection fails and a custom CA is configured, a recovery hint is
/// attached to the error pointing users toward `uptrakit auth ca trust` /
/// `uptrakit auth ca forget`.
pub fn authenticated_client(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<UptrakitClient> {
    let config = load_config()?;
    let ca_pem = config.ca_pem.clone();
    let (server, token) = resolve_server_and_token(server, token)?;

    UptrakitClient::new(
        &server,
        Some(&token),
        insecure,
        ca_pem.as_deref(),
        request_timeout,
    )
    .map_err(|e| {
        if ca_pem.is_some() {
            e.attach(
                "connection failed with a pinned CA; if the controller CA has rotated, run \
                     'uptrakit auth ca trust' to re-establish trust; if the controller now uses \
                     a public CA, run 'uptrakit auth ca forget' to return to system roots",
            )
        } else {
            e
        }
    })
    .context_to()
}
