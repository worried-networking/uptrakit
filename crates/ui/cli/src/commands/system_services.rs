use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::services::MessageResponse;
use uptrakit_openapi_client::types::system_services::{
    ListSystemServicesQuery, ParseServiceStatusError, SystemServiceResponse,
    UpdateSystemServiceRequest,
};

#[derive(Debug, Subcommand)]
pub enum SystemServicesCommands {
    /// List all system services
    List {
        /// Filter by capability (mqtt_bridge, scheduler)
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
    /// Show system service details
    Show {
        /// System service UUID
        id: Uuid,
    },
    /// Approve a pending system service
    Approve {
        /// System service UUID
        id: Uuid,
    },
    /// Reject a pending system service
    Reject {
        /// System service UUID
        id: Uuid,
    },
    /// Remove (deactivate) a system service
    Remove {
        /// System service UUID
        id: Uuid,
    },
    /// Update a system service's settings
    Update {
        /// System service UUID
        id: Uuid,
        /// Custom ping interval in seconds (0 to clear override)
        #[arg(long)]
        ping_interval: Option<u32>,
        /// Per-service certificate lifetime in hours (0 to clear override)
        #[arg(long)]
        cert_lifetime_hours: Option<u32>,
    },
    /// Perform a batch action on multiple system services
    Batch {
        /// Action to perform (e.g. approve, reject, deactivate, delete)
        action: String,
        /// System service UUIDs (space-separated)
        ids: Vec<Uuid>,
    },
}

pub async fn dispatch(command: SystemServicesCommands, ctx: &CliContext) -> Result<()> {
    match command {
        SystemServicesCommands::List {
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
        SystemServicesCommands::Show { id } => {
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
        SystemServicesCommands::Approve { id } => {
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
        SystemServicesCommands::Reject { id } => {
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
        SystemServicesCommands::Remove { id } => {
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
        SystemServicesCommands::Update {
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
        SystemServicesCommands::Batch { action, ids } => {
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

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<SystemServiceResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No system services found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<20} {:<25} {:<12} LAST SEEN\n",
            "ID", "HOSTNAME", "FRIENDLY NAME", "STATUS"
        );
        for s in &self.items {
            let last_seen = s
                .last_seen_at
                .as_ref()
                .map(|dt| dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string()));
            out.push_str(&format!(
                "{:<38} {:<20} {:<25} {:<12} {}\n",
                s.id,
                s.hostname,
                s.friendly_name,
                s.status,
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

impl HumanOutput for SystemServiceResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:            {}\n", self.id));
        out.push_str(&format!("Hostname:      {}\n", self.hostname));
        out.push_str(&format!("Friendly Name: {}\n", self.friendly_name));
        if !self.capabilities.is_empty() {
            out.push_str(&format!(
                "Capabilities:  {}\n",
                self.capabilities.join(", ")
            ));
        }
        if let Some(ref ip) = self.ip_address {
            out.push_str(&format!("IP Address:    {}\n", ip));
        }
        out.push_str(&format!("Status:        {}\n", self.status));
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

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for listing system services.
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

/// List system services with optional filters and pagination.
pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<SystemServiceResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let query = ListSystemServicesQuery {
        capability: params.capability.map(|s| s.to_string()),
        status: params
            .status
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: ParseServiceStatusError| report!(CliError::Other(e.to_string())))?,
        page: params.page,
        per_page: params.per_page,
    };
    client.list_system_services(&query).await.context_to()
}

/// Show details for a single system service.
pub async fn show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<SystemServiceResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_system_service(id).await.context_to()
}

/// Update a system service's configurable settings.
pub async fn update(
    id: &Uuid,
    ping_interval: Option<u32>,
    cert_lifetime_hours: Option<u32>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<SystemServiceResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateSystemServiceRequest {
        ping_interval_seconds: ping_interval,
        cert_lifetime_hours,
    };
    client.update_system_service(id, &req).await.context_to()
}

/// Approve a pending system service.
pub async fn approve(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<SystemServiceResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.approve_system_service(id).await.context_to()
}

/// Reject a pending system service.
pub async fn reject(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<SystemServiceResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.reject_system_service(id).await.context_to()
}

/// Remove (deactivate) a system service.
pub async fn remove(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MessageResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.remove_system_service(id).await.context_to()?;
    Ok(MessageResponse {
        message: "System service deactivated.".to_string(),
    })
}

/// Perform a batch action on multiple system services.
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
    client.batch_system_services(&req).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_system_service() -> SystemServiceResponse {
        SystemServiceResponse {
            id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                .parse::<Uuid>()
                .unwrap(),
            capabilities: vec!["mqtt_bridge".to_string(), "graceful_shutdown".to_string()],
            hostname: "mqtt-bridge.local".to_string(),
            friendly_name: "MQTT Bridge".to_string(),
            ip_address: None,
            status: "approved".parse().unwrap(),
            client_version: Some("2.0.0".to_string()),
            last_seen_at: Some(datetime!(2025-01-01 00:00:00 UTC)),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
            ping_interval_seconds: None,
            cert_lifetime_hours: None,
        }
    }

    #[test]
    fn system_service_detail_human_output() {
        let svc = sample_system_service();
        let s = svc.to_human_string();
        assert!(s.contains("mqtt-bridge.local"), "hostname missing");
        assert!(s.contains("MQTT Bridge"), "friendly name missing");
        assert!(s.contains("approved"), "status missing");
        assert!(s.contains("2.0.0"), "client version missing");
        assert!(s.contains("mqtt_bridge"), "capabilities missing");
    }

    #[test]
    fn paginated_system_services_empty() {
        let resp: PaginatedResponse<SystemServiceResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No system services found"));
    }

    #[test]
    fn paginated_system_services_has_rows() {
        let resp = PaginatedResponse {
            items: vec![sample_system_service()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("mqtt-bridge.local"), "hostname missing");
        assert!(s.contains("MQTT Bridge"), "friendly name missing");
    }

    #[test]
    fn remove_message_response_human_output() {
        let resp = MessageResponse {
            message: "System service deactivated.".to_string(),
        };
        assert!(
            resp.to_human_string()
                .contains("System service deactivated")
        );
    }
}
