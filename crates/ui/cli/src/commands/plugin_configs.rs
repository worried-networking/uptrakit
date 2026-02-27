use crate::client::authenticated_client;
use crate::commands::settings::DeletedOutput;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::autodiscovery::{
    DiscardDiscoveredResponse, TriggerDiscoveryResponse,
};
use uptrakit_openapi_client::types::pagination::{PaginatedResponse, PaginationParams};
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

pub struct DiscardDiscoveredParams<'a> {
    pub id: &'a Uuid,
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

pub async fn discard_discovered(
    params: DiscardDiscoveredParams<'_>,
) -> Result<DiscardDiscoveredResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .discard_plugin_config_discovered(params.id)
        .await
        .context_to()
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
            capabilities: vec!["discover_remote_releases".to_string()],
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
