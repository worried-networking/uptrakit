use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::scheduler::TriggerScheduledTaskResponse;
use uptrakit_openapi_client::types::software_items::TriggerVersionCheckResponse;

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for TriggerVersionCheckResponse {
    fn to_human_string(&self) -> String {
        format!(
            "Agents notified: {}\n{}\n",
            self.agents_notified, self.message
        )
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for triggering a bulk version check.
pub struct AllParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// Parameters for triggering a version check on a specific software item.
pub struct ItemParams<'a> {
    pub item_id: &'a Uuid,
    pub host_id: Option<&'a Uuid>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Trigger a bulk version check by finding and triggering the `version_check` scheduler task.
pub async fn all(params: AllParams<'_>) -> Result<TriggerScheduledTaskResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    // Find the version_check scheduler task
    let tasks = client.list_scheduled_tasks().await.context_to()?;
    let task = tasks.iter().find(|t| t.task_type == "version_check");

    let task = match task {
        Some(t) => t,
        None => {
            return Ok(TriggerScheduledTaskResponse {
                triggered: false,
                message: "No version_check scheduler task found".to_string(),
            });
        }
    };

    client.trigger_scheduled_task(&task.id).await.context_to()
}

/// Trigger a version check for a specific software item.
pub async fn item(params: ItemParams<'_>) -> Result<TriggerVersionCheckResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    match params.host_id {
        Some(hid) => client
            .check_versions_host(params.item_id, hid)
            .await
            .context_to(),
        None => client.check_versions(params.item_id).await.context_to(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_version_check_human_output() {
        let resp = TriggerVersionCheckResponse {
            agents_notified: 3,
            message: "Check dispatched".to_string(),
        };
        let s = resp.to_human_string();
        assert!(s.contains("3"), "agents_notified missing");
        assert!(s.contains("Check dispatched"), "message missing");
    }
}
