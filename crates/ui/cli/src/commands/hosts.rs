use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use uptrakit_openapi_client::types::pagination::PaginationParams;

/// List all hosts (paginated).
pub async fn list(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    page: Option<u64>,
    per_page: Option<u64>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;
    let params = PaginationParams { page, per_page };
    let resp = client.list_hosts(&params).await.context_to()?;

    let mut human = String::new();
    if resp.items.is_empty() {
        human.push_str("No hosts found.\n");
    } else {
        human.push_str(&format!(
            "{:<38} {:<30} {:<20} AGENTS\n",
            "ID", "FRIENDLY NAME", "HOSTNAME"
        ));
        for h in &resp.items {
            human.push_str(&format!(
                "{:<38} {:<30} {:<20} {}\n",
                h.id,
                h.friendly_name,
                h.hostname,
                h.agents.len()
            ));
        }
        human.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            resp.page, resp.total_pages, resp.total
        ));
    }

    print_output(format, &human, &resp)
}

/// Show details for a single host.
pub async fn show(
    id: &str,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;
    let resp = client.get_host(id).await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!("ID:           {}\n", resp.id));
    human.push_str(&format!("Hostname:     {}\n", resp.hostname));
    human.push_str(&format!("Friendly Name:{}\n", resp.friendly_name));
    human.push_str(&format!("Machine ID:   {}\n", resp.machine_id));
    if let Some(ref os) = resp.os_type {
        human.push_str(&format!("OS:           {}\n", os));
    }
    if let Some(ref ver) = resp.os_version {
        human.push_str(&format!("OS Version:   {}\n", ver));
    }
    if let Some(ref arch) = resp.architecture {
        human.push_str(&format!("Architecture: {}\n", arch));
    }
    if let Some(ref ip) = resp.ip_address {
        human.push_str(&format!("IP Address:   {}\n", ip));
    }
    if let Some(ref seen) = resp.last_seen_at {
        human.push_str(&format!("Last Seen:    {}\n", seen));
    }
    human.push_str(&format!("Created:      {}\n", resp.created_at));
    if !resp.agents.is_empty() {
        human.push_str("Agents:\n");
        for a in &resp.agents {
            human.push_str(&format!(
                "  - {} ({}, {})\n",
                a.id, a.friendly_name, a.status
            ));
        }
    }

    print_output(format, &human, &resp)
}
