use crate::client::ApiClient;
use crate::config::{load_config, load_credentials};
use crate::error::{CliError, Result};

/// Execute a raw API call and pretty-print the response.
pub async fn execute(
    method: &str,
    path: &str,
    data: Option<&str>,
    server_override: Option<&str>,
    token_override: Option<&str>,
) -> Result<()> {
    let config = load_config()?;
    let creds = load_credentials()?;

    let server = server_override
        .map(|s| s.to_string())
        .or(config.server)
        .ok_or(CliError::NotLoggedIn)?;

    let token = token_override
        .map(|t| t.to_string())
        .or(creds.token)
        .ok_or(CliError::NotLoggedIn)?;

    let client = ApiClient::with_token(&server, &token)?;

    let body = match data {
        Some(json_str) => Some(
            serde_json::from_str(json_str)
                .map_err(|e| CliError::Other(format!("Invalid JSON data: {e}")))?,
        ),
        None => None,
    };

    // Ensure path starts with /
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    let (status, response) = client.request(method, &path, body).await?;

    // Print status
    eprintln!("HTTP {} {}", status, status_text(status));

    // Pretty-print response body
    if !response.is_null() {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string())
        );
    }

    if status >= 400 {
        std::process::exit(1);
    }

    Ok(())
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "",
    }
}
