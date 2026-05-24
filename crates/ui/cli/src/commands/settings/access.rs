use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::SecretString;
use uptrakit_openapi_client::types::registration::RegistrationMode;
use uptrakit_openapi_client::types::settings_access::{
    AccessSettingsResponse, UpdateAccessSettingsRequest,
};

#[derive(Debug, Subcommand)]
pub enum AccessCommands {
    /// Show registration and authentication settings
    Show,
    /// Update access settings
    Update {
        /// Registration mode (open, invite, closed)
        #[arg(long, value_parser = crate::commands::parse_registration_mode, required = true)]
        mode: RegistrationMode,
        /// Registration token (required when mode is invite)
        #[arg(long)]
        token: Option<String>,
        /// Require registration token for OIDC users (only valid with invite mode)
        #[arg(long)]
        require_token_for_oidc: Option<bool>,
        /// Enable or disable password authentication
        #[arg(long)]
        password_auth_enabled: Option<bool>,
        /// Require two-factor authentication for all users
        #[arg(long)]
        two_factor_required: Option<bool>,
    },
}

impl HumanOutput for AccessSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::from("Registration:\n");
        out.push_str(&format!(
            "  Mode:                    {}\n",
            self.mode.as_str()
        ));
        out.push_str(&format!(
            "  Require Token for OIDC:  {}\n",
            self.require_token_for_oidc
        ));
        out.push_str("\nAuthentication:\n");
        out.push_str(&format!(
            "  Password Auth Enabled:   {}\n",
            self.password_auth_enabled
        ));
        out.push_str(&format!(
            "  Two-Factor Required:     {}\n",
            self.two_factor_required
        ));
        out
    }
}

/// Parameters for showing access settings.
pub struct AccessShowParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// Parameters for updating access settings.
pub struct AccessUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub auth_token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub mode: RegistrationMode,
    pub reg_token: Option<String>,
    pub require_token_for_oidc: Option<bool>,
    pub password_auth_enabled: Option<bool>,
    pub two_factor_required: Option<bool>,
}

pub async fn access_show(params: AccessShowParams<'_>) -> Result<AccessSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let (resp, _etag) = client.get_access_settings().await.context_to()?;
    Ok(resp)
}

pub async fn access_update(params: AccessUpdateParams<'_>) -> Result<AccessSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.auth_token,
        params.insecure,
        params.request_timeout,
    )?;
    let (_current, etag) = client.get_access_settings().await.context_to()?;
    let req = UpdateAccessSettingsRequest {
        mode: params.mode,
        token: params.reg_token.map(SecretString::new),
        require_token_for_oidc: params.require_token_for_oidc,
        password_auth_enabled: params.password_auth_enabled,
        two_factor_required: params.two_factor_required,
    };
    let (resp, _new_etag) = client
        .update_access_settings(&req, &etag)
        .await
        .context_to()?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_settings_human_output() {
        // Use serde deserialization — AccessSettingsResponse is #[non_exhaustive] and
        // cannot be constructed via struct literal in external crates.
        let resp: AccessSettingsResponse = serde_json::from_value(serde_json::json!({
            "mode": "invite",
            "require_token_for_oidc": true,
            "password_auth_enabled": false,
            "two_factor_required": true
        }))
        .expect("fixture should deserialize");
        let s = resp.to_human_string();
        assert!(s.contains("invite"), "mode missing");
        assert!(s.contains("true"), "require_token_for_oidc missing");
        assert!(s.contains("false"), "password_auth_enabled missing");
    }
}
