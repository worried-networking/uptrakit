use crate::client::authenticated_client;
use crate::error::{CliError, Result};
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::services::{
    ListServicesQuery, MergeAgentRequest, ParseServiceStatusError, ParseServiceTypeError,
};

/// Parameters for listing services.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
    pub service_type: Option<&'a str>,
    pub status: Option<&'a str>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

/// List services with optional filters and pagination.
pub async fn list(params: ListParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure)?;
    let query = ListServicesQuery {
        r#type: params
            .service_type
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: ParseServiceTypeError| Report::new(CliError::Other(e.to_string())))?,
        status: params
            .status
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: ParseServiceStatusError| Report::new(CliError::Other(e.to_string())))?,
        page: params.page,
        per_page: params.per_page,
    };
    let resp = client.list_services(&query).await.context_to()?;

    let mut human = String::new();
    if resp.items.is_empty() {
        human.push_str("No services found.\n");
    } else {
        human.push_str(&format!(
            "{:<38} {:<12} {:<20} {:<25} {:<12} LAST SEEN\n",
            "ID", "TYPE", "HOSTNAME", "FRIENDLY NAME", "STATUS"
        ));
        for s in &resp.items {
            human.push_str(&format!(
                "{:<38} {:<12} {:<20} {:<25} {:<12} {}\n",
                s.id,
                s.service_type,
                s.hostname,
                s.friendly_name,
                s.status,
                s.last_seen_at.as_deref().unwrap_or("-")
            ));
        }
        human.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            resp.page, resp.total_pages, resp.total
        ));
    }

    print_output(params.format, &human, &resp)
}

/// Show details for a single service.
pub async fn show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;
    let resp = client.get_service(id).await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!("ID:            {}\n", resp.id));
    human.push_str(&format!("Type:          {}\n", resp.service_type));
    human.push_str(&format!("Hostname:      {}\n", resp.hostname));
    human.push_str(&format!("Friendly Name: {}\n", resp.friendly_name));
    if let Some(ref ip) = resp.ip_address {
        human.push_str(&format!("IP Address:    {}\n", ip));
    }
    human.push_str(&format!("Status:        {}\n", resp.status));
    if let Some(ref ver) = resp.client_version {
        human.push_str(&format!("Client Version: {}\n", ver));
    }
    if let Some(ref seen) = resp.last_seen_at {
        human.push_str(&format!("Last Seen:     {}\n", seen));
    }
    human.push_str(&format!("Created:       {}\n", resp.created_at));
    human.push_str(&format!("Updated:       {}\n", resp.updated_at));

    print_output(format, &human, &resp)
}

/// Approve a pending service.
pub async fn approve(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;
    let resp = client.approve_service(id).await.context_to()?;

    let human = format!(
        "Service {} approved.\nType:     {}\nHostname: {}\nStatus:   {}\n",
        resp.id, resp.service_type, resp.hostname, resp.status
    );

    print_output(format, &human, &resp)
}

/// Reject a pending service.
pub async fn reject(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;
    let resp = client.reject_service(id).await.context_to()?;

    let human = format!(
        "Service {} rejected.\nType:     {}\nHostname: {}\nStatus:   {}\n",
        resp.id, resp.service_type, resp.hostname, resp.status
    );

    print_output(format, &human, &resp)
}

/// Remove (deactivate) a service.
pub async fn remove(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;
    let resp = client.remove_service(id).await.context_to()?;

    let human = format!("{}\n", resp.message);

    print_output(format, &human, &resp)
}

/// Merge a source service into a target service.
pub async fn merge(
    target_id: &Uuid,
    source_id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;
    let req = MergeAgentRequest {
        source_id: *source_id,
    };
    let resp = client.merge_service(target_id, &req).await.context_to()?;

    let human = format!(
        "Service {} merged into {}.\nType:     {}\nHostname: {}\nStatus:   {}\n",
        source_id, resp.id, resp.service_type, resp.hostname, resp.status
    );

    print_output(format, &human, &resp)
}
