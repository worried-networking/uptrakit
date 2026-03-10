use crate::client::authenticated_client;
use crate::commands::settings::DeletedOutput;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, HostPluginRoleAssignment,
    HostSoftwareAssignment, ListSoftwareItemsParams, SoftwareItemDetailResponse,
    SoftwareItemResponse, TriggerUpdateRequest, TriggerUpdateResponse, UpdateSoftwareItemRequest,
};
use uptrakit_shared_types::PluginRole;

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<SoftwareItemResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No software items found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<25} {:<30} {:<10} {:<7} UPDATE\n",
            "ID", "NAME", "PLUGINS", "FEATURED", "HOSTS"
        );
        for item in &self.items {
            let plugins_str = if item.plugins.is_empty() {
                "-".to_string()
            } else {
                item.plugins.join(", ")
            };
            let update = if item.update_available { "yes" } else { "-" };
            out.push_str(&format!(
                "{:<38} {:<25} {:<30} {:<10} {:<7} {}\n",
                item.id, item.name, plugins_str, item.featured, item.host_count, update
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for SoftwareItemDetailResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:              {}\n", self.id));
        out.push_str(&format!("Name:            {}\n", self.name));
        let plugins_str = if self.plugins.is_empty() {
            "-".to_string()
        } else {
            self.plugins.join(", ")
        };
        out.push_str(&format!("Plugins:         {}\n", plugins_str));
        out.push_str(&format!("Featured:        {}\n", self.featured));
        if let Some(ref lv) = self.latest_version {
            out.push_str(&format!("Latest Version:  {}\n", lv));
        }
        out.push_str(&format!(
            "Update Available:{}\n",
            if self.update_available { " yes" } else { " no" }
        ));
        if let Some(checked) = self.last_checked_at {
            out.push_str(&format!(
                "Last Checked:    {}\n",
                checked
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| checked.to_string())
            ));
        }
        out.push_str(&format!("Host Count:      {}\n", self.host_count));
        out.push_str(&format!(
            "Created:         {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out.push_str(&format!(
            "Updated:         {}\n",
            self.updated_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.updated_at.to_string())
        ));
        if !self.hosts.is_empty() {
            out.push_str("\nAssigned Hosts:\n");
            out.push_str(&format!(
                "  {:<38} {:<20} {:<20} {:<30} {:<15} {:<15} {:<16} LINKED AT\n",
                "HOST ID", "HOSTNAME", "PLUGIN", "PACKAGE", "INSTALLED", "LATEST", "STATUS"
            ));
            for h in &self.hosts {
                let linked = h
                    .linked_at
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| h.linked_at.to_string());
                let status = if h.update_available {
                    "update-available"
                } else if h.installed_version.is_some() && h.latest_version.is_some() {
                    "up-to-date"
                } else {
                    "—"
                };
                // Summarise plugin info from role assignments. Show the first
                // plugin_type and package_identifier found, or "-" if none.
                let plugin_type = h
                    .plugins
                    .first()
                    .map(|p| p.plugin_type.as_str())
                    .unwrap_or("-");
                let package_identifier = h
                    .plugins
                    .first()
                    .map(|p| p.package_identifier.as_str())
                    .unwrap_or("-");
                out.push_str(&format!(
                    "  {:<38} {:<20} {:<20} {:<30} {:<15} {:<15} {:<16} {}\n",
                    h.host_id,
                    h.hostname,
                    plugin_type,
                    package_identifier,
                    h.installed_version.as_deref().unwrap_or("-"),
                    h.latest_version.as_deref().unwrap_or("-"),
                    status,
                    linked
                ));
            }
        }
        out
    }
}

impl HumanOutput for SoftwareItemResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:              {}\n", self.id));
        out.push_str(&format!("Name:            {}\n", self.name));
        let plugins_str = if self.plugins.is_empty() {
            "-".to_string()
        } else {
            self.plugins.join(", ")
        };
        out.push_str(&format!("Plugins:         {}\n", plugins_str));
        out.push_str(&format!("Featured:        {}\n", self.featured));
        if let Some(ref lv) = self.latest_version {
            out.push_str(&format!("Latest Version:  {}\n", lv));
        }
        out.push_str(&format!(
            "Update Available:{}\n",
            if self.update_available { " yes" } else { " no" }
        ));
        if let Some(checked) = self.last_checked_at {
            out.push_str(&format!(
                "Last Checked:    {}\n",
                checked
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| checked.to_string())
            ));
        }
        out.push_str(&format!("Host Count:      {}\n", self.host_count));
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for listing software items.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

/// Parameters for showing a single software item.
pub struct ShowParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct CreateParams<'a> {
    pub name: String,
    pub featured: Option<bool>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct UpdateParams<'a> {
    pub id: &'a Uuid,
    pub name: Option<String>,
    pub featured: Option<bool>,
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

pub struct ApproveParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct AssignParams<'a> {
    pub id: &'a Uuid,
    pub host_id: &'a Uuid,
    pub plugin_config_id: Option<&'a Uuid>,
    pub package_identifier: Option<String>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct UnassignParams<'a> {
    pub id: &'a Uuid,
    pub host_id: &'a Uuid,
    pub ignore: bool,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// Parameters for `update-latest`: trigger update to the current `latest_version`.
pub struct UpdateLatestParams<'a> {
    pub id: &'a Uuid,
    pub host_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List all software items (paginated).
pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<SoftwareItemResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let list_params = ListSoftwareItemsParams {
        page: params.page,
        per_page: params.per_page,
        featured: None,
        host_id: None,
    };
    client.list_software_items(&list_params).await.context_to()
}

/// Show details for a single software item.
pub async fn show(params: ShowParams<'_>) -> Result<SoftwareItemDetailResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.get_software_item(params.id).await.context_to()
}

pub async fn create(params: CreateParams<'_>) -> Result<SoftwareItemResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateSoftwareItemRequest {
        name: params.name,
        featured: params.featured.unwrap_or(true),
    };
    client.create_software_item(&req).await.context_to()
}

pub async fn update(params: UpdateParams<'_>) -> Result<SoftwareItemResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateSoftwareItemRequest {
        name: params.name,
        featured: params.featured,
    };
    client
        .update_software_item(params.id, &req)
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
    client.delete_software_item(params.id).await.context_to()?;
    Ok(DeletedOutput {
        message: format!("Software item {} deleted.", params.id),
    })
}

pub async fn approve(params: ApproveParams<'_>) -> Result<SoftwareItemResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.approve_software_item(params.id).await.context_to()
}

pub async fn assign(params: AssignParams<'_>) -> Result<SoftwareItemDetailResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    // Build a role assignment for each of the three lifecycle roles using the
    // same plugin config and package identifier. Users who need finer-grained
    // per-role configuration can use the HTTP API directly.
    let roles = [
        PluginRole::DetectVersion,
        PluginRole::FetchReleases,
        PluginRole::ExecuteUpdate,
    ];
    let plugins: Vec<HostPluginRoleAssignment> = roles
        .into_iter()
        .map(|role| HostPluginRoleAssignment {
            role,
            plugin_config_id: params.plugin_config_id.copied(),
            plugin_config: None,
            package_identifier: params.package_identifier.clone().unwrap_or_default(),
            config_override: None,
            execution_site: "auto".to_string(),
        })
        .collect();
    let req = AssignHostsRequest {
        host_assignments: vec![HostSoftwareAssignment {
            host_id: *params.host_id,
            plugins,
        }],
    };
    client.assign_hosts(params.id, &req).await.context_to()
}

pub async fn unassign(params: UnassignParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    if params.ignore {
        client
            .unassign_host_with_ignore(params.id, params.host_id)
            .await
            .context_to()?;
    } else {
        client
            .unassign_host(params.id, params.host_id)
            .await
            .context_to()?;
    }
    Ok(DeletedOutput {
        message: format!(
            "Host {} unassigned from software item {}.",
            params.host_id, params.id
        ),
    })
}

/// Trigger an update to the latest known version for a software item on a specific host.
///
/// Loads the software item detail to obtain `latest_version`, then calls the update
/// trigger API. Errors if no latest version is known yet (run `check item <id>` first).
pub async fn update_latest(params: UpdateLatestParams<'_>) -> Result<TriggerUpdateResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    let detail = client.get_software_item(params.id).await.context_to()?;

    // Check per-host latest_version first, then fall back to item-level.
    let latest_version = detail
        .hosts
        .iter()
        .find(|h| h.host_id == *params.host_id)
        .and_then(|h| h.latest_version.clone())
        .or(detail.latest_version.clone());

    let Some(version) = latest_version else {
        bail!(crate::error::CliError::Other(format!(
            "No latest version known for software item {}.\n\
             Run `uptrakit check item {}` first, or use `uptrakit update trigger` to specify a version explicitly.",
            params.id, params.id
        )));
    };

    let req = TriggerUpdateRequest {
        to_version: version,
        release_info: None,
        interactive: false,
    };

    client
        .trigger_update(params.id, params.host_id, &req)
        .await
        .context_to()
}

/// Perform a batch action on multiple software items.
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
    client.batch_software_items(&req).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uptrakit_openapi_client::types::software_items::{
        HostPluginRoleSummary, SoftwareItemHostSummary, SoftwareItemResponse,
    };

    fn sample_item() -> SoftwareItemResponse {
        SoftwareItemResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6"
                .parse::<Uuid>()
                .unwrap(),
            name: "Node.js".to_string(),
            plugins: vec!["releases_github".to_string()],
            featured: true,
            last_checked_at: None,
            host_count: 2,
            latest_version: None,
            update_available: false,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
        }
    }

    fn sample_item_with_update() -> SoftwareItemResponse {
        SoftwareItemResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6"
                .parse::<Uuid>()
                .unwrap(),
            name: "Homebrew App".to_string(),
            plugins: vec!["package_manager_homebrew".to_string()],
            featured: true,
            last_checked_at: None,
            host_count: 1,
            latest_version: Some("3.0.0".to_string()),
            update_available: true,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
        }
    }

    #[test]
    fn paginated_software_items_empty() {
        let resp: PaginatedResponse<SoftwareItemResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No software items"));
    }

    #[test]
    fn paginated_software_items_has_header_and_row() {
        let resp = PaginatedResponse {
            items: vec![sample_item()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("Node.js"), "name missing");
        assert!(s.contains("releases_github"), "plugin missing");
        assert!(s.contains("UPDATE"), "UPDATE column header missing");
    }

    #[test]
    fn paginated_software_items_update_column() {
        let resp = PaginatedResponse {
            items: vec![sample_item_with_update()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("yes"), "update-available row should show 'yes'");
    }

    #[test]
    fn software_item_detail_human_output() {
        let detail = SoftwareItemDetailResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6"
                .parse::<Uuid>()
                .unwrap(),
            name: "Node.js".to_string(),
            plugins: vec!["releases_github".to_string()],
            featured: true,
            last_checked_at: None,
            host_count: 1,
            latest_version: Some("22.0.0".to_string()),
            update_available: true,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
            hosts: vec![SoftwareItemHostSummary {
                id: "f1f2f3f4-e1e2-d1d2-c1c2-b1b2b3b4b5b6"
                    .parse::<Uuid>()
                    .unwrap(),
                host_id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                    .parse::<Uuid>()
                    .unwrap(),
                hostname: "server-1.local".to_string(),
                friendly_name: "Production Server".to_string(),
                qualifier: None,
                plugins: vec![HostPluginRoleSummary {
                    role: PluginRole::DetectVersion,
                    plugin_config_id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                        .parse::<Uuid>()
                        .unwrap(),
                    plugin_config_name: "Default Config".to_string(),
                    plugin_type: "releases_github".to_string(),
                    package_identifier: "nodejs/node".to_string(),
                    config_override: None,
                    execution_site: "auto".to_string(),
                }],
                installed_version: Some("20.0.0".to_string()),
                installed_version_detected_at: None,
                latest_version: Some("22.0.0".to_string()),
                latest_release_metadata: None,
                update_available: true,
                update_category: "unknown".to_string(),
                last_updated_at: None,
                linked_at: datetime!(2025-01-01 00:00:00 UTC),
            }],
        };
        let s = detail.to_human_string();
        assert!(s.contains("Node.js"), "name missing");
        assert!(s.contains("server-1.local"), "hostname missing");
        assert!(s.contains("20.0.0"), "installed version missing");
        assert!(s.contains("22.0.0"), "latest version missing");
        assert!(s.contains("update-available"), "host status missing");
        assert!(
            s.contains("Latest Version:"),
            "item latest version header missing"
        );
    }

    #[test]
    fn software_item_detail_up_to_date_status() {
        let detail = SoftwareItemDetailResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6"
                .parse::<Uuid>()
                .unwrap(),
            name: "Curl".to_string(),
            plugins: vec!["package_manager_homebrew".to_string()],
            featured: true,
            last_checked_at: None,
            host_count: 1,
            latest_version: Some("8.7.0".to_string()),
            update_available: false,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
            hosts: vec![SoftwareItemHostSummary {
                id: "f1f2f3f4-e1e2-d1d2-c1c2-b1b2b3b4b5b6"
                    .parse::<Uuid>()
                    .unwrap(),
                host_id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                    .parse::<Uuid>()
                    .unwrap(),
                hostname: "mac-1".to_string(),
                friendly_name: "mac-1".to_string(),
                qualifier: None,
                plugins: vec![HostPluginRoleSummary {
                    role: PluginRole::DetectVersion,
                    plugin_config_id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                        .parse::<Uuid>()
                        .unwrap(),
                    plugin_config_name: "Homebrew".to_string(),
                    plugin_type: "package_manager_homebrew".to_string(),
                    package_identifier: "curl".to_string(),
                    config_override: None,
                    execution_site: "auto".to_string(),
                }],
                installed_version: Some("8.7.0".to_string()),
                installed_version_detected_at: None,
                latest_version: Some("8.7.0".to_string()),
                latest_release_metadata: None,
                update_available: false,
                update_category: "unknown".to_string(),
                last_updated_at: None,
                linked_at: datetime!(2025-01-01 00:00:00 UTC),
            }],
        };
        let s = detail.to_human_string();
        assert!(
            s.contains("up-to-date"),
            "host status should show up-to-date"
        );
    }

    #[test]
    fn software_item_response_human_output() {
        let item = SoftwareItemResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6"
                .parse::<Uuid>()
                .unwrap(),
            name: "My App".to_string(),
            plugins: vec!["package_manager_homebrew".to_string()],
            featured: true,
            last_checked_at: None,
            host_count: 0,
            latest_version: Some("1.5.0".to_string()),
            update_available: true,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
        };
        let s = item.to_human_string();
        assert!(s.contains("My App"));
        assert!(s.contains("package_manager_homebrew"));
        assert!(s.contains("true"), "featured should appear");
        assert!(s.contains("1.5.0"), "latest version should appear");
        assert!(s.contains("yes"), "update_available should show 'yes'");
    }

    #[test]
    fn software_item_response_no_update_available() {
        let item = SoftwareItemResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6"
                .parse::<Uuid>()
                .unwrap(),
            name: "Up-to-date App".to_string(),
            plugins: vec![],
            featured: true,
            last_checked_at: None,
            host_count: 1,
            latest_version: None,
            update_available: false,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
        };
        let s = item.to_human_string();
        assert!(s.contains(" no"), "update_available should show 'no'");
        assert!(
            !s.contains("Latest Version:"),
            "latest version should not appear when None"
        );
    }
}
