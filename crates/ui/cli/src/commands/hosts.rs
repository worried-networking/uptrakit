use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::pagination::PaginationParams;

/// Parameters for listing hosts.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

/// Parameters for showing a single host.
pub struct ShowParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// List all hosts (paginated).
pub async fn list(params: ListParams<'_>) -> Result<()> {
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
    let resp = client.list_hosts(&pagination).await.context_to()?;

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

    print_output(params.format, &human, &resp)
}

/// Show details for a single host.
pub async fn show(params: ShowParams<'_>) -> Result<()> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let resp = client.get_host(params.id).await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!("ID:           {}\n", resp.id));
    human.push_str(&format!("Hostname:     {}\n", resp.hostname));
    human.push_str(&format!("Friendly Name: {}\n", resp.friendly_name));
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
    if let Some(seen) = resp.last_seen_at {
        human.push_str(&format!(
            "Last Seen:    {}\n",
            seen.format(&Rfc3339).unwrap_or_else(|_| seen.to_string())
        ));
    }
    human.push_str(&format!(
        "Created:      {}\n",
        resp.created_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| resp.created_at.to_string())
    ));
    if !resp.agents.is_empty() {
        human.push_str("Agents:\n");
        for a in &resp.agents {
            human.push_str(&format!(
                "  - {} ({}, {})\n",
                a.id, a.friendly_name, a.status
            ));
        }
    }

    print_output(params.format, &human, &resp)
}
