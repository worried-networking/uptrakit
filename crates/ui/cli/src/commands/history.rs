use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Subcommand)]
pub enum HistoryCommands {
    /// List update history
    List {
        /// Filter by host UUID
        #[arg(long)]
        host: Option<uptrakit_openapi_client::Uuid>,
        /// Filter by software item UUID
        #[arg(long)]
        software_item: Option<uptrakit_openapi_client::Uuid>,
        /// Filter by status (pending, in_progress, completed, failed)
        #[arg(long)]
        status: Option<String>,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show update history details
    Show {
        /// Update history UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Tail update output in real-time
    Tail {
        /// Update history UUID
        id: uptrakit_openapi_client::Uuid,
    },
}

pub async fn dispatch(command: HistoryCommands, ctx: &CliContext) -> Result<()> {
    match command {
        HistoryCommands::List {
            host,
            software_item,
            status,
            page,
            per_page,
        } => {
            let resp = list(ListParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                host_id: host,
                software_item_id: software_item,
                status: status.as_deref(),
                page,
                per_page,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        HistoryCommands::Show { id } => {
            let resp = show(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        HistoryCommands::Tail { id } => {
            let tail_result = super::tail::tail(super::tail::TailParams {
                update_history_id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
            })
            .await?;
            std::process::exit(tail_result.exit_code());
        }
    }
    Ok(())
}
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::update_history::{
    ParseUpdateStatusError, UpdateHistoryQuery, UpdateHistoryResponse,
};

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<UpdateHistoryResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No update history found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<20} {:<20} {:<12} TO VERSION\n",
            "ID", "HOST", "SOFTWARE", "STATUS"
        );
        for entry in &self.items {
            out.push_str(&format!(
                "{:<38} {:<20} {:<20} {:<12} {}\n",
                entry.id,
                entry.host_name,
                entry.software_item_name,
                entry.status.as_str(),
                entry.to_version
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for UpdateHistoryResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:            {}\n", self.id));
        out.push_str(&format!(
            "Host:          {} ({})\n",
            self.host_name, self.host_id
        ));
        out.push_str(&format!(
            "Software Item: {} ({})\n",
            self.software_item_name, self.software_item_id
        ));
        if let Some(ref from) = self.from_version {
            out.push_str(&format!("From Version:  {}\n", from));
        }
        out.push_str(&format!("To Version:    {}\n", self.to_version));
        out.push_str(&format!("Status:        {}\n", self.status.as_str()));
        out.push_str(&format!("Category:      {}\n", self.update_category));
        out.push_str(&format!(
            "Actor:         {} ({})\n",
            self.actor_type, self.actor_id
        ));
        out.push_str(&format!(
            "Started At:    {}\n",
            self.started_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.started_at.to_string())
        ));
        if let Some(completed) = self.completed_at {
            out.push_str(&format!(
                "Completed At:  {}\n",
                completed
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| completed.to_string())
            ));
        }
        if !self.output.is_empty() {
            out.push_str(&format!("\nOutput:\n{}\n", self.output));
        }
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for listing update history.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub status: Option<&'a str>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List update history (paginated, with optional filters).
pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<UpdateHistoryResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    let query = UpdateHistoryQuery {
        host_id: params.host_id,
        software_item_id: params.software_item_id,
        status: params
            .status
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: ParseUpdateStatusError| report!(CliError::Other(e.to_string())))?,
        page: params.page,
        per_page: params.per_page,
    };

    client.list_update_history(&query).await.context_to()
}

/// Show details for a single update history entry.
pub async fn show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<UpdateHistoryResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_update_history(id).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uptrakit_openapi_client::types::update_history::UpdateStatus;

    fn sample_entry() -> UpdateHistoryResponse {
        UpdateHistoryResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            host_id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                .parse::<Uuid>()
                .unwrap(),
            host_name: "server-1.local".to_string(),
            software_item_id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6"
                .parse::<Uuid>()
                .unwrap(),
            software_item_name: "Node.js".to_string(),
            from_version: Some("18.0.0".to_string()),
            to_version: "20.0.0".to_string(),
            status: UpdateStatus::Completed,
            actor_type: "user".to_string(),
            actor_id: "admin".to_string(),
            started_at: datetime!(2025-01-01 00:00:00 UTC),
            completed_at: Some(datetime!(2025-01-01 00:01:00 UTC)),
            output: "Success".to_string(),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            update_category: "security".to_string(),
            interactive: false,
            output_truncated: false,
            pre_update_protection_status: None,
            pre_update_protection_summary: None,
            recovery_hint: None,
        }
    }

    #[test]
    fn update_history_detail_human_output() {
        let entry = sample_entry();
        let s = entry.to_human_string();
        assert!(s.contains("server-1.local"), "host missing");
        assert!(s.contains("Node.js"), "software item missing");
        assert!(s.contains("20.0.0"), "to_version missing");
        assert!(s.contains("Success"), "output missing");
    }

    #[test]
    fn paginated_history_empty() {
        let resp: PaginatedResponse<UpdateHistoryResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No update history"));
    }

    #[test]
    fn paginated_history_has_rows() {
        let resp = PaginatedResponse {
            items: vec![sample_entry()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("Node.js"), "software item missing");
        assert!(s.contains("20.0.0"), "to_version missing");
    }
}
