use crate::client::authenticated_client;
use crate::error::{CliError, Result};
use crate::output::{OutputFormat, print_value};
use rootcause::prelude::*;
use serde::Serialize;

/// Execute a raw API call and print the response in the requested format.
pub async fn execute(
    method: &str,
    path: &str,
    data: Option<&str>,
    server_override: Option<&str>,
    token_override: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server_override, token_override, insecure)?;

    let body = match data {
        Some(json_str) => Some(serde_json::from_str(json_str).context_to()?),
        None => None,
    };

    // Ensure path starts with /
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    let resp = client.raw_request(method, &path, body).await.context_to()?;

    match format {
        OutputFormat::Human => {
            eprintln!("HTTP {} {}", resp.status, status_text(resp.status));
            if !resp.body.is_null() {
                print_value(format, &resp.body)?;
            }
        }
        OutputFormat::Json | OutputFormat::Yaml => {
            let envelope = ApiResponse {
                status: resp.status,
                status_text: status_text(resp.status),
                body: if resp.body.is_null() {
                    None
                } else {
                    Some(resp.body)
                },
            };
            print_value(format, &serde_json::to_value(&envelope).context_to()?)?;
        }
    }

    if resp.status >= 400 {
        bail!(CliError::Api {
            status: resp.status,
            message: format!("HTTP {} {}", resp.status, status_text(resp.status)),
        });
    }

    Ok(())
}

#[derive(Serialize)]
struct ApiResponse {
    status: u16,
    status_text: &'static str,
    body: Option<serde_json::Value>,
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
