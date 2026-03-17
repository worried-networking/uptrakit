use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::commands::settings::DeletedOutput;
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;

#[derive(Debug, Subcommand)]
pub enum PluginConfigsCommands {
    /// List plugin configurations
    List {
        /// Page number
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show a plugin configuration
    Show {
        /// Plugin config UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Create a new plugin configuration
    Create {
        /// Config name
        #[arg(long)]
        name: String,
        /// Plugin type (github_releases, docker, homebrew, proxmox_helper_scripts)
        #[arg(long)]
        plugin_type: String,
        /// Plugin-specific config as JSON string
        #[arg(long)]
        config: Option<String>,
        /// Enable on creation
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Update a plugin configuration
    Update {
        /// Plugin config UUID
        id: uptrakit_openapi_client::Uuid,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// Updated config as JSON string
        #[arg(long)]
        config: Option<String>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Delete a plugin configuration
    Delete {
        /// Plugin config UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Trigger autodiscovery for a plugin config
    Discover {
        /// Plugin config UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Perform a batch action on multiple plugin configs
    Batch {
        /// Action to perform (e.g. delete, enable, disable)
        action: String,
        /// Plugin config UUIDs (space-separated)
        ids: Vec<uptrakit_openapi_client::Uuid>,
    },
    /// Test a plugin configuration (dry-run)
    Test {
        /// Plugin type (e.g. generic_shell, releases_github, homebrew)
        #[arg(long)]
        plugin_type: String,
        /// Plugin-specific config as JSON string
        #[arg(long)]
        config: String,
        /// Optional saved config ID to merge with
        #[arg(long)]
        plugin_config_id: Option<uptrakit_openapi_client::Uuid>,
        /// Host ID to test against (required for agent-side plugins)
        #[arg(long)]
        host_id: Option<uptrakit_openapi_client::Uuid>,
        /// What to test (version_detection, update_command_validation, pre_update_hook, post_update_hook, connectivity)
        #[arg(long)]
        test_kind: Option<String>,
        /// Package identifier for testing
        #[arg(long)]
        package_identifier: Option<String>,
    },
}

pub async fn dispatch(command: PluginConfigsCommands, ctx: &CliContext) -> Result<()> {
    match command {
        PluginConfigsCommands::List { page, per_page } => {
            let resp = list(ListParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                page,
                per_page,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginConfigsCommands::Show { id } => {
            let resp = show(ShowParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginConfigsCommands::Create {
            name,
            plugin_type,
            config,
            enabled,
        } => {
            let plugin_type: uptrakit_shared_types::PluginType =
                plugin_type.parse().map_err(|_| {
                    report!(CliError::Other(format!(
                        "unknown plugin type: {plugin_type}"
                    )))
                })?;
            let config_value: serde_json::Value = match config {
                Some(s) => serde_json::from_str(&s).map_err(|e| {
                    report!(CliError::Other(format!("invalid JSON for --config: {e}")))
                })?,
                None => serde_json::Value::Object(serde_json::Map::new()),
            };
            let resp = create(CreateParams {
                name,
                plugin_type,
                config: config_value,
                enabled,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginConfigsCommands::Update {
            id,
            name,
            config,
            enabled,
        } => {
            let config_value: Option<serde_json::Value> = match config {
                Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                    report!(CliError::Other(format!("invalid JSON for --config: {e}")))
                })?),
                None => None,
            };
            let resp = update(UpdateParams {
                id: &id,
                name,
                config: config_value,
                enabled,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginConfigsCommands::Delete { id } => {
            let resp = delete(DeleteParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginConfigsCommands::Discover { id } => {
            let resp = discover(DiscoverParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginConfigsCommands::Batch { action, ids } => {
            let resp = batch(
                &action,
                &ids,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        PluginConfigsCommands::Test {
            plugin_type,
            config,
            plugin_config_id,
            host_id,
            test_kind,
            package_identifier,
        } => {
            let config_value: serde_json::Value = serde_json::from_str(&config)
                .map_err(|e| report!(CliError::Other(format!("invalid JSON for --config: {e}"))))?;
            let resp = test_config(TestConfigParams {
                plugin_type,
                config: config_value,
                plugin_config_id,
                host_id,
                test_kind,
                package_identifier,
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
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::autodiscovery::TriggerDiscoveryResponse;
use uptrakit_openapi_client::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use uptrakit_openapi_client::types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_openapi_client::types::plugin_config_test::{
    TestPluginConfigRequest, TestPluginConfigResponse,
};
use uptrakit_openapi_client::types::plugin_configs::{
    CreatePluginConfigRequest, PluginConfigResponse, UpdatePluginConfigRequest,
};
use uptrakit_shared_types::PluginType;

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<PluginConfigResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No plugin configs found.\n".to_string();
        }
        let mut out = format!("{:<38} {:<25} {:<20} ENABLED\n", "ID", "NAME", "TYPE");
        for c in &self.items {
            out.push_str(&format!(
                "{:<38} {:<25} {:<20} {}\n",
                c.id, c.name, c.plugin_type, c.enabled
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for PluginConfigResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:           {}\n", self.id));
        out.push_str(&format!("Name:         {}\n", self.name));
        out.push_str(&format!("Type:         {}\n", self.plugin_type));
        out.push_str(&format!("Enabled:      {}\n", self.enabled));
        out.push_str(&format!("Config:       {}\n", self.config));
        out.push_str(&format!(
            "Created:      {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out
    }
}

impl HumanOutput for TestPluginConfigResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Result:       {}\n",
            if self.success { "OK" } else { "FAILED" }
        ));
        out.push_str(&format!("Test kind:    {}\n", self.test_kind));
        if let Some(v) = &self.detected_version {
            out.push_str(&format!("Version:      {v}\n"));
        }
        if let Some(o) = &self.output {
            out.push_str(&format!("Output:       {o}\n"));
        }
        if let Some(e) = &self.error {
            out.push_str(&format!("Error:        {e}\n"));
        }
        out.push_str(&format!("Duration:     {}ms\n", self.duration_ms));
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

pub struct ShowParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct CreateParams<'a> {
    pub name: String,
    pub plugin_type: PluginType,
    pub config: Value,
    pub enabled: Option<bool>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct UpdateParams<'a> {
    pub id: &'a Uuid,
    pub name: Option<String>,
    pub config: Option<Value>,
    pub enabled: Option<bool>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct DeleteParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct DiscoverParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct TestConfigParams<'a> {
    pub plugin_type: String,
    pub config: Value,
    pub plugin_config_id: Option<Uuid>,
    pub host_id: Option<Uuid>,
    pub test_kind: Option<String>,
    pub package_identifier: Option<String>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<PluginConfigResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let pagination = PaginationParams {
        page: params.page,
        per_page: params.per_page,
    };
    client.list_plugin_configs(&pagination).await.context_to()
}

pub async fn show(params: ShowParams<'_>) -> Result<PluginConfigResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.get_plugin_config(params.id).await.context_to()
}

pub async fn create(params: CreateParams<'_>) -> Result<PluginConfigResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreatePluginConfigRequest {
        name: params.name,
        plugin_type: params.plugin_type,
        config: params.config,
        enabled: params.enabled.unwrap_or(true),
    };
    client.create_plugin_config(&req).await.context_to()
}

pub async fn update(params: UpdateParams<'_>) -> Result<PluginConfigResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdatePluginConfigRequest {
        name: params.name,
        config: params.config,
        enabled: params.enabled,
    };
    client
        .update_plugin_config(params.id, &req)
        .await
        .context_to()
}

pub async fn delete(params: DeleteParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.delete_plugin_config(params.id).await.context_to()?;
    Ok(DeletedOutput {
        message: format!("Plugin config {} deleted.", params.id),
    })
}

pub async fn discover(params: DiscoverParams<'_>) -> Result<TriggerDiscoveryResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.discover_plugin_config(params.id).await.context_to()
}

/// Perform a batch action on multiple plugin configs.
pub async fn batch(
    action: &str,
    ids: &[Uuid],
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<BatchActionResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = BatchActionRequest {
        action: action.to_string(),
        ids: ids.to_vec(),
    };
    client.batch_plugin_configs(&req).await.context_to()
}

pub async fn test_config(params: TestConfigParams<'_>) -> Result<TestPluginConfigResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = TestPluginConfigRequest::new(params.plugin_type, params.config)
        .with_plugin_config_id(params.plugin_config_id)
        .with_host_id(params.host_id)
        .with_test_kind(params.test_kind)
        .with_package_identifier(params.package_identifier);
    client.test_plugin_config(&req).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_config() -> PluginConfigResponse {
        PluginConfigResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            name: "GitHub Releases".to_string(),
            plugin_type: PluginType::ReleasesGithub,
            config: serde_json::json!({"tag_strip_prefix": "v"}),
            enabled: true,
            capabilities: vec!["controller_side_fetch_releases".to_string()],
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
        }
    }

    #[test]
    fn plugin_config_detail_human_output() {
        let c = sample_config();
        let s = c.to_human_string();
        assert!(s.contains("GitHub Releases"), "name missing");
        assert!(s.contains("releases_github"), "type missing");
    }

    #[test]
    fn paginated_plugin_configs_empty() {
        let resp: PaginatedResponse<PluginConfigResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No plugin configs"));
    }

    #[test]
    fn paginated_plugin_configs_has_row() {
        let resp = PaginatedResponse {
            items: vec![sample_config()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("GitHub Releases"), "name missing");
    }
}
