use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use uptrakit_web_api_types::scheduler::{ScheduledTaskResponse, TriggerScheduledTaskResponse};
use uptrakit_web_api_types::software_items::TriggerVersionCheckResponse;

/// Trigger a bulk version check by finding and triggering the `version_check` scheduler task.
pub async fn all(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;

    // Find the version_check scheduler task
    let tasks: Vec<ScheduledTaskResponse> = client.get("/api/v1/scheduler/tasks").await?;
    let task = tasks.iter().find(|t| t.task_type == "version_check");

    let task = match task {
        Some(t) => t,
        None => {
            let human = "No version_check scheduler task found.\n".to_string();
            let resp = TriggerScheduledTaskResponse {
                triggered: false,
                message: "No version_check scheduler task found".to_string(),
            };
            return print_output(format, &human, &resp);
        }
    };

    let path = format!("/api/v1/scheduler/tasks/{}/trigger", task.id);
    let resp: TriggerScheduledTaskResponse = client.post_empty(&path).await?;

    let human = if resp.triggered {
        format!("Version check triggered: {}\n", resp.message)
    } else {
        format!("Could not trigger version check: {}\n", resp.message)
    };

    print_output(format, &human, &resp)
}

/// Trigger an installed version check for a specific software item.
pub async fn installed(
    item_id: &str,
    host_id: Option<&str>,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    check_item(item_id, host_id, server, token, format, insecure).await
}

/// Trigger an available version check for a specific software item.
pub async fn available(
    item_id: &str,
    host_id: Option<&str>,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    check_item(item_id, host_id, server, token, format, insecure).await
}

/// Shared implementation: both `installed` and `available` use the same API endpoint.
async fn check_item(
    item_id: &str,
    host_id: Option<&str>,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure)?;

    let path = match host_id {
        Some(hid) => format!("/api/v1/software-items/{item_id}/hosts/{hid}/check-versions"),
        None => format!("/api/v1/software-items/{item_id}/check-versions"),
    };

    let resp: TriggerVersionCheckResponse = client.post_empty(&path).await?;

    let human = format!(
        "Agents notified: {}\n{}\n",
        resp.agents_notified, resp.message
    );

    print_output(format, &human, &resp)
}
