use crate::client::{UptrakitClient, resolve_server_and_token};
use crate::commands::CliContext;
use crate::config::load_config;
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::api_tokens::CreateApiTokenRequest;

#[derive(Debug, Subcommand)]
pub enum AuthCommands {
    /// Login to the server via browser authorization
    Login,
    /// Show current authentication status
    Status,
    /// API token management
    Token {
        #[command(subcommand)]
        command: TokenCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum TokenCommands {
    /// Create a new API token
    Create {
        /// Token name
        #[arg(long)]
        name: String,
    },
    /// List API tokens
    List,
    /// Revoke an API token
    Revoke {
        /// Token ID to revoke
        id: Uuid,
    },
}

pub async fn dispatch(command: AuthCommands, ctx: &CliContext) -> Result<()> {
    match command {
        AuthCommands::Login => {
            login(ctx.server.as_deref(), ctx.insecure).await?;
        }
        AuthCommands::Status => {
            let resp = status(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        AuthCommands::Token { command } => match command {
            TokenCommands::Create { name } => {
                let resp = token_create(
                    &name,
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            TokenCommands::List => {
                let resp = token_list(
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            TokenCommands::Revoke { id } => {
                let resp = token_revoke(
                    &id,
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
        },
    }
    Ok(())
}

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

impl HumanOutput for AuthStatusOutput {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Server:      {}\n", self.server));
        out.push_str(&format!(
            "User:        {} {}\n",
            self.first_name, self.last_name
        ));
        out.push_str(&format!("Email:       {}\n", self.email));
        out.push_str(&format!("User ID:     {}\n", self.user_id));
        if !self.permissions.is_empty() {
            out.push_str(&format!("Permissions: {}\n", self.permissions.join(", ")));
        }
        out
    }
}

/// Serializable output for `auth token create`.
#[derive(Debug, Serialize)]
pub struct TokenCreateOutput {
    pub id: Uuid,
    pub token: String,
}

impl HumanOutput for TokenCreateOutput {
    fn to_human_string(&self) -> String {
        let mut out = "Token created:\n".to_string();
        out.push_str(&format!("  ID:    {}\n", self.id));
        out.push_str(&format!("  Token: {}\n", self.token));
        out.push('\n');
        out.push_str("Store this token securely - it will not be shown again.\n");
        out
    }
}

/// Serializable output for `auth token list`.
#[derive(Debug, Serialize)]
pub struct TokenListOutput {
    pub tokens: Vec<TokenEntry>,
}

impl HumanOutput for TokenListOutput {
    fn to_human_string(&self) -> String {
        if self.tokens.is_empty() {
            return "No API tokens found.\n".to_string();
        }
        let mut out = format!("{:<38} {:<30} {:<25} STATUS\n", "ID", "NAME", "CREATED");
        for t in &self.tokens {
            let created = t
                .created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| t.created_at.to_string());
            out.push_str(&format!(
                "{:<38} {:<30} {:<25} {}\n",
                t.id, t.name, created, t.status,
            ));
        }
        out
    }
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

impl HumanOutput for TokenRevokeOutput {
    fn to_human_string(&self) -> String {
        format!("Token {} revoked.\n", self.id)
    }
}

/// Interactive login flow using device authorization (RFC 8628).
///
/// 1. Prompt for server URL (if not stored/provided)
/// 2. POST /api/v1/oauth/device_authorization — get device_code, user_code, verification_uri
/// 3. Open verification_uri_complete in browser; print user_code + plain verification_uri
/// 4. Poll /api/v1/oauth/token with RFC error-code parsing until authorized/denied/expired
/// 5. Store server URL + API token locally
///
/// Full implementation lands in Plan 2 (openapi-client + CLI rewrite).
pub async fn login(server_override: Option<&str>, _insecure: bool) -> Result<()> {
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

    bail!(CliError::Other(
        "Device authorization login requires the updated CLI client (Plan 2 not yet landed)."
            .into()
    ));
}

/// Show current authentication status.
pub async fn status(
    server_override: Option<&str>,
    token_override: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AuthStatusOutput> {
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client =
        UptrakitClient::new(&server, Some(&token), insecure, request_timeout).context_to()?;

    let user = client.me().await.context_to()?;

    let permissions: Vec<String> = user.permissions.iter().map(|p| p.to_string()).collect();

    Ok(AuthStatusOutput {
        server,
        first_name: user.first_name,
        last_name: user.last_name,
        email: user.email,
        user_id: user.id,
        permissions,
    })
}

/// Create a new API token.
pub async fn token_create(
    name: &str,
    server_override: Option<&str>,
    token_override: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<TokenCreateOutput> {
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client =
        UptrakitClient::new(&server, Some(&token), insecure, request_timeout).context_to()?;

    let resp = client
        .create_api_token(&CreateApiTokenRequest {
            name: name.to_string(),
        })
        .await
        .context_to()?;

    let id = resp.id;
    let new_token = resp.token.expose_secret().to_string();

    Ok(TokenCreateOutput {
        id,
        token: new_token,
    })
}

/// List API tokens.
pub async fn token_list(
    server_override: Option<&str>,
    token_override: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<TokenListOutput> {
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client =
        UptrakitClient::new(&server, Some(&token), insecure, request_timeout).context_to()?;

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

    Ok(TokenListOutput { tokens: entries })
}

/// Revoke an API token.
pub async fn token_revoke(
    id: &Uuid,
    server_override: Option<&str>,
    token_override: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<TokenRevokeOutput> {
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client =
        UptrakitClient::new(&server, Some(&token), insecure, request_timeout).context_to()?;

    client.revoke_api_token(id).await.context_to()?;

    Ok(TokenRevokeOutput {
        id: *id,
        revoked: true,
    })
}

fn prompt(msg: &str) -> Result<String> {
    eprint!("{}", msg);
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).context_to()?;
    Ok(input.trim().to_string())
}

#[cfg(test)]
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
#[cfg(test)]
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
#[expect(dead_code, reason = "Plan 2 login rewrite will restore caller")]
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
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok/is_err()) is idiomatic in tests"
    )]

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
    fn auth_status_human_output() {
        let output = AuthStatusOutput {
            server: "https://example.com".to_string(),
            first_name: "John".to_string(),
            last_name: "Doe".to_string(),
            email: "john@example.com".to_string(),
            user_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid"),
            permissions: vec!["view_settings".to_string()],
        };
        let s = output.to_human_string();
        assert!(s.contains("https://example.com"), "server missing");
        assert!(s.contains("john@example.com"), "email missing");
        assert!(s.contains("view_settings"), "permissions missing");
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
    fn token_create_human_output() {
        let output = TokenCreateOutput {
            id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").expect("uuid"),
            token: "secret-value".to_string(),
        };
        let s = output.to_human_string();
        assert!(s.contains("Token created"), "header missing");
        assert!(s.contains("secret-value"), "token missing");
        assert!(s.contains("securely"), "security note missing");
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
    fn token_list_empty_human_output() {
        let output = TokenListOutput { tokens: vec![] };
        assert!(output.to_human_string().contains("No API tokens"));
    }

    #[test]
    fn token_list_human_output_has_header() {
        use time::macros::datetime;
        let output = TokenListOutput {
            tokens: vec![TokenEntry {
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").expect("uuid"),
                name: "my-token".to_string(),
                created_at: datetime!(2025-01-01 00:00:00 UTC),
                status: TokenStatus::Active,
            }],
        };
        let s = output.to_human_string();
        assert!(s.contains("my-token"), "name missing");
        assert!(s.contains("active"), "status missing");
    }

    #[test]
    fn token_revoke_human_output() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").expect("uuid");
        let output = TokenRevokeOutput { id, revoked: true };
        let s = output.to_human_string();
        assert!(s.contains("revoked"), "revoked word missing");
        assert!(
            s.contains("550e8400-e29b-41d4-a716-446655440001"),
            "id missing"
        );
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
