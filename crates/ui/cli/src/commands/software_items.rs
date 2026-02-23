use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::software_items::ListSoftwareItemsParams;

/// Parameters for listing software items.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
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
    pub format: OutputFormat,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// List all software items (paginated).
pub async fn list(params: ListParams<'_>) -> Result<()> {
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
    let resp = client.list_software_items(&list_params).await.context_to()?;

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
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
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
    if let Some(checked) = resp.last_checked_at {
        human.push_str(&format!(
            "Last Checked:    {}\n",
            checked
                .format(&Rfc3339)
                .unwrap_or_else(|_| checked.to_string())
        ));
    }
    human.push_str(&format!("Host Count:      {}\n", resp.host_count));
    human.push_str(&format!(
        "Created:         {}\n",
        resp.created_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| resp.created_at.to_string())
    ));
    human.push_str(&format!(
        "Updated:         {}\n",
        resp.updated_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| resp.updated_at.to_string())
    ));

    if !resp.hosts.is_empty() {
        human.push_str("\nAssigned Hosts:\n");
        human.push_str(&format!(
            "  {:<38} {:<20} {:<15} LINKED AT\n",
            "HOST ID", "HOSTNAME", "INSTALLED"
        ));
        for h in &resp.hosts {
            let linked = h
                .linked_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| h.linked_at.to_string());
            human.push_str(&format!(
                "  {:<38} {:<20} {:<15} {}\n",
                h.host_id,
                h.hostname,
                h.installed_version.as_deref().unwrap_or("-"),
                linked
            ));
        }
    }

    print_output(params.format, &human, &resp)
}
