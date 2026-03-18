use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::commands::settings::DeletedOutput;
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;

#[derive(Debug, Subcommand)]
pub enum PluginTypeSettingsCommands {
    /// List all plugin-type-level settings
    List,
    /// Show plugin-type-level settings for a specific plugin type
    Show {
        /// Plugin type (e.g. releases_github)
        plugin_type: String,
    },
    /// Create or update plugin-type-level settings for a specific plugin type
    Set {
        /// Plugin type (e.g. releases_github)
        plugin_type: String,
        /// Settings as a JSON object string
        #[arg(long)]
        config: String,
    },
    /// Delete plugin-type-level settings for a specific plugin type
    Reset {
        /// Plugin type (e.g. releases_github)
        plugin_type: String,
    },
}

pub async fn dispatch(command: PluginTypeSettingsCommands, ctx: &CliContext) -> Result<()> {
    match command {
        PluginTypeSettingsCommands::List => {
            let resp = list(ListParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginTypeSettingsCommands::Show { plugin_type } => {
            let resp = show(ShowParams {
                plugin_type: &plugin_type,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginTypeSettingsCommands::Set {
            plugin_type,
            config,
        } => {
            let config: serde_json::Value = serde_json::from_str(&config)
                .map_err(|e| report!(CliError::Other(format!("invalid JSON for --config: {e}"))))?;
            let resp = set(SetParams {
                plugin_type: &plugin_type,
                config,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginTypeSettingsCommands::Reset { plugin_type } => {
            let resp = reset(ResetParams {
                plugin_type: &plugin_type,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::types::plugin_type_settings::{
    PluginTypeSettingsResponse, UpsertPluginTypeSettingsRequest,
};

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for Vec<PluginTypeSettingsResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No plugin type settings configured.\n".to_string();
        }
        let mut out = format!("{:<38} CONFIG\n", "PLUGIN TYPE");
        for s in self {
            out.push_str(&format!("{:<38} {}\n", s.plugin_type, s.config));
        }
        out
    }
}

impl HumanOutput for PluginTypeSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Plugin Type: {}\n", self.plugin_type));
        out.push_str(&format!("Config:      {}\n", self.config));
        out.push_str(&format!(
            "Created:     {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out.push_str(&format!(
            "Updated:     {}\n",
            self.updated_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.updated_at.to_string())
        ));
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ShowParams<'a> {
    pub plugin_type: &'a str,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct SetParams<'a> {
    pub plugin_type: &'a str,
    pub config: serde_json::Value,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ResetParams<'a> {
    pub plugin_type: &'a str,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List all plugin-type-level settings.
pub async fn list(params: ListParams<'_>) -> Result<Vec<PluginTypeSettingsResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.list_plugin_type_settings().await.context_to()
}

/// Get plugin-type-level settings for a specific plugin type.
pub async fn show(params: ShowParams<'_>) -> Result<PluginTypeSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .get_plugin_type_settings(params.plugin_type)
        .await
        .context_to()
}

/// Create or update plugin-type-level settings for a specific plugin type.
pub async fn set(params: SetParams<'_>) -> Result<PluginTypeSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpsertPluginTypeSettingsRequest {
        config: params.config,
    };
    client
        .upsert_plugin_type_settings(params.plugin_type, &req)
        .await
        .context_to()
}

/// Delete plugin-type-level settings for a specific plugin type.
pub async fn reset(params: ResetParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .delete_plugin_type_settings(params.plugin_type)
        .await
        .context_to()?;
    Ok(DeletedOutput {
        message: format!("Plugin type settings for {} reset.", params.plugin_type),
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uptrakit_shared_types::plugin_ids;

    fn sample_entry() -> PluginTypeSettingsResponse {
        PluginTypeSettingsResponse {
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            config: serde_json::json!({"poll_interval_secs": 300}),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 00:00:00 UTC),
        }
    }

    #[test]
    fn single_entry_human_output() {
        let e = sample_entry();
        let s = e.to_human_string();
        assert!(s.contains("Plugin Type:"));
        assert!(s.contains("releases_github"));
        assert!(s.contains("Config:"));
        assert!(s.contains("Created:"));
        assert!(s.contains("Updated:"));
    }

    #[test]
    fn list_empty_human_output() {
        let entries: Vec<PluginTypeSettingsResponse> = vec![];
        let s = entries.to_human_string();
        assert!(s.contains("No plugin type settings configured."));
    }

    #[test]
    fn list_non_empty_human_output() {
        let entries = vec![sample_entry()];
        let s = entries.to_human_string();
        assert!(s.contains("PLUGIN TYPE"));
        assert!(s.contains("CONFIG"));
        assert!(s.contains("releases_github"));
    }
}
