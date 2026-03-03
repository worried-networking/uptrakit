use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::audit_logs::{
    AuditLogListParams, AuditLogResponse, SystemAuditLogResponse,
};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;

// ── Human output ────────────────────────────────────────────────────────────

fn format_occurred_at(dt: &time::OffsetDateTime) -> String {
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}

impl HumanOutput for PaginatedResponse<AuditLogResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No audit log entries found.\n".to_string();
        }
        let mut out = format!(
            "{:<27} {:<7} {:<45} {:<6} {:<10} {:<10} {}\n",
            "OCCURRED_AT", "METHOD", "PATH", "STATUS", "ACTOR_TYPE", "AUTH", "IP"
        );
        for entry in &self.items {
            out.push_str(&format!(
                "{:<27} {:<7} {:<45} {:<6} {:<10} {:<10} {}\n",
                format_occurred_at(&entry.occurred_at),
                entry.http_method,
                entry.http_path,
                entry.http_status,
                entry.actor_type,
                entry.auth_method,
                entry.client_ip.as_deref().unwrap_or("-"),
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for PaginatedResponse<SystemAuditLogResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No system audit log entries found.\n".to_string();
        }
        let mut out = format!(
            "{:<27} {:<7} {:<45} {:<6} {:<10} {:<10} {}\n",
            "OCCURRED_AT", "METHOD", "PATH", "STATUS", "ACTOR_TYPE", "AUTH", "IP"
        );
        for entry in &self.items {
            out.push_str(&format!(
                "{:<27} {:<7} {:<45} {:<6} {:<10} {:<10} {}\n",
                format_occurred_at(&entry.occurred_at),
                entry.http_method,
                entry.http_path,
                entry.http_status,
                entry.actor_type,
                entry.auth_method,
                entry.client_ip.as_deref().unwrap_or("-"),
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for listing audit log entries (tenant or system).
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub actor_type: Option<&'a str>,
    pub method: Option<&'a str>,
    pub status: Option<u16>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub actor_id: Option<Uuid>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List tenant-scoped audit log entries (paginated, with optional filters).
pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<AuditLogResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    let query = AuditLogListParams {
        actor_type: params.actor_type.map(|s| s.to_string()),
        method: params.method.map(|s| s.to_string()),
        status: params.status,
        from: params.from.map(|s| s.to_string()),
        to: params.to.map(|s| s.to_string()),
        actor_id: params.actor_id,
        page: params.page,
        per_page: params.per_page,
    };

    use rootcause::prelude::*;
    client.list_audit_logs(&query).await.context_to()
}

/// List system-level audit log entries (paginated, with optional filters).
pub async fn list_system(params: ListParams<'_>) -> Result<PaginatedResponse<SystemAuditLogResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    let query = AuditLogListParams {
        actor_type: params.actor_type.map(|s| s.to_string()),
        method: params.method.map(|s| s.to_string()),
        status: params.status,
        from: params.from.map(|s| s.to_string()),
        to: params.to.map(|s| s.to_string()),
        actor_id: params.actor_id,
        page: params.page,
        per_page: params.per_page,
    };

    use rootcause::prelude::*;
    client.list_system_audit_logs(&query).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uuid::Uuid;

    fn sample_tenant_entry() -> AuditLogResponse {
        AuditLogResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            actor_id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                .parse::<Uuid>()
                .unwrap(),
            actor_type: "user".to_string(),
            auth_method: "password".to_string(),
            http_method: "GET".to_string(),
            http_path: "/api/v1/hosts".to_string(),
            route_pattern: Some("/api/v1/hosts".to_string()),
            http_status: 200,
            client_ip: Some("127.0.0.1".to_string()),
            user_agent: Some("curl/8.0".to_string()),
            duration_ms: 42,
            occurred_at: datetime!(2025-01-01 12:00:00 UTC),
        }
    }

    fn sample_system_entry() -> SystemAuditLogResponse {
        SystemAuditLogResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6"
                .parse::<Uuid>()
                .unwrap(),
            actor_id: "d1d2d3d4-e1e2-f1f2-a1a2-b1b2b3b4b5b6"
                .parse::<Uuid>()
                .unwrap(),
            actor_type: "user".to_string(),
            auth_method: "oidc".to_string(),
            http_method: "PUT".to_string(),
            http_path: "/api/v1/global-settings/network".to_string(),
            route_pattern: Some("/api/v1/global-settings/network".to_string()),
            http_status: 200,
            client_ip: Some("10.0.0.1".to_string()),
            user_agent: None,
            duration_ms: 15,
            occurred_at: datetime!(2025-01-02 08:30:00 UTC),
        }
    }

    #[test]
    fn tenant_audit_log_paginated_human_output() {
        let resp = PaginatedResponse {
            items: vec![sample_tenant_entry()],
            total: 1,
            page: 1,
            per_page: 25,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("GET"), "method missing");
        assert!(s.contains("/api/v1/hosts"), "path missing");
        assert!(s.contains("200"), "status missing");
        assert!(s.contains("user"), "actor_type missing");
        assert!(s.contains("password"), "auth_method missing");
        assert!(s.contains("127.0.0.1"), "ip missing");
    }

    #[test]
    fn tenant_audit_log_paginated_empty() {
        let resp: PaginatedResponse<AuditLogResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 25,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No audit log entries"));
    }

    #[test]
    fn system_audit_log_paginated_human_output() {
        let resp = PaginatedResponse {
            items: vec![sample_system_entry()],
            total: 1,
            page: 1,
            per_page: 25,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("PUT"), "method missing");
        assert!(s.contains("/api/v1/global-settings/network"), "path missing");
        assert!(s.contains("oidc"), "auth_method missing");
    }

    #[test]
    fn system_audit_log_paginated_empty() {
        let resp: PaginatedResponse<SystemAuditLogResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 25,
            total_pages: 0,
        };
        assert!(resp
            .to_human_string()
            .contains("No system audit log entries"));
    }

    #[test]
    fn no_ip_shown_as_dash() {
        let mut entry = sample_tenant_entry();
        entry.client_ip = None;
        let resp = PaginatedResponse {
            items: vec![entry],
            total: 1,
            page: 1,
            per_page: 25,
            total_pages: 1,
        };
        assert!(resp.to_human_string().contains('-'), "missing dash for no IP");
    }
}
