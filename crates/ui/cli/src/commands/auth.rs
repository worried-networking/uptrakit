// TODO: cover slow_down / access_denied / verification_uri_complete flows via mock server once the harness lands
use crate::client::{UptrakitClient, resolve_server_and_token};
use crate::commands::CliContext;
use crate::config::{Config, Credentials, load_config, save_config, save_credentials};
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::ClientError;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::api_tokens::CreateApiTokenRequest;
use uptrakit_openapi_client::types::oauth::{
    DeviceAuthorizationRequest, DeviceAuthorizationResponse, OAuthErrorCode, OAuthTokenRequest,
};

use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use sha2::{Digest, Sha256};

const CLI_CLIENT_ID: &str = "uptrakit-cli";
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Parse a `--tofu=<value>` fingerprint string into normalized 64-char lowercase hex.
///
/// Accepts plain 64-char lowercase hex or a `sha256:` prefix.
///
/// # Errors
///
/// Returns an error if the algorithm prefix is not `sha256`, the hex part is
/// not exactly 64 characters, or it contains non-lowercase-hex characters.
pub fn parse_fingerprint(s: &str) -> Result<String> {
    let hex_part = if let Some(rest) = s.strip_prefix("sha256:") {
        rest
    } else if let Some((prefix, _rest)) = s.split_once(':') {
        bail!(CliError::Other(format!(
            "unsupported fingerprint algorithm '{prefix}'; supported: sha256"
        )));
    } else {
        s
    };

    if hex_part.len() != 64 || !hex_part.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        bail!(CliError::Other(
            "invalid fingerprint: expected 64 lowercase hex characters".into()
        ));
    }

    Ok(hex_part.to_string())
}

/// Decode a PEM-encoded certificate and return its SHA-256 fingerprint as 64-char lowercase hex.
fn pem_fingerprint(pem: &str) -> Result<String> {
    let der = CertificateDer::from_pem_slice(pem.as_bytes()).map_err(|e| {
        report!(CliError::Other(format!(
            "failed to parse CA certificate PEM: {e:?}"
        )))
    })?;
    let mut h = Sha256::new();
    h.update(der.as_ref());
    Ok(uptrakit_shared_types::hex::encode(h.finalize()))
}

/// Fetch the controller CA over an intentionally-insecure bootstrap client
/// and return `(pem, sha256_fingerprint)`.
///
/// SsrfSafeResolver intentionally omitted: CLI tool where operator IS the
/// user — they either typed the server URL directly or explicitly
/// confirmed a LAN-discovered one before any request is sent. Restricting
/// private-range IPs breaks legitimate self-hosted setups. The workspace
/// SSRF rule targets server-side paths processing user-submitted URLs;
/// the documented operator-context exception lives in
/// docs/security/secure-development.md (SSRF section).
async fn fetch_ca(server: &str) -> Result<(String, String)> {
    let bootstrap_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .tls_danger_accept_invalid_certs(true)
        .build()
        .context_to()?;

    let ca_url = format!("{}/api/v1/pki/ca.crt", server.trim_end_matches('/'));
    let fetched_pem = bootstrap_client
        .get(&ca_url)
        .send()
        .await
        .context_to()?
        .error_for_status()
        .context_to()?
        .text()
        .await
        .context_to()?;

    let fetched_fp = pem_fingerprint(&fetched_pem)?;
    Ok((fetched_pem, fetched_fp))
}

/// Fetch the controller CA, optionally verify its fingerprint, prompt for
/// interactive confirmation, and persist the PEM to config.
///
/// Used by both `auth login --tofu` and `auth ca trust`.
/// `allow_rotation` controls whether a stored-CA mismatch is an error (login)
/// or a warning (ca trust).
///
/// Returns `()` on success; the persisted PEM is available via `config.ca_pem`
/// after the call. (The spec draft showed `-> Result<String>` but all call sites
/// read `config.ca_pem` directly, so returning `()` avoids a `must_use` lint.)
///
/// # Errors
///
/// Returns an error if the CA cannot be fetched, the PEM cannot be parsed,
/// the fingerprint does not match, the user declines in interactive mode, or
/// no fingerprint is provided in non-interactive mode.
pub async fn establish_ca_trust(
    server: &str,
    fingerprint_hint: Option<&str>,
    allow_rotation: bool,
    config: &mut Config,
) -> Result<()> {
    use std::io::IsTerminal as _;

    let (fetched_pem, fetched_fp) = fetch_ca(server).await?;

    if let Some(stored_pem) = &config.ca_pem {
        let stored_fp = pem_fingerprint(stored_pem)?;

        if stored_fp != fetched_fp {
            if !allow_rotation {
                bail!(CliError::Other(format!(
                    "Controller CA has changed (stored: {stored_fp}, fetched: {fetched_fp}). \
                     Run 'uptrakit auth ca trust --tofu={fetched_fp}' to update."
                )));
            }
            eprintln!(
                "Warning: CA fingerprint has changed (stored: {stored_fp}). \
                 Proceeding will update stored trust."
            );
        }
    }

    if let Some(expected) = fingerprint_hint {
        if fetched_fp != expected {
            bail!(CliError::Other(format!(
                "CA fingerprint mismatch: expected {expected}, got {fetched_fp}"
            )));
        }
    } else if !std::io::stdin().is_terminal() {
        bail!(CliError::Other(
            "--tofu requires interactive confirmation when no fingerprint is provided; \
             use --tofu=<fingerprint> for non-interactive use"
                .into()
        ));
    } else {
        let rotation_note = if config.ca_pem.is_some() {
            "\nWARNING: This will REPLACE the currently stored CA trust anchor.\n"
        } else {
            ""
        };
        eprintln!(
            "Controller CA fingerprint: {fetched_fp}\n{rotation_note}\
             WARNING: This cannot detect a man-in-the-middle attack. To verify securely,\n\
             obtain the fingerprint from the Dashboard (Global Settings) before running\n\
             this command and compare it to the value above. Use --tofu=<fingerprint> to\n\
             confirm without this prompt."
        );
        let answer = prompt("Trust this CA? [y/N]: ")?;
        if !matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") {
            bail!(CliError::Other(
                "CA trust establishment aborted by user".into()
            ));
        }
    }

    config.ca_pem = Some(fetched_pem);
    save_config(config).await?;
    eprintln!("Controller CA trusted and stored. Future connections will use the pinned CA.");

    Ok(())
}

#[non_exhaustive]
#[derive(Debug, Subcommand)]
pub enum AuthCommands {
    /// Login to the server via browser authorization
    Login {
        /// Trust the controller's CA on first use.
        /// Bare --tofu prompts interactively; --tofu=<FINGERPRINT> pins without prompting.
        #[arg(
            long,
            num_args = 0..=1,
            require_equals = true,
            value_name = "FINGERPRINT",
            default_missing_value = ""
        )]
        tofu: Option<String>,
    },
    /// Show current authentication status
    Status,
    /// API token management
    Token {
        #[command(subcommand)]
        command: TokenCommands,
    },
    /// CA trust management
    Ca {
        #[command(subcommand)]
        command: CaCommands,
    },
}

#[non_exhaustive]
#[derive(Debug, Subcommand)]
pub enum CaCommands {
    /// Establish or update stored CA trust
    Trust {
        /// Trust the controller's CA. Bare --tofu prompts interactively; --tofu=<FINGERPRINT> is non-interactive.
        #[arg(
            long,
            num_args = 0..=1,
            require_equals = true,
            value_name = "FINGERPRINT",
            default_missing_value = ""
        )]
        tofu: Option<String>,
    },
    /// Show stored CA trust status
    Status,
    /// Remove stored CA trust (revert to system roots)
    Forget,
}

#[non_exhaustive]
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
        AuthCommands::Login { tofu } => {
            if ctx.insecure && tofu.is_some() {
                bail!(CliError::Other(
                    "--insecure and --tofu are mutually exclusive".into()
                ));
            }
            login(ctx.server.as_deref(), ctx.insecure, tofu).await?;
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
        AuthCommands::Ca { command } => match command {
            CaCommands::Trust { tofu } => {
                ca_trust(ctx.server.as_deref(), tofu).await?;
            }
            CaCommands::Status => {
                ca_status()?;
            }
            CaCommands::Forget => {
                ca_forget().await?;
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
    pub ca_fingerprint: Option<String>,
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
        match &self.ca_fingerprint {
            Some(fp) => out.push_str(&format!("CA trust:    {fp}\n")),
            None => out.push_str("CA trust:    system roots\n"),
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
/// 2. Optionally establish CA trust via TOFU before OAuth flow
/// 3. POST /api/v1/oauth/device_authorization — get device_code, user_code, verification_uri
/// 4. Open verification_uri_complete in browser; print user_code + plain verification_uri
/// 5. Poll /api/v1/oauth/token with RFC error-code parsing until authorized/denied/expired
/// 6. Store server URL + API token locally
pub async fn login(
    server_override: Option<&str>,
    insecure: bool,
    tofu: Option<String>,
) -> Result<()> {
    let mut config = load_config()?;
    use std::io::IsTerminal as _;

    let mut discovered: Option<uptrakit_zeroconf::DiscoveredController> = None;
    let server =
        match crate::discovery::resolve_server_source(server_override, config.server.as_deref()) {
            crate::discovery::ServerSource::Explicit(server) => server,
            crate::discovery::ServerSource::PromptWithDefault(stored) => {
                let input = prompt(&format!("Server URL [{}]: ", stored))?;
                if input.is_empty() { stored } else { input }
            }
            crate::discovery::ServerSource::Discover => {
                if !std::io::stdin().is_terminal() {
                    bail!(CliError::Other(
                        "no server configured; pass --server <url> (and --tofu=<fingerprint> \
                     for non-interactive CA pinning) — zeroconf discovery requires an \
                     interactive terminal"
                            .into()
                    ));
                }
                match crate::discovery::discover_server_interactive().await? {
                    Some(controller) => {
                        let url = controller.url.clone();
                        discovered = Some(controller);
                        url
                    }
                    None => {
                        // Hint only where its advice is actionable: --tofu is rejected
                        // alongside --insecure at dispatch, and with --tofu the pin
                        // already applies.
                        if tofu.is_none() && !insecure {
                            eprintln!(
                                "Tip: manual entry uses system trust roots; pass --tofu to pin a \
                             self-hosted controller's CA."
                            );
                        }
                        prompt("Server URL: ")?
                    }
                }
            }
        };

    if server.is_empty() {
        bail!(CliError::Other("server URL is required".into()));
    }

    // TOFU: establish CA trust before OAuth flow.
    // None=no TOFU, ""=interactive prompt, "fp"=non-interactive fingerprint.
    // A discovery-accepted server implies TOFU (interactive ceremony) unless
    // --insecure was given (--insecure + --tofu is rejected in dispatch()).
    let effective_tofu = if discovered.is_some() && !insecure && tofu.is_none() {
        Some(String::new())
    } else {
        tofu
    };

    if let Some(raw) = effective_tofu {
        let fp_hint = if raw.is_empty() {
            None
        } else {
            Some(parse_fingerprint(&raw)?)
        };

        // Discovery pre-step: consistency cross-check of the advertised CA
        // fingerprint against the CA the server actually serves. Hard-fails
        // only when no explicit --tofu fingerprint outranks the
        // advertisement; emits at most one advisory line. The trust decision
        // itself is always made by establish_ca_trust below.
        if let Some(advertised) = discovered
            .as_ref()
            .and_then(|d| d.ca_fingerprint.as_deref())
        {
            let (_pem, fetched_fp) = fetch_ca(&server).await?;
            match crate::discovery::cross_check_advertised(
                Some(advertised),
                &fetched_fp,
                fp_hint.is_some(),
            ) {
                crate::discovery::CrossCheck::Ok => {}
                crate::discovery::CrossCheck::Warn(msg) => eprintln!("Warning: {msg}"),
                crate::discovery::CrossCheck::Fail(msg) => bail!(CliError::Other(msg)),
            }
        }

        establish_ca_trust(&server, fp_hint.as_deref(), false, &mut config).await?;
    }

    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let date = chrono_date();
    let client_name = format!("cli-{host}-{date}");

    let ca_pem = config.ca_pem.clone();
    let client =
        UptrakitClient::new(&server, None, insecure, ca_pem.as_deref(), None).context_to()?;

    let start_resp = client
        .oauth_device_authorization(&DeviceAuthorizationRequest::new(
            CLI_CLIENT_ID.to_string(),
            None,
            Some(client_name.clone()),
        ))
        .await
        .context_to()?;

    print_browser_instructions(&start_resp, insecure);

    eprintln!("  Waiting for authorization...");

    poll_for_token(&client, &server, &start_resp, &client_name, &mut config).await
}

fn print_browser_instructions(start_resp: &DeviceAuthorizationResponse, insecure: bool) {
    eprintln!();
    eprintln!("  Open this URL in your browser:");
    eprintln!("  {}", start_resp.verification_uri);
    eprintln!();
    eprintln!("  And enter this code: {}", start_resp.user_code);
    eprintln!();

    let url_to_open = &start_resp.verification_uri_complete;
    if let Err(e) = validate_url_scheme(url_to_open, insecure) {
        eprintln!("  (URL validation failed: {})", e);
        eprintln!("  Please verify and open the URL above manually.");
        eprintln!();
    } else if let Err(e) = open_url(url_to_open) {
        eprintln!("  (Could not open browser automatically: {})", e);
        eprintln!("  Please open the URL above manually.");
        eprintln!();
    }
}

async fn poll_for_token(
    client: &UptrakitClient,
    server: &str,
    start_resp: &DeviceAuthorizationResponse,
    client_name: &str,
    config: &mut Config,
) -> Result<()> {
    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(start_resp.expires_in);
    let mut interval = u64::try_from(start_resp.interval.max(1)).unwrap_or(5);

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        if started.elapsed() > timeout {
            bail!(CliError::Other(
                "authorization request expired, please run again".into()
            ));
        }

        let req = OAuthTokenRequest::new(
            DEVICE_CODE_GRANT.to_string(),
            Some(start_resp.device_code.clone()),
            Some(CLI_CLIENT_ID.to_string()),
        );

        match client.oauth_token(&req).await {
            Ok(resp) => {
                config.server = Some(server.to_string());
                save_config(config).await?;
                save_credentials(&Credentials {
                    token: Some(resp.access_token),
                })
                .await?;
                eprintln!();
                println!("Logged in to {} successfully.", server);
                println!("API token stored locally (name: {}).", client_name);
                return Ok(());
            }
            Err(e) => match e.current_context() {
                ClientError::OAuthError(err_resp) => match &err_resp.error {
                    OAuthErrorCode::AuthorizationPending => continue,
                    OAuthErrorCode::SlowDown => {
                        let bumped = err_resp
                            .interval
                            .and_then(|i| u64::try_from(i).ok())
                            .unwrap_or_else(|| interval.saturating_add(5));
                        interval = bumped;
                        continue;
                    }
                    OAuthErrorCode::AccessDenied => {
                        bail!(CliError::Other("authorization denied by operator".into()));
                    }
                    OAuthErrorCode::ExpiredToken => {
                        bail!(CliError::Other(
                            "authorization request expired, please run again".into()
                        ));
                    }
                    OAuthErrorCode::InvalidGrant
                    | OAuthErrorCode::InvalidClient
                    | OAuthErrorCode::InvalidRequest
                    | OAuthErrorCode::UnsupportedGrantType => {
                        bail!(CliError::Other(format!(
                            "CLI/server version mismatch: {}",
                            err_resp.error.as_str()
                        )));
                    }
                    OAuthErrorCode::ServerError => {
                        bail!(CliError::Other(
                            "server-side error, please try again".into()
                        ));
                    }
                    OAuthErrorCode::Other(s) => {
                        bail!(CliError::Other(format!("unexpected OAuth error: {s}")));
                    }
                    _ => {
                        tracing::warn!(
                            error_code = err_resp.error.as_str(),
                            "received unknown OAuth error code from server"
                        );
                        bail!(CliError::Other(format!(
                            "unexpected OAuth error: {}",
                            err_resp.error.as_str()
                        )));
                    }
                },
                ClientError::RateLimited {
                    retry_after_seconds,
                } => {
                    let delay = *retry_after_seconds;
                    tokio::time::sleep(std::time::Duration::from_secs(delay.unwrap_or(interval)))
                        .await;
                    continue;
                }
                _ => return Err(e.context_to()),
            },
        }
    }
}

/// Show current authentication status.
pub async fn status(
    server_override: Option<&str>,
    token_override: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AuthStatusOutput> {
    let config = load_config()?;
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client = UptrakitClient::new(
        &server,
        Some(&token),
        insecure,
        config.ca_pem.as_deref(),
        request_timeout,
    )
    .context_to()?;

    let user = client.me().await.context_to()?;

    let permissions: Vec<String> = user.permissions.iter().map(|p| p.to_string()).collect();

    let ca_fingerprint = if insecure {
        None
    } else {
        match config.ca_pem.as_deref() {
            None => None,
            Some(pem) => Some(pem_fingerprint(pem).map_err(|e| {
                e.attach(
                    "stored CA PEM is unparseable; run 'uptrakit auth ca trust' to re-establish",
                )
            })?),
        }
    };

    Ok(AuthStatusOutput {
        server,
        first_name: user.first_name,
        last_name: user.last_name,
        email: user.email,
        user_id: user.id,
        permissions,
        ca_fingerprint,
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
    let config = load_config()?;
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client = UptrakitClient::new(
        &server,
        Some(&token),
        insecure,
        config.ca_pem.as_deref(),
        request_timeout,
    )
    .context_to()?;

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
    let config = load_config()?;
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client = UptrakitClient::new(
        &server,
        Some(&token),
        insecure,
        config.ca_pem.as_deref(),
        request_timeout,
    )
    .context_to()?;

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
    let config = load_config()?;
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client = UptrakitClient::new(
        &server,
        Some(&token),
        insecure,
        config.ca_pem.as_deref(),
        request_timeout,
    )
    .context_to()?;

    client.revoke_api_token(id).await.context_to()?;

    Ok(TokenRevokeOutput {
        id: *id,
        revoked: true,
    })
}

/// `auth ca trust` — establish or update stored CA trust.
pub async fn ca_trust(server_override: Option<&str>, tofu: Option<String>) -> Result<()> {
    let mut config = load_config()?;

    let server = server_override
        .map(|s| s.to_string())
        .or_else(|| config.server.clone())
        .ok_or_else(|| {
            report!(CliError::Other(
                "no server URL configured; run 'uptrakit auth login' first or supply --server"
                    .into(),
            ))
        })?;

    let fp_hint = match tofu {
        None => None,
        Some(s) if s.is_empty() => None,
        Some(s) => Some(parse_fingerprint(&s)?),
    };

    establish_ca_trust(&server, fp_hint.as_deref(), true, &mut config).await
}

/// `auth ca status` — print stored CA fingerprint or "system roots".
pub fn ca_status() -> Result<()> {
    let config = load_config()?;
    match &config.ca_pem {
        None => println!("CA trust:    system roots"),
        Some(pem) => {
            let fp = pem_fingerprint(pem)?;
            println!("CA trust:    {fp}");
        }
    }
    Ok(())
}

/// `auth ca forget` — clear stored CA trust, revert to system roots.
pub async fn ca_forget() -> Result<()> {
    let mut config = load_config()?;
    config.ca_pem = None;
    save_config(&config).await?;
    eprintln!("Stored CA trust removed. System roots will be used for future connections.");
    Ok(())
}

pub(crate) fn prompt(msg: &str) -> Result<String> {
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
fn validate_url_scheme(url: &str, allow_http: bool) -> Result<()> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if allow_http && url.starts_with("http://") {
        return Ok(());
    }
    bail!(CliError::Other(format!(
        "refusing to open URL with untrusted scheme: {url}"
    )))
}

/// Open a URL in the user's default browser.
fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(url).spawn()?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg(url).spawn()?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn()?;
    // On unsupported targets the supported-target Ok(()) is compiled away, and vice versa,
    // so neither branch sees unreachable_code under warnings=deny.
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "opening browser not supported on this platform",
    ));
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
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
            ca_fingerprint: None,
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
            ca_fingerprint: None,
        };
        let s = output.to_human_string();
        assert!(s.contains("https://example.com"), "server missing");
        assert!(s.contains("john@example.com"), "email missing");
        assert!(s.contains("view_settings"), "permissions missing");
    }

    #[test]
    fn auth_status_output_includes_ca_fingerprint() {
        let fp = "e".repeat(64);
        let output = AuthStatusOutput {
            server: "https://example.com".into(),
            first_name: "Alice".into(),
            last_name: "B".into(),
            email: "alice@b.com".into(),
            user_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid"),
            permissions: vec![],
            ca_fingerprint: Some(fp.clone()),
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(v["ca_fingerprint"], fp);
    }

    #[test]
    fn auth_status_output_null_when_no_ca() {
        let output = AuthStatusOutput {
            server: "https://example.com".into(),
            first_name: "Alice".into(),
            last_name: "B".into(),
            email: "alice@b.com".into(),
            user_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid"),
            permissions: vec![],
            ca_fingerprint: None,
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(v["ca_fingerprint"], serde_json::Value::Null);
    }

    #[test]
    fn auth_status_human_output_shows_ca_trust() {
        let fp = "e".repeat(64);
        let output = AuthStatusOutput {
            server: "https://example.com".into(),
            first_name: "A".into(),
            last_name: "B".into(),
            email: "a@b.com".into(),
            user_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid"),
            permissions: vec![],
            ca_fingerprint: Some(fp.clone()),
        };
        let s = output.to_human_string();
        assert!(s.contains(&fp), "fingerprint missing from human output");
    }

    #[test]
    fn auth_status_human_output_shows_system_roots_when_no_ca() {
        let output = AuthStatusOutput {
            server: "https://example.com".into(),
            first_name: "A".into(),
            last_name: "B".into(),
            email: "a@b.com".into(),
            user_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid"),
            permissions: vec![],
            ca_fingerprint: None,
        };
        let s = output.to_human_string();
        assert!(s.contains("system roots"), "system roots line missing");
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

    mod fingerprint_tests {
        use super::*;

        #[test]
        fn plain_hex_accepted() {
            let fp = "a".repeat(64);
            assert_eq!(parse_fingerprint(&fp).unwrap(), fp);
        }

        #[test]
        fn sha256_prefix_stripped() {
            let hex = "b".repeat(64);
            let input = format!("sha256:{hex}");
            assert_eq!(parse_fingerprint(&input).unwrap(), hex);
        }

        #[test]
        fn unsupported_prefix_rejected() {
            let err = parse_fingerprint("sha1:aabbcc").unwrap_err();
            assert!(
                err.to_string()
                    .contains("unsupported fingerprint algorithm 'sha1'")
            );
        }

        #[test]
        fn wrong_length_rejected() {
            let err = parse_fingerprint("aabbcc").unwrap_err();
            assert!(err.to_string().contains("64 lowercase hex characters"));
        }

        #[test]
        fn uppercase_hex_rejected() {
            let fp = "A".repeat(64);
            let err = parse_fingerprint(&fp).unwrap_err();
            assert!(err.to_string().contains("64 lowercase hex characters"));
        }

        #[test]
        fn non_hex_chars_rejected() {
            let fp = format!("{}zzzz", "a".repeat(60));
            let err = parse_fingerprint(&fp).unwrap_err();
            assert!(err.to_string().contains("64 lowercase hex characters"));
        }

        #[test]
        fn exactly_64_lowercase_hex_passes() {
            let fp = "0123456789abcdef".repeat(4);
            assert_eq!(fp.len(), 64);
            assert_eq!(parse_fingerprint(&fp).unwrap(), fp);
        }
    }
}

#[cfg(test)]
mod ca_trust_tests {
    use super::*;
    use httpmock::prelude::*;

    /// Serialize env-mutating async tests so parallel test threads do not race on HOME.
    // parking_lot::Mutex intentionally not used here: the guard must span .await points
    // to serialize HOME env writes across async test tasks.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Run an async closure with HOME pointing at a unique temp directory.
    /// Holds `ENV_LOCK` for the full duration of the closure so that HOME is
    /// never mutated by a concurrent test while the async body is executing.
    async fn with_temp_home<F, Fut>(f: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let _guard = ENV_LOCK.lock().await;

        let tmp = std::env::temp_dir().join(format!(
            "uptrakit-auth-test-home-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).expect("create temp home");
        let prev = std::env::var_os("HOME");
        // SAFETY: ENV_LOCK is held for the entire duration of this function,
        // including all await points, so no other test thread mutates HOME
        // concurrently.
        unsafe { std::env::set_var("HOME", &tmp) };

        f().await;

        match prev {
            // SAFETY: ENV_LOCK is still held; restoring HOME to its previous value.
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            // SAFETY: ENV_LOCK is still held; no concurrent HOME access.
            None => unsafe { std::env::remove_var("HOME") },
        }
        drop(std::fs::remove_dir_all(&tmp));
        // _guard dropped here, releasing the lock.
    }

    fn make_test_cert_pem() -> String {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("key pair");
        let params = rcgen::CertificateParams::default();
        params
            .self_signed(&key_pair)
            .expect("self-signed cert")
            .pem()
    }

    fn fingerprint_of_pem(pem: &str) -> String {
        super::pem_fingerprint(pem).expect("parse pem")
    }

    #[tokio::test]
    async fn fetch_succeeds_with_matching_fingerprint_hint() {
        with_temp_home(|| async {
            let pem = make_test_cert_pem();
            let fp = fingerprint_of_pem(&pem);

            let server = MockServer::start_async().await;
            server.mock(|when, then| {
                when.method(GET).path("/api/v1/pki/ca.crt");
                then.status(200).body(pem.as_str());
            });

            let mut config = Config::default();
            establish_ca_trust(&server.base_url(), Some(&fp), false, &mut config)
                .await
                .expect("should succeed");
            assert_eq!(config.ca_pem.as_deref(), Some(pem.as_str()));
        })
        .await;
    }

    #[tokio::test]
    async fn fetch_fails_with_wrong_fingerprint_hint() {
        with_temp_home(|| async {
            let pem = make_test_cert_pem();
            let server = MockServer::start_async().await;
            server.mock(|when, then| {
                when.method(GET).path("/api/v1/pki/ca.crt");
                then.status(200).body(pem.as_str());
            });

            let wrong_fp = "0".repeat(64);
            let mut config = Config::default();
            let err = establish_ca_trust(&server.base_url(), Some(&wrong_fp), false, &mut config)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("CA fingerprint mismatch"));
            assert!(config.ca_pem.is_none());
        })
        .await;
    }

    #[tokio::test]
    async fn non_interactive_without_fingerprint_fails() {
        with_temp_home(|| async {
            let pem = make_test_cert_pem();
            let server = MockServer::start_async().await;
            server.mock(|when, then| {
                when.method(GET).path("/api/v1/pki/ca.crt");
                then.status(200).body(pem.as_str());
            });

            let mut config = Config::default();
            let err = establish_ca_trust(&server.base_url(), None, false, &mut config)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("non-interactive"));
        })
        .await;
    }

    #[tokio::test]
    async fn stored_ca_matches_fetched_proceeds_silently() {
        with_temp_home(|| async {
            let pem = make_test_cert_pem();
            let fp = fingerprint_of_pem(&pem);

            let server = MockServer::start_async().await;
            server.mock(|when, then| {
                when.method(GET).path("/api/v1/pki/ca.crt");
                then.status(200).body(pem.as_str());
            });

            let mut config = Config {
                ca_pem: Some(pem.clone()),
                ..Default::default()
            };
            establish_ca_trust(&server.base_url(), Some(&fp), false, &mut config)
                .await
                .expect("should succeed — fingerprints match");
            assert_eq!(config.ca_pem.as_deref(), Some(pem.as_str()));
        })
        .await;
    }

    #[tokio::test]
    async fn stored_ca_differs_allow_rotation_false_fails() {
        with_temp_home(|| async {
            let pem = make_test_cert_pem();
            let new_pem = make_test_cert_pem();
            let new_fp = fingerprint_of_pem(&new_pem);

            let server = MockServer::start_async().await;
            server.mock(|when, then| {
                when.method(GET).path("/api/v1/pki/ca.crt");
                then.status(200).body(new_pem.as_str());
            });

            let mut config = Config {
                ca_pem: Some(pem.clone()),
                ..Default::default()
            };
            let err = establish_ca_trust(&server.base_url(), Some(&new_fp), false, &mut config)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("Controller CA has changed"));
            assert_eq!(config.ca_pem.as_deref(), Some(pem.as_str()));
        })
        .await;
    }

    #[tokio::test]
    async fn stored_ca_differs_allow_rotation_true_updates() {
        with_temp_home(|| async {
            let pem = make_test_cert_pem();
            let new_pem = make_test_cert_pem();
            let new_fp = fingerprint_of_pem(&new_pem);

            let server = MockServer::start_async().await;
            server.mock(|when, then| {
                when.method(GET).path("/api/v1/pki/ca.crt");
                then.status(200).body(new_pem.as_str());
            });

            let mut config = Config {
                ca_pem: Some(pem.clone()),
                ..Default::default()
            };
            establish_ca_trust(&server.base_url(), Some(&new_fp), true, &mut config)
                .await
                .expect("should succeed — rotation allowed");
            assert_eq!(config.ca_pem.as_deref(), Some(new_pem.as_str()));
        })
        .await;
    }

    #[tokio::test]
    async fn non_200_response_returns_clear_error() {
        with_temp_home(|| async {
            let server = MockServer::start_async().await;
            server.mock(|when, then| {
                when.method(GET).path("/api/v1/pki/ca.crt");
                then.status(404).body("Not Found");
            });

            let mut config = Config::default();
            let err = establish_ca_trust(
                &server.base_url(),
                Some(&"a".repeat(64)),
                false,
                &mut config,
            )
            .await
            .unwrap_err();
            // Should get HTTP error, not "failed to parse CA certificate PEM"
            let msg = err.to_string();
            assert!(
                !msg.contains("failed to parse"),
                "got misleading parse error: {msg}"
            );
        })
        .await;
    }
}
