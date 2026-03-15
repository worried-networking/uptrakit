use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::settings_auth::{
    AuthenticationSettingsResponse, UpdateAuthenticationSettingsRequest,
};

#[derive(Debug, Subcommand)]
pub enum AuthenticationCommands {
    /// Show authentication settings
    Show,
    /// Update authentication settings
    Update {
        /// Enable or disable password authentication
        #[arg(long)]
        password_auth_enabled: Option<bool>,
    },
}

// ── Human output ─────────────────────────────────────────────────────────────

impl HumanOutput for AuthenticationSettingsResponse {
    fn to_human_string(&self) -> String {
        format!("Password Auth Enabled:  {}\n", self.password_auth_enabled)
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Show authentication settings.
pub async fn authentication_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AuthenticationSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_authentication_settings().await.context_to()
}

/// Update authentication settings.
pub async fn authentication_update(
    password_auth_enabled: Option<bool>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AuthenticationSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateAuthenticationSettingsRequest {
        password_auth_enabled,
    };
    client
        .update_authentication_settings(&req)
        .await
        .context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_settings_human_output() {
        let resp = AuthenticationSettingsResponse {
            password_auth_enabled: false,
        };
        let s = resp.to_human_string();
        assert!(s.contains("false"), "password_auth_enabled missing");
    }
}
