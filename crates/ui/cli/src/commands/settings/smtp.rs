use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::settings_smtp::{
    SmtpSettingsResponse, UpdateSmtpSettingsRequest,
};

#[derive(Debug, Subcommand)]
pub enum SmtpCommands {
    /// Show current SMTP settings
    Show,
    /// Update SMTP settings
    Set {
        /// SMTP server hostname
        #[arg(long)]
        host: Option<String>,
        /// SMTP server port (default: 587)
        #[arg(long)]
        port: Option<u16>,
        /// SMTP username
        #[arg(long)]
        username: Option<String>,
        /// Clear the saved username
        #[arg(long, conflicts_with = "username")]
        clear_username: bool,
        /// SMTP password
        #[arg(long)]
        password: Option<String>,
        /// Clear the saved password
        #[arg(long, conflicts_with = "password")]
        clear_password: bool,
        /// Sender email address
        #[arg(long)]
        from_address: Option<String>,
        /// Sender display name
        #[arg(long)]
        from_name: Option<String>,
        /// Clear the saved sender display name
        #[arg(long, conflicts_with = "from_name")]
        clear_from_name: bool,
        /// TLS mode: starttls, tls, or none (default: starttls)
        #[arg(long)]
        tls_mode: Option<String>,
        /// EHLO hostname sent in the SMTP EHLO command
        #[arg(long)]
        helo_host: Option<String>,
        /// Clear the saved EHLO hostname (derive from from_address domain)
        #[arg(long, conflicts_with = "helo_host")]
        clear_helo_host: bool,
    },
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for setting SMTP configuration.
pub struct SmtpSetParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub clear_username: bool,
    pub password: Option<String>,
    pub clear_password: bool,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub clear_from_name: bool,
    pub tls_mode: Option<String>,
    pub helo_host: Option<String>,
    pub clear_helo_host: bool,
}

// ── Human output ─────────────────────────────────────────────────────────────

impl HumanOutput for SmtpSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Host:          {}\n",
            self.host.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!(
            "Port:          {}\n",
            self.port.map_or("-".to_string(), |p| p.to_string())
        ));
        out.push_str(&format!(
            "Username:      {}\n",
            self.username.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!("Has Password:  {}\n", self.has_password));
        out.push_str(&format!(
            "From Address:  {}\n",
            self.from_address.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!(
            "From Name:     {}\n",
            self.from_name.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!("TLS Mode:      {}\n", self.tls_mode));
        out.push_str(&format!(
            "Helo Host:     {}\n",
            self.helo_host.as_deref().unwrap_or("-")
        ));
        out
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Show current SMTP settings.
pub async fn smtp_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<SmtpSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_smtp_settings().await.context_to()
}

/// Update SMTP settings.
pub async fn smtp_set(params: SmtpSetParams<'_>) -> Result<SmtpSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let username = if params.clear_username {
        Some(serde_json::Value::Null)
    } else {
        params.username.map(serde_json::Value::String)
    };
    let password = if params.clear_password {
        Some(serde_json::Value::Null)
    } else {
        params.password.map(serde_json::Value::String)
    };
    let from_name = if params.clear_from_name {
        Some(serde_json::Value::Null)
    } else {
        params.from_name.map(serde_json::Value::String)
    };
    let helo_host = if params.clear_helo_host {
        Some(serde_json::Value::Null)
    } else {
        params.helo_host.map(serde_json::Value::String)
    };
    let req = UpdateSmtpSettingsRequest {
        host: params.host,
        port: params.port,
        username,
        password,
        from_address: params.from_address,
        from_name,
        tls_mode: params.tls_mode,
        helo_host,
    };
    client.update_smtp_settings(&req).await.context_to()
}
