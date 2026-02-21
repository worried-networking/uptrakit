use crate::client::authenticated_client;
use crate::error::{CliError, Result};
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::update_history::{ParseUpdateStatusError, UpdateHistoryQuery};

/// Parameters for listing update history.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub status: Option<&'a str>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

/// List update history (paginated, with optional filters).
pub async fn list(params: ListParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure)?;

    let query = UpdateHistoryQuery {
        host_id: params.host_id,
        software_item_id: params.software_item_id,
        status: params
            .status
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: ParseUpdateStatusError| Report::new(CliError::Other(e.to_string())))?,
        page: params.page,
        per_page: params.per_page,
    };

    let resp = client.list_update_history(&query).await.context_to()?;

    let mut human = String::new();
    if resp.items.is_empty() {
        human.push_str("No update history found.\n");
    } else {
        human.push_str(&format!(
            "{:<38} {:<20} {:<20} {:<12} TO VERSION\n",
            "ID", "HOST", "SOFTWARE", "STATUS"
        ));
        for entry in &resp.items {
            human.push_str(&format!(
                "{:<38} {:<20} {:<20} {:<12} {}\n",
                entry.id,
                entry.host_name,
                entry.software_item_name,
                entry.status.as_str(),
                entry.to_version
            ));
        }
        human.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            resp.page, resp.total_pages, resp.total
        ));
    }

    print_output(params.format, &human, &resp)
}

/// Show details for a single update history entry.
pub async fn show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;
    let resp = client.get_update_history(id).await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!("ID:            {}\n", resp.id));
    human.push_str(&format!(
        "Host:          {} ({})\n",
        resp.host_name, resp.host_id
    ));
    human.push_str(&format!(
        "Software Item: {} ({})\n",
        resp.software_item_name, resp.software_item_id
    ));
    if let Some(ref from) = resp.from_version {
        human.push_str(&format!("From Version:  {}\n", from));
    }
    human.push_str(&format!("To Version:    {}\n", resp.to_version));
    human.push_str(&format!("Status:        {}\n", resp.status.as_str()));
    human.push_str(&format!("Initiated By:  {}\n", resp.initiated_by));
    human.push_str(&format!(
        "Started At:    {}\n",
        resp.started_at.format(&Rfc3339).unwrap_or_else(|_| resp.started_at.to_string())
    ));
    if let Some(completed) = resp.completed_at {
        human.push_str(&format!(
            "Completed At:  {}\n",
            completed.format(&Rfc3339).unwrap_or_else(|_| completed.to_string())
        ));
    }
    if !resp.output.is_empty() {
        human.push_str(&format!("\nOutput:\n{}\n", resp.output));
    }

    print_output(format, &human, &resp)
}
