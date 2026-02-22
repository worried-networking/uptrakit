use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::scheduler::TriggerScheduledTaskResponse;

/// Parameters for triggering a bulk version check.
pub struct AllParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// Parameters for triggering a version check on a specific software item.
pub struct ItemParams<'a> {
    pub item_id: &'a Uuid,
    pub host_id: Option<&'a Uuid>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// Trigger a bulk version check by finding and triggering the `version_check` scheduler task.
pub async fn all(params: AllParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;

    // Find the version_check scheduler task
    let tasks = client.list_scheduled_tasks().await.context_to()?;
    let task = tasks.iter().find(|t| t.task_type == "version_check");

    let task = match task {
        Some(t) => t,
        None => {
            let human = "No version_check scheduler task found.\n".to_string();
            let resp = TriggerScheduledTaskResponse {
                triggered: false,
                message: "No version_check scheduler task found".to_string(),
            };
            return print_output(params.format, &human, &resp);
        }
    };

    let resp = client.trigger_scheduled_task(&task.id).await.context_to()?;

    let human = if resp.triggered {
        format!("Version check triggered: {}\n", resp.message)
    } else {
        format!("Could not trigger version check: {}\n", resp.message)
    };

    print_output(params.format, &human, &resp)
}

/// Trigger a version check for a specific software item.
pub async fn item(params: ItemParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;

    let resp = match params.host_id {
        Some(hid) => client
            .check_versions_host(params.item_id, hid)
            .await
            .context_to()?,
        None => client.check_versions(params.item_id).await.context_to()?,
    };

    let human = format!(
        "Agents notified: {}\n{}\n",
        resp.agents_notified, resp.message
    );

    print_output(params.format, &human, &resp)
}
