use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::scheduler::{
    ScheduledTaskResponse, TriggerScheduledTaskResponse,
};

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for Vec<ScheduledTaskResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No scheduled tasks found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<25} {:<15} {:<8} NEXT RUN\n",
            "ID", "TYPE", "CRON", "ENABLED"
        );
        for task in self {
            let next_run = task
                .next_run_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| task.next_run_at.to_string());
            out.push_str(&format!(
                "{:<38} {:<25} {:<15} {:<8} {}\n",
                task.id, task.task_type, task.cron_expression, task.enabled, next_run
            ));
        }
        out
    }
}

impl HumanOutput for ScheduledTaskResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:         {}\n", self.id));
        out.push_str(&format!("Type:       {}\n", self.task_type));
        out.push_str(&format!("Label:      {}\n", self.label));
        out.push_str(&format!("Cron:       {}\n", self.cron_expression));
        out.push_str(&format!("Enabled:    {}\n", self.enabled));
        out.push_str(&format!("Running:    {}\n", self.is_running));
        out.push_str(&format!("Run Count:  {}\n", self.run_count));
        if let Some(last) = self.last_run_at {
            out.push_str(&format!(
                "Last Run:   {}\n",
                last.format(&Rfc3339).unwrap_or_else(|_| last.to_string())
            ));
        }
        out.push_str(&format!(
            "Next Run:   {}\n",
            self.next_run_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.next_run_at.to_string())
        ));
        if let Some(ref err) = self.last_error {
            out.push_str(&format!("Last Error: {}\n", err));
        }
        out.push_str(&format!(
            "Created:    {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out.push_str(&format!(
            "Updated:    {}\n",
            self.updated_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.updated_at.to_string())
        ));
        out
    }
}

impl HumanOutput for TriggerScheduledTaskResponse {
    fn to_human_string(&self) -> String {
        if self.triggered {
            format!("Task triggered: {}\n", self.message)
        } else {
            format!("Could not trigger task: {}\n", self.message)
        }
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for listing scheduled tasks.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// Parameters for showing a single scheduled task.
pub struct ShowParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// Parameters for triggering a scheduled task.
pub struct TriggerParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List all scheduled tasks.
pub async fn list(params: ListParams<'_>) -> Result<Vec<ScheduledTaskResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.list_scheduled_tasks().await.context_to()
}

/// Show details for a single scheduled task.
pub async fn show(params: ShowParams<'_>) -> Result<ScheduledTaskResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.get_scheduled_task(params.id).await.context_to()
}

/// Trigger immediate execution of a scheduled task.
pub async fn trigger(params: TriggerParams<'_>) -> Result<TriggerScheduledTaskResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.trigger_scheduled_task(params.id).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uptrakit_openapi_client::Uuid;
    use uptrakit_openapi_client::types::scheduler::TASK_TYPE_FETCH_RELEASES;

    fn sample_task() -> ScheduledTaskResponse {
        ScheduledTaskResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            task_type: TASK_TYPE_FETCH_RELEASES.to_string(),
            label: "Fetch Latest Releases".to_string(),
            cron_expression: "0 * * * *".to_string(),
            enabled: true,
            task_config: None,
            is_running: false,
            run_count: 5,
            last_run_at: Some(datetime!(2025-01-01 00:00:00 UTC)),
            next_run_at: datetime!(2025-01-01 01:00:00 UTC),
            last_error: None,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
        }
    }

    #[test]
    fn scheduled_task_human_output_contains_key_fields() {
        let task = sample_task();
        let s = task.to_human_string();
        assert!(s.contains("fetch_releases"), "task_type missing");
        assert!(s.contains("Fetch Latest Releases"), "label missing");
        assert!(s.contains("0 * * * *"), "cron missing");
        assert!(s.contains("true"), "enabled missing");
    }

    #[test]
    fn vec_scheduled_tasks_empty() {
        let tasks: Vec<ScheduledTaskResponse> = vec![];
        assert!(tasks.to_human_string().contains("No scheduled tasks"));
    }

    #[test]
    fn vec_scheduled_tasks_has_header_and_row() {
        let tasks = vec![sample_task()];
        let s = tasks.to_human_string();
        assert!(s.contains("TYPE"), "header missing");
        assert!(s.contains("fetch_releases"), "task type missing");
    }

    #[test]
    fn trigger_response_triggered() {
        let resp = TriggerScheduledTaskResponse {
            triggered: true,
            message: "OK".to_string(),
        };
        assert!(resp.to_human_string().contains("triggered"));
    }

    #[test]
    fn trigger_response_not_triggered() {
        let resp = TriggerScheduledTaskResponse {
            triggered: false,
            message: "not found".to_string(),
        };
        let s = resp.to_human_string();
        assert!(s.contains("Could not trigger"));
        assert!(s.contains("not found"));
    }
}
