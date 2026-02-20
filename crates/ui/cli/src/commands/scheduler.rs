use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;

/// Parameters for listing scheduled tasks.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
}

/// Parameters for showing a single scheduled task.
pub struct ShowParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
}

/// Parameters for triggering a scheduled task.
pub struct TriggerParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
}

/// List all scheduled tasks.
pub async fn list(params: ListParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure)?;

    let resp = client.list_scheduled_tasks().await.context_to()?;

    let mut human = String::new();
    if resp.is_empty() {
        human.push_str("No scheduled tasks found.\n");
    } else {
        human.push_str(&format!(
            "{:<38} {:<25} {:<15} {:<8} NEXT RUN\n",
            "ID", "TYPE", "CRON", "ENABLED"
        ));
        for task in &resp {
            human.push_str(&format!(
                "{:<38} {:<25} {:<15} {:<8} {}\n",
                task.id, task.task_type, task.cron_expression, task.enabled, task.next_run_at
            ));
        }
    }

    print_output(params.format, &human, &resp)
}

/// Show details for a single scheduled task.
pub async fn show(params: ShowParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure)?;
    let resp = client.get_scheduled_task(params.id).await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!("ID:         {}\n", resp.id));
    human.push_str(&format!("Type:       {}\n", resp.task_type));
    human.push_str(&format!("Label:      {}\n", resp.label));
    human.push_str(&format!("Cron:       {}\n", resp.cron_expression));
    human.push_str(&format!("Enabled:    {}\n", resp.enabled));
    human.push_str(&format!("Running:    {}\n", resp.is_running));
    human.push_str(&format!("Run Count:  {}\n", resp.run_count));
    if let Some(ref last) = resp.last_run_at {
        human.push_str(&format!("Last Run:   {}\n", last));
    }
    human.push_str(&format!("Next Run:   {}\n", resp.next_run_at));
    if let Some(ref err) = resp.last_error {
        human.push_str(&format!("Last Error: {}\n", err));
    }
    human.push_str(&format!("Created:    {}\n", resp.created_at));
    human.push_str(&format!("Updated:    {}\n", resp.updated_at));

    print_output(params.format, &human, &resp)
}

/// Trigger immediate execution of a scheduled task.
pub async fn trigger(params: TriggerParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure)?;
    let resp = client
        .trigger_scheduled_task(params.id)
        .await
        .context_to()?;

    let human = if resp.triggered {
        format!("Task triggered: {}\n", resp.message)
    } else {
        format!("Could not trigger task: {}\n", resp.message)
    };

    print_output(params.format, &human, &resp)
}
