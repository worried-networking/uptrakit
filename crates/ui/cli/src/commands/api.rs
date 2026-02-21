use crate::client::authenticated_client;
use crate::error::{CliError, Result};
use crate::output::{OutputFormat, print_value};
use rootcause::prelude::*;
use serde::Serialize;
use uptrakit_openapi_client::StatusCode;

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
    let reason = resp.status.canonical_reason().unwrap_or("");

    match format {
        OutputFormat::Human => {
            println!("HTTP {} {reason}", resp.status);
            if !resp.body.is_null() {
                print_value(format, &resp.body)?;
            }
        }
        OutputFormat::Json | OutputFormat::Yaml => {
            let envelope = ApiResponse {
                status: resp.status,
                status_text: reason,
                body: if resp.body.is_null() {
                    None
                } else {
                    Some(resp.body)
                },
            };
            print_value(format, &serde_json::to_value(&envelope).context_to()?)?;
        }
    }

    if resp.status.is_client_error() || resp.status.is_server_error() {
        bail!(CliError::Api {
            status: resp.status,
            message: format!("HTTP {} {reason}", resp.status),
        });
    }

    Ok(())
}

/// Serialize a `StatusCode` as its numeric `u16` value for JSON wire compatibility.
fn serialize_status_code<S: serde::Serializer>(
    status: &StatusCode,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_u16(status.as_u16())
}

#[derive(Serialize)]
struct ApiResponse {
    #[serde(serialize_with = "serialize_status_code")]
    status: StatusCode,
    status_text: &'static str,
    body: Option<serde_json::Value>,
}
