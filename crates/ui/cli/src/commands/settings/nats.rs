use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::settings_nats::{
    NatsSettingsResponse, UpdateNatsSettingsRequest,
};

#[derive(Debug, Subcommand)]
pub enum NatsCommands {
    /// Show current NATS server URL configuration
    Show,
    /// Set the NATS server URL
    Set {
        /// NATS server URL (e.g. nats://host:4222 or nats://user:password@host:4222)
        #[arg(long)]
        url: String,
    },
    /// Clear the stored NATS server URL
    Clear,
}

// ── Human output ─────────────────────────────────────────────────────────────

impl HumanOutput for NatsSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "URL:      {}\n",
            self.url
                .as_ref()
                .map(|u| u.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        out.push_str(&format!("Has URL:  {}\n", self.has_url));
        out
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Show current NATS settings.
pub async fn nats_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<NatsSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_nats_settings().await.context_to()
}

/// Set the NATS URL.
pub async fn nats_set(
    url: String,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<NatsSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateNatsSettingsRequest {
        url: Some(serde_json::Value::String(url)),
    };
    client.update_nats_settings(&req).await.context_to()
}

/// Clear the NATS URL.
pub async fn nats_clear(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<NatsSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateNatsSettingsRequest {
        url: Some(serde_json::Value::Null),
    };
    client.update_nats_settings(&req).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nats_settings_human_output_with_url() {
        let resp = NatsSettingsResponse {
            url: Some(uptrakit_openapi_client::types::MaskedUrl::new(
                "nats://user:secret@host:4222",
            )),
            has_url: true,
        };
        let s = resp.to_human_string();
        assert!(
            s.contains("has_url") || s.contains("Has URL"),
            "has_url missing"
        );
        // Password must not appear
        assert!(!s.contains("secret"), "password must not appear in output");
        assert!(s.contains("***"), "masked password must appear");
    }

    #[test]
    fn nats_settings_human_output_no_url() {
        let resp = NatsSettingsResponse {
            url: None,
            has_url: false,
        };
        let s = resp.to_human_string();
        assert!(
            s.contains('-') || s.contains("false"),
            "empty state should show"
        );
    }
}
