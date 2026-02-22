use crate::client::{UptrakitClient, resolve_server_and_token};
use crate::config::{Config, Credentials, load_config, save_config, save_credentials};
use crate::error::{CliError, Result};
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::api_tokens::CreateApiTokenRequest;
use uptrakit_openapi_client::types::device_auth::{DeviceAuthPollRequest, DeviceAuthStartRequest};

/// Serializable output for `auth status`.
#[derive(Debug, Serialize)]
pub struct AuthStatusOutput {
    pub server: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub user_id: Uuid,
    pub permissions: Vec<String>,
}

/// Serializable output for `auth token create`.
#[derive(Debug, Serialize)]
pub struct TokenCreateOutput {
    pub id: Uuid,
    pub token: String,
}

/// Serializable output for `auth token list`.
#[derive(Debug, Serialize)]
pub struct TokenListOutput {
    pub tokens: Vec<TokenEntry>,
}

/// Status of an API token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenStatus {
    Active,
    Revoked,
}

impl std::fmt::Display for TokenStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => f.write_str("active"),
            Self::Revoked => f.write_str("revoked"),
        }
    }
}

/// A single token entry in `auth token list`.
#[derive(Debug, Serialize)]
pub struct TokenEntry {
    pub id: Uuid,
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub status: TokenStatus,
}

/// Serializable output for `auth token revoke`.
#[derive(Debug, Serialize)]
pub struct TokenRevokeOutput {
    pub id: Uuid,
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
    let client = UptrakitClient::new(&server, None, insecure, None).context_to()?;
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

    // Validate URL scheme before opening in browser (prevents malicious schemes
    // like file:// or javascript:// from a compromised server).
    if let Err(e) = validate_url_scheme(&start_resp.verification_url, insecure) {
        eprintln!("  (URL validation failed: {})", e);
        eprintln!("  Please verify and open the URL above manually.");
        eprintln!();
    } else if let Err(e) = open_url(&start_resp.verification_url) {
        eprintln!("  (Could not open browser automatically: {})", e);
        eprintln!("  Please open the URL above manually.");
        eprintln!();
    }

    eprintln!("  Waiting for authorization...");

    // Poll for completion
    let poll_client = UptrakitClient::new(&server, None, insecure, None).context_to()?;
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
                if let uptrakit_openapi_client::ClientError::RateLimited {
                    retry_after_seconds,
                } = e.current_context()
                {
                    let delay = retry_after_seconds.unwrap_or(interval);
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
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
                })
                .await?;
                save_credentials(&Credentials {
                    token: Some(api_token.expose_secret().to_string()),
                })
                .await?;

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
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client = UptrakitClient::new(&server, Some(&token), insecure, request_timeout).context_to()?;

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
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client = UptrakitClient::new(&server, Some(&token), insecure, request_timeout).context_to()?;

    let resp = client
        .create_api_token(&CreateApiTokenRequest {
            name: name.to_string(),
        })
        .await
        .context_to()?;

    let id = resp.id;
    let new_token = resp.token.expose_secret().to_string();

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
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client = UptrakitClient::new(&server, Some(&token), insecure, request_timeout).context_to()?;

    let resp = client.list_api_tokens().await.context_to()?;

    let entries: Vec<TokenEntry> = resp
        .tokens
        .iter()
        .map(|t| {
            let status = if t.revoked_at.is_some() {
                TokenStatus::Revoked
            } else {
                TokenStatus::Active
            };
            TokenEntry {
                id: t.id,
                name: t.name.clone(),
                created_at: t.created_at,
                status,
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
            let created = t.created_at.format(&Rfc3339).unwrap_or_else(|_| t.created_at.to_string());
            human.push_str(&format!(
                "{:<38} {:<30} {:<25} {}\n",
                t.id, t.name, created, t.status,
            ));
        }
    }

    let data = TokenListOutput { tokens: entries };

    print_output(format, &human, &data)
}

/// Revoke an API token.
pub async fn token_revoke(
    id: &Uuid,
    server_override: Option<&str>,
    token_override: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client = UptrakitClient::new(&server, Some(&token), insecure, request_timeout).context_to()?;

    client.revoke_api_token(id).await.context_to()?;

    let human = format!("Token {} revoked.\n", id);
    let data = TokenRevokeOutput {
        id: *id,
        revoked: true,
    };

    print_output(format, &human, &data)
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

/// Validate that a URL uses a safe scheme before opening in a browser.
///
/// Only `https://` URLs are allowed by default. When `allow_http` is true,
/// `http://` URLs are also accepted (for `--insecure` mode).
/// Returns an error for any other scheme (e.g. `file://`, `javascript:`).
fn validate_url_scheme(url: &str, allow_http: bool) -> std::result::Result<(), CliError> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if allow_http && url.starts_with("http://") {
        return Ok(());
    }
    Err(CliError::Other(format!(
        "refusing to open URL with untrusted scheme: {url}"
    )))
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
        let user_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid");
        let output = AuthStatusOutput {
            server: "https://example.com".to_string(),
            first_name: "John".to_string(),
            last_name: "Doe".to_string(),
            email: "john@example.com".to_string(),
            user_id,
            permissions: vec!["view_settings".to_string(), "manage_agents".to_string()],
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["server"], "https://example.com");
        assert_eq!(parsed["first_name"], "John");
        assert_eq!(parsed["user_id"], user_id.to_string());
        assert_eq!(parsed["permissions"][0], "view_settings");
    }

    #[test]
    fn token_create_output_serialization() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").expect("uuid");
        let output = TokenCreateOutput {
            id,
            token: "secret-value".to_string(),
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["id"], id.to_string());
        assert_eq!(parsed["token"], "secret-value");
    }

    #[test]
    fn token_list_output_serialization() {
        use time::macros::datetime;
        let id1 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").expect("uuid");
        let id2 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440002").expect("uuid");
        let output = TokenListOutput {
            tokens: vec![
                TokenEntry {
                    id: id1,
                    name: "my-token".to_string(),
                    created_at: datetime!(2025-01-01 00:00:00 UTC),
                    status: TokenStatus::Active,
                },
                TokenEntry {
                    id: id2,
                    name: "old-token".to_string(),
                    created_at: datetime!(2024-06-15 12:00:00 UTC),
                    status: TokenStatus::Revoked,
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
    fn token_status_display() {
        assert_eq!(TokenStatus::Active.to_string(), "active");
        assert_eq!(TokenStatus::Revoked.to_string(), "revoked");
    }

    #[test]
    fn token_status_serialization() {
        let json = serde_json::to_string(&TokenStatus::Active).expect("serialize");
        assert_eq!(json, r#""active""#);
        let json = serde_json::to_string(&TokenStatus::Revoked).expect("serialize");
        assert_eq!(json, r#""revoked""#);
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
    fn validate_url_scheme_https_allowed() {
        assert!(validate_url_scheme("https://example.com/auth", false).is_ok());
        assert!(validate_url_scheme("https://example.com/auth", true).is_ok());
    }

    #[test]
    fn validate_url_scheme_http_requires_insecure() {
        assert!(validate_url_scheme("http://example.com/auth", false).is_err());
        assert!(validate_url_scheme("http://example.com/auth", true).is_ok());
    }

    #[test]
    fn validate_url_scheme_rejects_dangerous_schemes() {
        assert!(validate_url_scheme("file:///etc/passwd", false).is_err());
        assert!(validate_url_scheme("file:///etc/passwd", true).is_err());
        assert!(validate_url_scheme("javascript:alert(1)", false).is_err());
        assert!(validate_url_scheme("ftp://example.com", false).is_err());
        assert!(validate_url_scheme("data:text/html,<h1>hi</h1>", false).is_err());
    }

    #[test]
    fn validate_url_scheme_rejects_empty_and_relative() {
        assert!(validate_url_scheme("", false).is_err());
        assert!(validate_url_scheme("/path/only", false).is_err());
        assert!(validate_url_scheme("example.com", false).is_err());
    }

    #[test]
    fn token_revoke_output_serialization() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").expect("uuid");
        let output = TokenRevokeOutput { id, revoked: true };
        let json = serde_json::to_string(&output).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["id"], id.to_string());
        assert_eq!(parsed["revoked"], true);
    }
}
