use crate::client::authenticated_client;
use crate::error::{CliError, Result};
use crate::output::{OutputFormat, print_value};
use rootcause::prelude::*;
use serde::Serialize;
use uptrakit_openapi_client::StatusCode;

/// Parameters for executing a raw API call.
pub struct ExecuteParams<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub data: Option<&'a str>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// Execute a raw API call and print the response in the requested format.
pub async fn execute(params: ExecuteParams<'_>) -> Result<()> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let method = params.method;
    let data = params.data;
    let format = params.format;

    let body = match data {
        Some(json_str) => Some(serde_json::from_str(json_str).context_to()?),
        None => None,
    };

    // Ensure path starts with /
    let path = if params.path.starts_with('/') {
        params.path.to_string()
    } else {
        format!("/{}", params.path)
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
