use crate::client::UptrakitClient;
use crate::config::{
    Config, Credentials, load_config, load_credentials, save_config, save_credentials,
};
use crate::error::{CliError, Result};
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use serde::Serialize;
use uptrakit_openapi_client::types::api_tokens::CreateApiTokenRequest;
use uptrakit_openapi_client::types::device_auth::{DeviceAuthPollRequest, DeviceAuthStartRequest};

/// Serializable output for `auth status`.
#[derive(Debug, Serialize)]
pub struct AuthStatusOutput {
    pub server: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub user_id: String,
    pub permissions: Vec<String>,
}

/// Serializable output for `auth token create`.
#[derive(Debug, Serialize)]
pub struct TokenCreateOutput {
    pub id: String,
    pub token: String,
}

/// Serializable output for `auth token list`.
#[derive(Debug, Serialize)]
pub struct TokenListOutput {
    pub tokens: Vec<TokenEntry>,
}

/// A single token entry in `auth token list`.
#[derive(Debug, Serialize)]
pub struct TokenEntry {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub status: String,
}

/// Serializable output for `auth token revoke`.
#[derive(Debug, Serialize)]
pub struct TokenRevokeOutput {
    pub id: String,
    pub revoked: bool,
}

/// Interactive login flow using device authorization.
///
/// 1. Prompt for server URL (if not stored/provided)
/// 2. POST /api/v1/auth/device -> get device_code, user_code, verification_url
/// 3. Open verification URL in user's browser
/// 4. Poll /api/v1/auth/device/poll until authorized/expired
/// 5. Store server URL + API token locally
pub async fn login(server_override: Option<&str>, insecure: bool) -> Result<()> {
    // Determine server URL
    let config = load_config()?;
    let server = if let Some(s) = server_override {
        s.to_string()
    } else if let Some(s) = &config.server {
        let input = prompt(&format!("Server URL [{}]: ", s))?;
        if input.is_empty() { s.clone() } else { input }
    } else {
        prompt("Server URL: ")?
    };

    if server.is_empty() {
        bail!(CliError::Other("Server URL is required".into()));
    }

    // Build client name for the token
    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let date = chrono_date();
    let client_name = format!("cli-{host}-{date}");

    // Start device authorization flow
    let client = UptrakitClient::new(&server, None, insecure).context_to()?;
    let start_resp = client
        .device_auth_start(&DeviceAuthStartRequest {
            client_name: Some(client_name.clone()),
        })
        .await
        .context_to()?;

    // Display the code and URL
    eprintln!();
    eprintln!("  Open this URL in your browser:");
    eprintln!("  {}", start_resp.verification_url);
    eprintln!();
    eprintln!("  And enter this code: {}", start_resp.user_code);
    eprintln!();

    // Try to open the URL in the user's browser
    if let Err(e) = open_url(&start_resp.verification_url) {
        eprintln!("  (Could not open browser automatically: {})", e);
        eprintln!("  Please open the URL above manually.");
        eprintln!();
    }

    eprintln!("  Waiting for authorization...");

    // Poll for completion
    let poll_client = UptrakitClient::new(&server, None, insecure).context_to()?;
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(start_resp.expires_in);
    let interval = start_resp.interval;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        if start.elapsed() > timeout {
            bail!(CliError::Other("Device authorization timed out".into()));
        }

        let poll_result = poll_client
            .device_auth_poll(&DeviceAuthPollRequest {
                device_code: start_resp.device_code.clone(),
            })
            .await;

        let poll_resp = match poll_result {
            Ok(resp) => resp,
            Err(e) => {
                if matches!(
                    e.current_context(),
                    uptrakit_openapi_client::ClientError::RateLimited
                ) {
                    // Rate limited, wait extra interval
                    tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                    continue;
                }
                if matches!(
                    e.current_context(),
                    uptrakit_openapi_client::ClientError::NotFound(_)
                ) {
                    bail!(CliError::Other(
                        "Device authorization session not found or expired".into(),
                    ));
                }
                return Err(e.context_to());
            }
        };

        let flow_status = poll_resp.status;

        match flow_status {
            uptrakit_openapi_client::DeviceAuthStatus::Pending => continue,
            uptrakit_openapi_client::DeviceAuthStatus::Expired => {
                bail!(CliError::Other("Device authorization expired".into()));
            }
            uptrakit_openapi_client::DeviceAuthStatus::Authorized => {
                let api_token = poll_resp
                    .token
                    .ok_or_else(|| report!(CliError::Other("No token in response".into())))?;
                let token_name = poll_resp.token_name.as_deref().unwrap_or(&client_name);

                // Store config and credentials
                save_config(&Config {
                    server: Some(server.clone()),
                })?;
                save_credentials(&Credentials {
                    token: Some(api_token),
                })?;

                eprintln!();
                println!("Logged in to {} successfully.", server);
                println!("API token stored locally (name: {}).", token_name);

                return Ok(());
            }
        }
    }
}

/// Show current authentication status.
pub async fn status(
    server_override: Option<&str>,
    token_override: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = UptrakitClient::with_token(&server, &token, insecure).context_to()?;

    let user = client.me().await.context_to()?;

    let permissions: Vec<String> = user.permissions.iter().map(|p| p.to_string()).collect();

    let mut human = String::new();
    human.push_str(&format!("Server:      {}\n", server));
    human.push_str(&format!(
        "User:        {} {}\n",
        user.first_name, user.last_name
    ));
    human.push_str(&format!("Email:       {}\n", user.email));
    human.push_str(&format!("User ID:     {}\n", user.id));
    if !permissions.is_empty() {
        human.push_str(&format!("Permissions: {}\n", permissions.join(", ")));
    }

    let data = AuthStatusOutput {
        server,
        first_name: user.first_name,
        last_name: user.last_name,
        email: user.email,
        user_id: user.id,
        permissions,
    };

    print_output(format, &human, &data)
}

/// Create a new API token.
pub async fn token_create(
    name: &str,
    server_override: Option<&str>,
    token_override: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = UptrakitClient::with_token(&server, &token, insecure).context_to()?;

    let resp = client
        .create_api_token(&CreateApiTokenRequest {
            name: name.to_string(),
        })
        .await
        .context_to()?;

    let id = resp.id;
    let new_token = resp.token;

    let mut human = String::new();
    human.push_str("Token created:\n");
    human.push_str(&format!("  ID:    {}\n", id));
    human.push_str(&format!("  Token: {}\n", new_token));
    human.push('\n');
    human.push_str("Store this token securely - it will not be shown again.\n");

    let data = TokenCreateOutput {
        id,
        token: new_token,
    };

    print_output(format, &human, &data)
}

/// List API tokens.
pub async fn token_list(
    server_override: Option<&str>,
    token_override: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = UptrakitClient::with_token(&server, &token, insecure).context_to()?;

    let resp = client.list_api_tokens().await.context_to()?;

    let entries: Vec<TokenEntry> = resp
        .tokens
        .iter()
        .map(|t| {
            let status_str = if t.revoked_at.is_some() {
                "revoked"
            } else {
                "active"
            };
            TokenEntry {
                id: t.id.clone(),
                name: t.name.clone(),
                created_at: t.created_at.clone(),
                status: status_str.to_string(),
            }
        })
        .collect();

    let mut human = String::new();
    if entries.is_empty() {
        human.push_str("No API tokens found.\n");
    } else {
        human.push_str(&format!(
            "{:<38} {:<30} {:<25} STATUS\n",
            "ID", "NAME", "CREATED"
        ));
        for t in &entries {
            human.push_str(&format!(
                "{:<38} {:<30} {:<25} {}\n",
                t.id, t.name, t.created_at, t.status,
            ));
        }
    }

    let data = TokenListOutput { tokens: entries };

    print_output(format, &human, &data)
}

/// Revoke an API token.
pub async fn token_revoke(
    id: &str,
    server_override: Option<&str>,
    token_override: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = UptrakitClient::with_token(&server, &token, insecure).context_to()?;

    client.revoke_api_token(id).await.context_to()?;

    let human = format!("Token {} revoked.\n", id);
    let data = TokenRevokeOutput {
        id: id.to_string(),
        revoked: true,
    };

    print_output(format, &human, &data)
}

/// Resolve server and token from overrides or stored config.
fn resolve_auth(
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

fn prompt(msg: &str) -> Result<String> {
    eprint!("{}", msg);
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).context_to()?;
    Ok(input.trim().to_string())
}

fn chrono_date() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month() as u8,
        now.day()
    )
}

/// Open a URL in the user's default browser.
fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_status_output_serialization() {
        let output = AuthStatusOutput {
            server: "https://example.com".to_string(),
            first_name: "John".to_string(),
            last_name: "Doe".to_string(),
            email: "john@example.com".to_string(),
            user_id: "abc-123".to_string(),
            permissions: vec!["view_settings".to_string(), "manage_agents".to_string()],
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["server"], "https://example.com");
        assert_eq!(parsed["first_name"], "John");
        assert_eq!(parsed["permissions"][0], "view_settings");
    }

    #[test]
    fn token_create_output_serialization() {
        let output = TokenCreateOutput {
            id: "tok-1".to_string(),
            token: "secret-value".to_string(),
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["id"], "tok-1");
        assert_eq!(parsed["token"], "secret-value");
    }

    #[test]
    fn token_list_output_serialization() {
        let output = TokenListOutput {
            tokens: vec![
                TokenEntry {
                    id: "tok-1".to_string(),
                    name: "my-token".to_string(),
                    created_at: "2025-01-01T00:00:00Z".to_string(),
                    status: "active".to_string(),
                },
                TokenEntry {
                    id: "tok-2".to_string(),
                    name: "old-token".to_string(),
                    created_at: "2024-06-15T12:00:00Z".to_string(),
                    status: "revoked".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["tokens"].as_array().expect("array").len(), 2);
        assert_eq!(parsed["tokens"][0]["status"], "active");
        assert_eq!(parsed["tokens"][1]["status"], "revoked");
    }

    #[test]
    fn chrono_date_format_is_yyyy_mm_dd() {
        let date = chrono_date();
        // Must match YYYY-MM-DD pattern
        assert_eq!(date.len(), 10, "date should be 10 characters: {date}");
        let parts: Vec<&str> = date.split('-').collect();
        assert_eq!(parts.len(), 3, "date should have 3 dash-separated parts");
        assert_eq!(parts[0].len(), 4, "year should be 4 digits");
        assert_eq!(parts[1].len(), 2, "month should be 2 digits");
        assert_eq!(parts[2].len(), 2, "day should be 2 digits");
        // All parts should be numeric
        assert!(parts[0].chars().all(|c| c.is_ascii_digit()));
        assert!(parts[1].chars().all(|c| c.is_ascii_digit()));
        assert!(parts[2].chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn token_revoke_output_serialization() {
        let output = TokenRevokeOutput {
            id: "tok-1".to_string(),
            revoked: true,
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["id"], "tok-1");
        assert_eq!(parsed["revoked"], true);
    }
}
