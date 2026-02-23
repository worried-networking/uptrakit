use crate::client::authenticated_client;
use crate::commands::settings::DeletedOutput;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, HostSoftwareAssignment,
    ListSoftwareItemsParams, SoftwareItemDetailResponse, SoftwareItemResponse,
    UpdateSoftwareItemRequest,
};

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<SoftwareItemResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No software items found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<25} {:<30} {:<8} HOSTS\n",
            "ID", "NAME", "PROVIDERS", "ENABLED"
        );
        for item in &self.items {
            let providers = if item.provider_types.is_empty() {
                "-".to_string()
            } else {
                item.provider_types.join(", ")
            };
            out.push_str(&format!(
                "{:<38} {:<25} {:<30} {:<8} {}\n",
                item.id, item.name, providers, item.enabled, item.host_count
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
        let providers = if self.provider_types.is_empty() {
            "-".to_string()
        } else {
            self.provider_types.join(", ")
        };
        out.push_str(&format!("Providers:       {}\n", providers));
        out.push_str(&format!("Enabled:         {}\n", self.enabled));
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
                "  {:<38} {:<20} {:<20} {:<30} {:<15} LINKED AT\n",
                "HOST ID", "HOSTNAME", "PROVIDER", "PACKAGE", "INSTALLED"
            ));
            for h in &self.hosts {
                let linked = h
                    .linked_at
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| h.linked_at.to_string());
                out.push_str(&format!(
                    "  {:<38} {:<20} {:<20} {:<30} {:<15} {}\n",
                    h.host_id,
                    h.hostname,
                    h.provider_type,
                    h.package_identifier,
                    h.installed_version.as_deref().unwrap_or("-"),
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
        let providers = if self.provider_types.is_empty() {
            "-".to_string()
        } else {
            self.provider_types.join(", ")
        };
        out.push_str(&format!("Providers:       {}\n", providers));
        out.push_str(&format!("Enabled:         {}\n", self.enabled));
        if let Some(ds) = &self.discovery_state {
            out.push_str(&format!("Discovery State: {}\n", ds));
        }
        if let Some(checked) = self.last_checked_at {
            out.push_str(&format!(
                "Last Checked:    {}\n",
                checked.format(&Rfc3339).unwrap_or_else(|_| checked.to_string())
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
    pub enabled: Option<bool>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct UpdateParams<'a> {
    pub id: &'a Uuid,
    pub name: Option<String>,
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
    pub provider_config_id: Option<&'a Uuid>,
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
        discovery_state: None,
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
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
    let req = CreateSoftwareItemRequest {
        name: params.name,
        enabled: params.enabled.unwrap_or(true),
    };
    client.create_software_item(&req).await.context_to()
}

pub async fn update(params: UpdateParams<'_>) -> Result<SoftwareItemResponse> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
    let req = UpdateSoftwareItemRequest {
        name: params.name,
        enabled: params.enabled,
    };
    client.update_software_item(params.id, &req).await.context_to()
}

pub async fn delete(params: DeleteParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
    client.delete_software_item(params.id).await.context_to()?;
    Ok(DeletedOutput { message: format!("Software item {} deleted.", params.id) })
}

pub async fn approve(params: ApproveParams<'_>) -> Result<SoftwareItemResponse> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
    client.approve_software_item(params.id).await.context_to()
}

pub async fn assign(params: AssignParams<'_>) -> Result<SoftwareItemDetailResponse> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
    let req = AssignHostsRequest {
        host_assignments: vec![HostSoftwareAssignment {
            host_id: *params.host_id,
            provider_config_id: params.provider_config_id.copied(),
            provider_config: None,
            package_identifier: params.package_identifier,
            config_override: None,
        }],
    };
    client.assign_hosts(params.id, &req).await.context_to()
}

pub async fn unassign(params: UnassignParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
    if params.ignore {
        client.unassign_host_with_ignore(params.id, params.host_id).await.context_to()?;
    } else {
        client.unassign_host(params.id, params.host_id).await.context_to()?;
    }
    Ok(DeletedOutput {
        message: format!("Host {} unassigned from software item {}.", params.host_id, params.id),
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uptrakit_openapi_client::types::software_items::{SoftwareItemHostSummary, SoftwareItemResponse};

    fn sample_item() -> SoftwareItemResponse {
        SoftwareItemResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6".parse::<Uuid>().unwrap(),
            name: "Node.js".to_string(),
            provider_types: vec!["github_releases".to_string()],
            enabled: true,
            discovery_state: None,
            last_checked_at: None,
            host_count: 2,
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
        assert!(s.contains("github_releases"), "provider missing");
    }

    #[test]
    fn software_item_detail_human_output() {
        let detail = SoftwareItemDetailResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6".parse::<Uuid>().unwrap(),
            name: "Node.js".to_string(),
            provider_types: vec!["github_releases".to_string()],
            enabled: true,
            discovery_state: None,
            last_checked_at: None,
            host_count: 1,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
            hosts: vec![SoftwareItemHostSummary {
                host_id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                    .parse::<Uuid>()
                    .unwrap(),
                hostname: "server-1.local".to_string(),
                friendly_name: "Production Server".to_string(),
                provider_config_id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                    .parse::<Uuid>()
                    .unwrap(),
                provider_config_name: "Default Config".to_string(),
                provider_type: "github_releases".to_string(),
                package_identifier: "nodejs/node".to_string(),
                config_override: None,
                installed_version: Some("20.0.0".to_string()),
                installed_version_detected_at: None,
                last_updated_at: None,
                linked_at: datetime!(2025-01-01 00:00:00 UTC),
            }],
        };
        let s = detail.to_human_string();
        assert!(s.contains("Node.js"), "name missing");
        assert!(s.contains("server-1.local"), "hostname missing");
        assert!(s.contains("20.0.0"), "installed version missing");
    }

    #[test]
    fn software_item_response_human_output() {
        use uptrakit_shared_types::SoftwareDiscoveryState;
        let item = SoftwareItemResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6".parse::<Uuid>().unwrap(),
            name: "My App".to_string(),
            provider_types: vec!["homebrew".to_string()],
            enabled: true,
            discovery_state: Some(SoftwareDiscoveryState::Approved),
            last_checked_at: None,
            host_count: 0,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
        };
        let s = item.to_human_string();
        assert!(s.contains("My App"));
        assert!(s.contains("homebrew"));
        assert!(s.contains("approved"));
    }
}
