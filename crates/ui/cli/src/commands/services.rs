use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum ServicesCommands {
    /// List all services
    List {
        /// Filter by capability (software_discovery, update_tracking, ssh_remote)
        #[arg(long)]
        capability: Option<String>,
        /// Filter by status (pending, approved, rejected, deactivated)
        #[arg(long)]
        status: Option<String>,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show service details
    Show {
        /// Service UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Approve a pending service
    Approve {
        /// Service UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Reject a pending service
    Reject {
        /// Service UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Remove (deactivate) a service
    Remove {
        /// Service UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Update a service's settings
    Update {
        /// Service UUID
        id: uptrakit_openapi_client::Uuid,
        /// Custom ping interval in seconds (0 to clear override)
        #[arg(long)]
        ping_interval: Option<u32>,
        /// Per-service certificate lifetime in hours (0 to clear override)
        #[arg(long)]
        cert_lifetime_hours: Option<u32>,
    },
    /// Merge a source service into a target service
    Merge {
        /// Target service UUID (approved)
        target_id: uptrakit_openapi_client::Uuid,
        /// Source service UUID (pending)
        source_id: uptrakit_openapi_client::Uuid,
    },
    /// Enable or disable the update freeze on a connected service
    UpdateFreeze {
        /// Service UUID
        id: uptrakit_openapi_client::Uuid,
        /// Enable the freeze (blocks updates on the agent)
        #[arg(long, group = "freeze_action")]
        enable: bool,
        /// Disable the freeze (allows updates on the agent)
        #[arg(long, group = "freeze_action")]
        disable: bool,
        /// Optional reason for the freeze
        #[arg(long)]
        reason: Option<String>,
    },
    /// Perform a batch action on multiple services
    Batch {
        /// Action to perform (e.g. approve, reject, deactivate, delete)
        action: String,
        /// Service UUIDs (space-separated)
        ids: Vec<uptrakit_openapi_client::Uuid>,
    },
}

pub async fn dispatch(command: ServicesCommands, ctx: &CliContext) -> Result<()> {
    match command {
        ServicesCommands::List {
            capability,
            status,
            page,
            per_page,
        } => {
            let resp = list(ListParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                capability: capability.as_deref(),
                status: status.as_deref(),
                page,
                per_page,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ServicesCommands::Show { id } => {
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
        ServicesCommands::Approve { id } => {
            let resp = approve(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ServicesCommands::Reject { id } => {
            let resp = reject(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ServicesCommands::Remove { id } => {
            let resp = remove(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ServicesCommands::Update {
            id,
            ping_interval,
            cert_lifetime_hours,
        } => {
            let resp = update(
                &id,
                ping_interval,
                cert_lifetime_hours,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ServicesCommands::Merge {
            target_id,
            source_id,
        } => {
            let resp = merge(
                &target_id,
                &source_id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ServicesCommands::UpdateFreeze {
            id,
            enable,
            disable: _,
            reason,
        } => {
            let resp = update_freeze(
                &id,
                enable,
                reason.as_deref(),
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ServicesCommands::Batch { action, ids } => {
            let resp = batch(
                &action,
                &ids,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::services::{
    ListServicesQuery, MergeAgentRequest, MessageResponse, ParseServiceStatusError,
    ServiceResponse, SetUpdateFreezeRequest, UpdateServiceRequest,
};

// ── Local wrapper types ───────────────────────────────────────────────────────

/// Output for `services merge` — carries the source service ID alongside the
/// merged service response. JSON output includes `source_id` as a top-level
/// field via `#[serde(flatten)]`.
#[derive(Debug, Serialize)]
pub struct MergeServiceOutput {
    pub source_id: Uuid,
    #[serde(flatten)]
    pub inner: ServiceResponse,
}

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<ServiceResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No services found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<12} {:<20} {:<25} {:<12} LAST SEEN\n",
            "ID", "LABEL", "HOSTNAME", "FRIENDLY NAME", "STATUS"
        );
        for s in &self.items {
            let last_seen = s
                .last_seen_at
                .as_ref()
                .map(|dt| dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string()));
            let status = format_status_with_flags(
                &s.status.to_string(),
                s.is_embedded,
                s.yielded_to.as_deref(),
            );
            out.push_str(&format!(
                "{:<38} {:<12} {:<20} {:<25} {:<12} {}\n",
                s.id,
                s.service_label,
                s.hostname,
                s.friendly_name,
                status,
                last_seen.as_deref().unwrap_or("-")
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for ServiceResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:            {}\n", self.id));
        out.push_str(&format!("Label:         {}\n", self.service_label));
        out.push_str(&format!("Hostname:      {}\n", self.hostname));
        out.push_str(&format!("Friendly Name: {}\n", self.friendly_name));
        if let Some(ref ip) = self.ip_address {
            out.push_str(&format!("IP Address:    {}\n", ip));
        }
        out.push_str(&format!("Status:        {}\n", self.status));
        if self.is_embedded {
            out.push_str("Embedded:      Yes\n");
        }
        if let Some(ids) = self.yielded_to.as_ref().filter(|v| !v.is_empty()) {
            let id_list: Vec<String> = ids.iter().map(ToString::to_string).collect();
            out.push_str(&format!("Yielded To:    {}\n", id_list.join(", ")));
        }
        if let Some(ref ver) = self.client_version {
            out.push_str(&format!("Client Version: {}\n", ver));
        }
        if let Some(seen) = self.last_seen_at {
            out.push_str(&format!(
                "Last Seen:     {}\n",
                seen.format(&Rfc3339).unwrap_or_else(|_| seen.to_string())
            ));
        }
        if let Some(ping) = self.ping_interval_seconds {
            out.push_str(&format!("Ping Interval: {}s\n", ping));
        }
        if let Some(h) = self.cert_lifetime_hours {
            out.push_str(&format!("Cert Lifetime: {}h\n", h));
        }
        out.push_str(&format!(
            "Created:       {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out.push_str(&format!(
            "Updated:       {}\n",
            self.updated_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.updated_at.to_string())
        ));
        out
    }
}

impl HumanOutput for MergeServiceOutput {
    fn to_human_string(&self) -> String {
        let mut out = format!(
            "Service {} merged into {}.\n",
            self.source_id, self.inner.id
        );
        out.push_str(&format!("Label:    {}\n", self.inner.service_label));
        out.push_str(&format!("Hostname: {}\n", self.inner.hostname));
        out.push_str(&format!("Status:   {}\n", self.inner.status));
        out
    }
}

impl HumanOutput for MessageResponse {
    fn to_human_string(&self) -> String {
        format!("{}\n", self.message)
    }
}

impl HumanOutput for BatchActionResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        if !self.succeeded.is_empty() {
            out.push_str(&format!("Succeeded ({}):\n", self.succeeded.len()));
            for s in &self.succeeded {
                out.push_str(&format!("  {}\n", s.id));
            }
        }
        if !self.failed.is_empty() {
            out.push_str(&format!("Failed ({}):\n", self.failed.len()));
            for f in &self.failed {
                out.push_str(&format!("  {} — {}\n", f.id, f.error));
            }
        }
        if self.succeeded.is_empty() && self.failed.is_empty() {
            out.push_str("No items processed.\n");
        }
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for listing services.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub capability: Option<&'a str>,
    pub status: Option<&'a str>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List services with optional filters and pagination.
pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<ServiceResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let query = ListServicesQuery {
        capability: params.capability.map(|s| s.to_string()),
        status: params
            .status
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: ParseServiceStatusError| report!(CliError::Other(e.to_string())))?,
        page: params.page,
        per_page: params.per_page,
    };
    client.list_services(&query).await.context_to()
}

/// Show details for a single service.
pub async fn show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<ServiceResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_service(id).await.context_to()
}

/// Update a service's configurable settings.
pub async fn update(
    id: &Uuid,
    ping_interval: Option<u32>,
    cert_lifetime_hours: Option<u32>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<ServiceResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateServiceRequest {
        ping_interval_seconds: ping_interval,
        cert_lifetime_hours,
    };
    client.update_service(id, &req).await.context_to()
}

/// Approve a pending service.
pub async fn approve(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<ServiceResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.approve_service(id).await.context_to()
}

/// Reject a pending service.
pub async fn reject(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<ServiceResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.reject_service(id).await.context_to()
}

/// Remove (deactivate) a service.
pub async fn remove(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MessageResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.remove_service(id).await.context_to()?;
    Ok(MessageResponse {
        message: "Service deactivated.".to_string(),
    })
}

/// Merge a source service into a target service.
pub async fn merge(
    target_id: &Uuid,
    source_id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MergeServiceOutput> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = MergeAgentRequest {
        source_id: *source_id,
    };
    let inner = client.merge_service(target_id, &req).await.context_to()?;
    Ok(MergeServiceOutput {
        source_id: *source_id,
        inner,
    })
}

/// Perform a batch action on multiple services.
pub async fn batch(
    action: &str,
    ids: &[Uuid],
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<BatchActionResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = BatchActionRequest {
        action: action.to_string(),
        ids: ids.to_vec(),
    };
    client.batch_services(&req).await.context_to()
}

/// Enable or disable the update freeze on a connected service.
pub async fn update_freeze(
    id: &Uuid,
    enabled: bool,
    reason: Option<&str>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MessageResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = SetUpdateFreezeRequest {
        enabled,
        reason: reason.map(|s| s.to_string()),
    };
    client.set_update_freeze(id, &req).await.context_to()
}

/// Format a status string with embedded/yielded flags for list output.
fn format_status_with_flags(
    status: &str,
    is_embedded: bool,
    yielded_to: Option<&[Uuid]>,
) -> String {
    let has_yields = yielded_to.is_some_and(|ids| !ids.is_empty());
    match (is_embedded, has_yields) {
        (true, true) => format!("{status} (yielded)"),
        (true, false) => format!("{status} (embedded)"),
        _ => status.to_string(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_service() -> ServiceResponse {
        ServiceResponse {
            id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                .parse::<Uuid>()
                .unwrap(),
            capabilities: vec![
                "graceful_shutdown".to_string(),
                "software_discovery".to_string(),
                "update_hooks".to_string(),
            ],
            service_label: "Agent".to_string(),
            hostname: "agent-host.local".to_string(),
            friendly_name: "Test Agent".to_string(),
            is_embedded: false,
            ip_address: None,
            status: "approved".parse().unwrap(),
            client_version: Some("1.0.0".to_string()),
            last_seen_at: Some(datetime!(2025-01-01 00:00:00 UTC)),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
            ping_interval_seconds: None,
            cert_lifetime_hours: None,
            yielded_to: None,
        }
    }

    #[test]
    fn service_detail_human_output() {
        let svc = sample_service();
        let s = svc.to_human_string();
        assert!(s.contains("agent-host.local"), "hostname missing");
        assert!(s.contains("Test Agent"), "friendly name missing");
        assert!(s.contains("approved"), "status missing");
        assert!(s.contains("1.0.0"), "client version missing");
    }

    #[test]
    fn paginated_services_empty() {
        let resp: PaginatedResponse<ServiceResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No services found"));
    }

    #[test]
    fn paginated_services_has_rows() {
        let resp = PaginatedResponse {
            items: vec![sample_service()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("agent-host.local"), "hostname missing");
    }

    #[test]
    fn merge_output_human_output() {
        let source_id: Uuid = "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6".parse().unwrap();
        let out = MergeServiceOutput {
            source_id,
            inner: sample_service(),
        };
        let s = out.to_human_string();
        assert!(
            s.contains("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"),
            "source_id missing"
        );
        assert!(s.contains("merged"), "merged word missing");
    }

    #[test]
    fn message_response_human_output() {
        let resp = MessageResponse {
            message: "Service removed.".to_string(),
        };
        assert!(resp.to_human_string().contains("Service removed"));
    }

    #[test]
    fn embedded_service_show_output() {
        let mut svc = sample_service();
        svc.is_embedded = true;
        let s = svc.to_human_string();
        assert!(s.contains("Embedded:      Yes"), "embedded flag missing");
        assert!(!s.contains("Yielded To:"), "yielded_to should be absent");
    }

    #[test]
    fn yielded_service_show_output() {
        let mut svc = sample_service();
        svc.is_embedded = true;
        let id1: Uuid = "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6".parse().unwrap();
        svc.yielded_to = Some(vec![id1]);
        let s = svc.to_human_string();
        assert!(s.contains("Embedded:      Yes"), "embedded flag missing");
        assert!(
            s.contains("Yielded To:    a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"),
            "yielded_to missing"
        );
    }

    #[test]
    fn embedded_service_list_status_marker() {
        let mut svc = sample_service();
        svc.is_embedded = true;
        let resp = PaginatedResponse {
            items: vec![svc],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("(embedded)"), "embedded marker missing in list");
    }

    #[test]
    fn yielded_service_list_status_marker() {
        let mut svc = sample_service();
        svc.is_embedded = true;
        let id1: Uuid = "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6".parse().unwrap();
        svc.yielded_to = Some(vec![id1]);
        let resp = PaginatedResponse {
            items: vec![svc],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("(yielded)"), "yielded marker missing in list");
    }
}
