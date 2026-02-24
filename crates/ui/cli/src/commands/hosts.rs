use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::autodiscovery::{
    DiscardDiscoveredResponse, TriggerDiscoveryResponse,
};
use uptrakit_openapi_client::types::hosts::{HostMessageResponse, HostResponse, UpdateHostRequest};
use uptrakit_openapi_client::types::pagination::{PaginatedResponse, PaginationParams};

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<HostResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No hosts found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<30} {:<20} AGENTS\n",
            "ID", "FRIENDLY NAME", "HOSTNAME"
        );
        for h in &self.items {
            out.push_str(&format!(
                "{:<38} {:<30} {:<20} {}\n",
                h.id,
                h.friendly_name,
                h.hostname,
                h.agents.len()
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for HostResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:           {}\n", self.id));
        out.push_str(&format!("Hostname:     {}\n", self.hostname));
        out.push_str(&format!("Friendly Name: {}\n", self.friendly_name));
        out.push_str(&format!("Machine ID:   {}\n", self.machine_id));
        if let Some(ref os) = self.os_type {
            out.push_str(&format!("OS:           {}\n", os));
        }
        if let Some(ref ver) = self.os_version {
            out.push_str(&format!("OS Version:   {}\n", ver));
        }
        if let Some(ref arch) = self.architecture {
            out.push_str(&format!("Architecture: {}\n", arch));
        }
        if let Some(ref ip) = self.ip_address {
            out.push_str(&format!("IP Address:   {}\n", ip));
        }
        if let Some(seen) = self.last_seen_at {
            out.push_str(&format!(
                "Last Seen:    {}\n",
                seen.format(&Rfc3339).unwrap_or_else(|_| seen.to_string())
            ));
        }
        out.push_str(&format!(
            "Created:      {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        if !self.agents.is_empty() {
            out.push_str("Agents:\n");
            for a in &self.agents {
                out.push_str(&format!(
                    "  - {} ({}, {})\n",
                    a.id, a.friendly_name, a.status
                ));
            }
        }
        out
    }
}

impl HumanOutput for TriggerDiscoveryResponse {
    fn to_human_string(&self) -> String {
        format!(
            "{} (providers queued: {})\n",
            self.message, self.providers_queued
        )
    }
}

impl HumanOutput for DiscardDiscoveredResponse {
    fn to_human_string(&self) -> String {
        format!("Discarded {} discovered item(s).\n", self.discarded_count)
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for listing hosts.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

/// Parameters for showing a single host.
pub struct ShowParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct UpdateParams<'a> {
    pub id: &'a Uuid,
    pub friendly_name: Option<String>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct DeactivateParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct DiscoverParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct DiscardDiscoveredParams<'a> {
    pub id: &'a Uuid,
    pub provider_config_id: Option<&'a Uuid>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List all hosts (paginated).
pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<HostResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let pagination = PaginationParams {
        page: params.page,
        per_page: params.per_page,
    };
    client.list_hosts(&pagination).await.context_to()
}

/// Show details for a single host.
pub async fn show(params: ShowParams<'_>) -> Result<HostResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.get_host(params.id).await.context_to()
}

pub async fn update(params: UpdateParams<'_>) -> Result<HostResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateHostRequest {
        friendly_name: params.friendly_name,
    };
    client.update_host(params.id, &req).await.context_to()
}

pub async fn deactivate(params: DeactivateParams<'_>) -> Result<HostMessageResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.deactivate_host(params.id).await.context_to()
}

pub async fn discover(params: DiscoverParams<'_>) -> Result<TriggerDiscoveryResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.discover_host(params.id).await.context_to()
}

pub async fn discard_discovered(
    params: DiscardDiscoveredParams<'_>,
) -> Result<DiscardDiscoveredResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .discard_host_discovered(params.id, params.provider_config_id)
        .await
        .context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uptrakit_openapi_client::types::autodiscovery::{
        DiscardDiscoveredResponse, TriggerDiscoveryResponse,
    };
    use uptrakit_openapi_client::types::hosts::HostAgentSummary;

    fn sample_host() -> HostResponse {
        HostResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            machine_id: "machine-001".to_string(),
            hostname: "server-1.local".to_string(),
            friendly_name: "Production Server".to_string(),
            os_type: Some("linux".to_string()),
            os_version: Some("Ubuntu 22.04".to_string()),
            architecture: Some("x86_64".to_string()),
            ip_address: Some("192.168.1.100".to_string()),
            last_seen_at: Some(datetime!(2025-01-01 00:00:00 UTC)),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
            agents: vec![],
        }
    }

    #[test]
    fn host_detail_human_output_contains_key_fields() {
        let host = sample_host();
        let s = host.to_human_string();
        assert!(s.contains("server-1.local"), "hostname missing");
        assert!(s.contains("Production Server"), "friendly name missing");
        assert!(s.contains("machine-001"), "machine id missing");
        assert!(s.contains("linux"), "os type missing");
        assert!(s.contains("192.168.1.100"), "ip missing");
    }

    #[test]
    fn host_detail_with_agents() {
        let mut host = sample_host();
        host.agents = vec![HostAgentSummary {
            id: "d1d2d3d4-e1e2-f1f2-a1a2-b1b2b3b4b5b6"
                .parse::<Uuid>()
                .unwrap(),
            friendly_name: "agent-1".to_string(),
            status: "approved".parse().unwrap(),
        }];
        let s = host.to_human_string();
        assert!(s.contains("agent-1"), "agent name missing");
    }

    #[test]
    fn paginated_hosts_empty() {
        let resp: PaginatedResponse<HostResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No hosts found"));
    }

    #[test]
    fn paginated_hosts_has_header_and_row() {
        let resp = PaginatedResponse {
            items: vec![sample_host()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("FRIENDLY NAME"), "header missing");
        assert!(s.contains("Production Server"), "host row missing");
    }

    #[test]
    fn host_message_response_human_output() {
        let resp = HostMessageResponse {
            message: "Host deactivated.".to_string(),
        };
        let s = resp.to_human_string();
        assert!(s.contains("Host deactivated"));
    }

    #[test]
    fn trigger_discovery_response_human_output() {
        let resp = TriggerDiscoveryResponse {
            providers_queued: 3,
            message: "Discovery queued".to_string(),
        };
        let s = resp.to_human_string();
        assert!(s.contains("Discovery queued"));
        assert!(s.contains("3"));
    }

    #[test]
    fn discard_discovered_response_human_output() {
        let resp = DiscardDiscoveredResponse { discarded_count: 5 };
        let s = resp.to_human_string();
        assert!(s.contains("5"));
    }
}
