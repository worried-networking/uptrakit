use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::SecretString;
use uptrakit_openapi_client::types::registration::RegistrationMode;
use uptrakit_openapi_client::types::settings::{
    RegistrationSettingsResponse, UpdateRegistrationSettingsRequest,
};

#[derive(Debug, Subcommand)]
pub enum RegistrationCommands {
    /// Show registration settings
    Show,
    /// Update registration settings
    Update {
        /// Registration mode (open, invite, closed)
        #[arg(long, value_parser = crate::commands::parse_registration_mode)]
        mode: RegistrationMode,
        /// Registration token (required for invite mode)
        #[arg(long)]
        token: Option<String>,
        /// Whether OIDC users also need a registration token
        #[arg(long)]
        require_token_for_oidc: Option<bool>,
    },
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for updating registration settings.
pub struct RegistrationUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub mode: RegistrationMode,
    pub reg_token: Option<String>,
    pub require_token_for_oidc: Option<bool>,
}

// ── Human output ─────────────────────────────────────────────────────────────

impl HumanOutput for RegistrationSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Mode:                    {}\n",
            self.mode.as_str()
        ));
        out.push_str(&format!(
            "Require Token for OIDC:  {}\n",
            self.require_token_for_oidc
        ));
        out
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Show registration settings.
pub async fn registration_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<RegistrationSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_registration_settings().await.context_to()
}

/// Update registration settings.
pub async fn registration_update(
    params: RegistrationUpdateParams<'_>,
) -> Result<RegistrationSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateRegistrationSettingsRequest {
        mode: params.mode,
        token: params.reg_token.map(SecretString::new),
        require_token_for_oidc: params.require_token_for_oidc,
    };
    client.update_registration_settings(&req).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_settings_human_output() {
        let resp = RegistrationSettingsResponse {
            mode: RegistrationMode::Invite,
            require_token_for_oidc: true,
        };
        let s = resp.to_human_string();
        assert!(s.contains("invite"), "mode missing");
        assert!(s.contains("true"), "require_token_for_oidc missing");
    }
}
