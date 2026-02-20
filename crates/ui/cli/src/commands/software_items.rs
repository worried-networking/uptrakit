use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::pagination::PaginationParams;

/// Parameters for listing software items.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

/// Parameters for showing a single software item.
pub struct ShowParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
}

/// List all software items (paginated).
pub async fn list(params: ListParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure)?;
    let pagination = PaginationParams {
        page: params.page,
        per_page: params.per_page,
    };
    let resp = client.list_software_items(&pagination).await.context_to()?;

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

    print_output(params.format, &human, &resp)
}

/// Show details for a single software item.
pub async fn show(params: ShowParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure)?;
    let resp = client.get_software_item(params.id).await.context_to()?;

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

    print_output(params.format, &human, &resp)
}
