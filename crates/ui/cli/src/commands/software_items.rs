use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::software_items::{SoftwareItemDetailResponse, SoftwareItemResponse};

/// List all software items (paginated).
pub async fn list(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    page: Option<u64>,
    per_page: Option<u64>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;

    let mut path = "/api/v1/software-items".to_string();
    let mut params = Vec::new();
    if let Some(p) = page {
        params.push(format!("page={p}"));
    }
    if let Some(pp) = per_page {
        params.push(format!("per_page={pp}"));
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }

    let resp: PaginatedResponse<SoftwareItemResponse> = client.get(&path).await?;

    let mut human = String::new();
    if resp.items.is_empty() {
        human.push_str("No software items found.\n");
    } else {
        human.push_str(&format!(
            "{:<38} {:<25} {:<20} {:<8} HOSTS\n",
            "ID", "NAME", "PROVIDER", "ENABLED"
        ));
        for item in &resp.items {
            human.push_str(&format!(
                "{:<38} {:<25} {:<20} {:<8} {}\n",
                item.id, item.name, item.provider_type, item.enabled, item.host_count
            ));
        }
        human.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            resp.page, resp.total_pages, resp.total
        ));
    }

    print_output(format, &human, &resp)
}

/// Show details for a single software item.
pub async fn show(
    id: &str,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;
    let path = format!("/api/v1/software-items/{id}");
    let resp: SoftwareItemDetailResponse = client.get(&path).await?;

    let mut human = String::new();
    human.push_str(&format!("ID:              {}\n", resp.id));
    human.push_str(&format!("Name:            {}\n", resp.name));
    human.push_str(&format!("Provider Type:   {}\n", resp.provider_type));
    human.push_str(&format!(
        "Provider Config: {} ({})\n",
        resp.provider_config_name, resp.provider_config_id
    ));
    if !resp.package_identifier.is_empty() {
        human.push_str(&format!("Package ID:      {}\n", resp.package_identifier));
    }
    human.push_str(&format!("Enabled:         {}\n", resp.enabled));
    if let Some(ref checked) = resp.last_checked_at {
        human.push_str(&format!("Last Checked:    {}\n", checked));
    }
    human.push_str(&format!("Host Count:      {}\n", resp.host_count));
    human.push_str(&format!("Created:         {}\n", resp.created_at));
    human.push_str(&format!("Updated:         {}\n", resp.updated_at));

    if !resp.hosts.is_empty() {
        human.push_str("\nAssigned Hosts:\n");
        human.push_str(&format!(
            "  {:<38} {:<20} {:<15} LINKED AT\n",
            "HOST ID", "HOSTNAME", "INSTALLED"
        ));
        for h in &resp.hosts {
            human.push_str(&format!(
                "  {:<38} {:<20} {:<15} {}\n",
                h.host_id,
                h.hostname,
                h.installed_version.as_deref().unwrap_or("-"),
                h.linked_at
            ));
        }
    }

    print_output(format, &human, &resp)
}
