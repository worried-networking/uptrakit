use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::settings_agent_certs::{
    AgentCertificateSettingsResponse, UpdateAgentCertificateSettingsRequest,
};

#[derive(Debug, Subcommand)]
pub enum CertificateCommands {
    /// Show agent certificate settings
    Show,
    /// Update agent certificate settings
    Update {
        /// Certificate lifetime in hours (max 17520)
        #[arg(long)]
        lifetime_hours: Option<u32>,
        /// Certificate renewal window in hours (use 0 to reset to automatic: min(14 days, lifetime/5))
        #[arg(long)]
        renewal_window_hours: Option<u16>,
    },
}

// ── Human output ─────────────────────────────────────────────────────────────

impl HumanOutput for AgentCertificateSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Lifetime (hours):        {}\n",
            self.lifetime_hours
        ));
        let window_desc = match self.renewal_window_hours_override {
            None => format!(
                "automatic ({} hours, 1/5 of lifetime capped at 14 days)",
                self.effective_renewal_window_hours
            ),
            Some(h) => format!("{h} hours (custom override)"),
        };
        out.push_str(&format!("Renewal Window:          {window_desc}\n"));
        out
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Show agent certificate settings.
pub async fn certificates_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AgentCertificateSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_agent_certificate_settings().await.context_to()
}

/// Update agent certificate settings.
pub async fn certificates_update(
    lifetime_hours: Option<u32>,
    renewal_window_hours: Option<u16>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AgentCertificateSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateAgentCertificateSettingsRequest {
        lifetime_hours,
        renewal_window_hours,
    };
    client
        .update_agent_certificate_settings(&req)
        .await
        .context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_settings_human_output_auto_mode() {
        let resp = AgentCertificateSettingsResponse {
            lifetime_hours: 8760,
            renewal_window_hours_override: None,
            effective_renewal_window_hours: 336,
        };
        let s = resp.to_human_string();
        assert!(s.contains("8760"), "lifetime_hours missing");
        assert!(s.contains("336"), "effective hours missing");
        assert!(s.contains("automatic"), "auto mode indicator missing");
    }

    #[test]
    fn certificate_settings_human_output_custom_override() {
        let resp = AgentCertificateSettingsResponse {
            lifetime_hours: 8760,
            renewal_window_hours_override: Some(72),
            effective_renewal_window_hours: 72,
        };
        let s = resp.to_human_string();
        assert!(s.contains("8760"), "lifetime_hours missing");
        assert!(s.contains("72"), "override hours missing");
        assert!(s.contains("custom"), "custom indicator missing");
    }
}
