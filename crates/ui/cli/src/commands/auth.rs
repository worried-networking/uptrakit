use crate::client::ApiClient;
use crate::config::{
    Config, Credentials, load_config, load_credentials, save_config, save_credentials,
};
use crate::error::{CliError, Result};
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use serde::Serialize;

/// Serializable output for `auth status`.
#[derive(Debug, Serialize)]
pub struct AuthStatusOutput {
    pub server: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub user_id: String,
    pub roles: Vec<String>,
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
pub async fn login(server_override: Option<&str>) -> Result<()> {
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
        return Err(report!(CliError::Other("Server URL is required".into())));
    }

    // Build client name for the token
    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let date = chrono_date();
    let client_name = format!("cli-{host}-{date}");

    // Start device authorization flow
    let client = ApiClient::new(&server, None)?;
    let start_body = serde_json::json!({ "client_name": client_name });

    let (status, body) = client
        .request("POST", "/api/v1/auth/device", Some(start_body))
        .await?;

    if status != 200 {
        let msg = body
            .as_str()
            .unwrap_or("Failed to start device authorization")
            .to_string();
        return Err(report!(CliError::Api {
            status,
            message: msg,
        }));
    }

    let device_code = body["device_code"]
        .as_str()
        .ok_or_else(|| report!(CliError::Other("No device_code in response".into())))?
        .to_string();

    let user_code = body["user_code"]
        .as_str()
        .ok_or_else(|| report!(CliError::Other("No user_code in response".into())))?;

    let verification_url = body["verification_url"]
        .as_str()
        .ok_or_else(|| report!(CliError::Other("No verification_url in response".into())))?;

    let interval = body["interval"].as_u64().unwrap_or(5);
    let expires_in = body["expires_in"].as_u64().unwrap_or(600);

    // Display the code and URL
    eprintln!();
    eprintln!("  Open this URL in your browser:");
    eprintln!("  {}", verification_url);
    eprintln!();
    eprintln!("  And enter this code: {}", user_code);
    eprintln!();

    // Try to open the URL in the user's browser
    if let Err(e) = open::that(verification_url) {
        eprintln!("  (Could not open browser automatically: {})", e);
        eprintln!("  Please open the URL above manually.");
        eprintln!();
    }

    eprintln!("  Waiting for authorization...");

    // Poll for completion
    let poll_client = ApiClient::new(&server, None)?;
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(expires_in);

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        if start.elapsed() > timeout {
            return Err(report!(CliError::Other(
                "Device authorization timed out".into()
            )));
        }

        let poll_body = serde_json::json!({ "device_code": device_code });
        let (poll_status, poll_resp) = poll_client
            .request("POST", "/api/v1/auth/device/poll", Some(poll_body))
            .await?;

        if poll_status == 429 {
            // Rate limited, wait extra interval
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            continue;
        }

        if poll_status == 404 {
            return Err(report!(CliError::Other(
                "Device authorization session not found or expired".into(),
            )));
        }

        if poll_status != 200 {
            let msg = poll_resp
                .as_str()
                .unwrap_or("Unexpected error during polling")
                .to_string();
            return Err(report!(CliError::Api {
                status: poll_status,
                message: msg,
            }));
        }

        let flow_status = poll_resp["status"].as_str().unwrap_or("unknown");

        match flow_status {
            "pending" => continue,
            "expired" => {
                return Err(report!(CliError::Other(
                    "Device authorization expired".into()
                )));
            }
            "authorized" => {
                let api_token = poll_resp["token"]
                    .as_str()
                    .ok_or_else(|| report!(CliError::Other("No token in response".into())))?;
                let token_name = poll_resp["token_name"].as_str().unwrap_or(&client_name);

                // Store config and credentials
                save_config(&Config {
                    server: Some(server.clone()),
                })?;
                save_credentials(&Credentials {
                    token: Some(api_token.to_string()),
                })?;

                eprintln!();
                println!("Logged in to {} successfully.", server);
                println!("API token stored locally (name: {}).", token_name);

                return Ok(());
            }
            other => {
                return Err(report!(CliError::Other(format!(
                    "Unexpected device flow status: {other}"
                ))));
            }
        }
    }
}

/// Show current authentication status.
pub async fn status(
    server_override: Option<&str>,
    token_override: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = ApiClient::with_token(&server, &token)?;

    let (status, body) = client.request("GET", "/api/v1/auth/me", None).await?;

    if status != 200 {
        let msg = body
            .as_str()
            .unwrap_or("Failed to get user info")
            .to_string();
        return Err(report!(CliError::Api {
            status,
            message: msg,
        }));
    }

    let first_name = body["first_name"].as_str().unwrap_or("").to_string();
    let last_name = body["last_name"].as_str().unwrap_or("").to_string();
    let email = body["email"].as_str().unwrap_or("").to_string();
    let user_id = body["id"].as_str().unwrap_or("").to_string();
    let roles: Vec<String> = body["roles"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut human = String::new();
    human.push_str(&format!("Server:     {}\n", server));
    human.push_str(&format!("User:       {} {}\n", first_name, last_name));
    human.push_str(&format!("Email:      {}\n", email));
    human.push_str(&format!("User ID:    {}\n", user_id));
    if !roles.is_empty() {
        human.push_str(&format!("Roles:      {}\n", roles.join(", ")));
    }

    let data = AuthStatusOutput {
        server,
        first_name,
        last_name,
        email,
        user_id,
        roles,
    };

    print_output(format, &human, &data)
}

/// Create a new API token.
pub async fn token_create(
    name: &str,
    server_override: Option<&str>,
    token_override: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = ApiClient::with_token(&server, &token)?;

    let create_body = serde_json::json!({ "name": name });
    let (status, body) = client
        .request("POST", "/api/v1/auth/api-tokens", Some(create_body))
        .await?;

    if status != 201 {
        let msg = body
            .as_str()
            .unwrap_or("Failed to create token")
            .to_string();
        return Err(report!(CliError::Api {
            status,
            message: msg,
        }));
    }

    let id = body["id"].as_str().unwrap_or("").to_string();
    let new_token = body["token"].as_str().unwrap_or("").to_string();

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
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = ApiClient::with_token(&server, &token)?;

    let (status, body) = client
        .request("GET", "/api/v1/auth/api-tokens", None)
        .await?;

    if status != 200 {
        let msg = body.as_str().unwrap_or("Failed to list tokens").to_string();
        return Err(report!(CliError::Api {
            status,
            message: msg,
        }));
    }

    let tokens = body["tokens"].as_array();
    let entries: Vec<TokenEntry> = match tokens {
        Some(tokens) => tokens
            .iter()
            .map(|t| {
                let status_str = if t["revoked_at"].is_string() {
                    "revoked"
                } else {
                    "active"
                };
                TokenEntry {
                    id: t["id"].as_str().unwrap_or("").to_string(),
                    name: t["name"].as_str().unwrap_or("").to_string(),
                    created_at: t["created_at"].as_str().unwrap_or("").to_string(),
                    status: status_str.to_string(),
                }
            })
            .collect(),
        None => Vec::new(),
    };

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
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = ApiClient::with_token(&server, &token)?;

    let path = format!("/api/v1/auth/api-tokens/{id}");
    let (status, body) = client.request("DELETE", &path, None).await?;

    if status != 204 {
        let msg = body
            .as_str()
            .unwrap_or("Failed to revoke token")
            .to_string();
        return Err(report!(CliError::Api {
            status,
            message: msg,
        }));
    }

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
    // Simple date string YYYY-MM-DD without pulling in chrono
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Days since epoch
    let days = now / 86400;
    // Approximate date calculation
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days: &[i64] = if is_leap(y) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 1;
    for &md in month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }
    let d = remaining + 1;
    format!("{y:04}-{m:02}-{d:02}")
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
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
            roles: vec!["admin".to_string(), "user".to_string()],
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["server"], "https://example.com");
        assert_eq!(parsed["first_name"], "John");
        assert_eq!(parsed["roles"][0], "admin");
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
