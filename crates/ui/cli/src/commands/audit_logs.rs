use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::audit_logs::{
    AuditLogListParams, AuditLogResponse, SystemAuditLogResponse,
};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;

#[derive(Debug, Subcommand)]
pub enum AuditLogsCommands {
    /// List tenant-scoped audit log entries
    List {
        /// Filter by actor type (user, api_token, oidc)
        #[arg(long)]
        actor_type: Option<String>,
        /// Filter by semantic action type (for example, plugin_config.create)
        #[arg(long)]
        action_type: Option<String>,
        /// Filter by action outcome (success, failed, denied, validation_failed, partial)
        #[arg(long)]
        outcome: Option<String>,
        /// Filter by semantic target type (for example, plugin_config)
        #[arg(long)]
        target_type: Option<String>,
        /// Filter by semantic target id
        #[arg(long)]
        target_id: Option<String>,
        /// Lower bound timestamp (inclusive), RFC 3339 format
        #[arg(long)]
        from: Option<String>,
        /// Upper bound timestamp (inclusive), RFC 3339 format
        #[arg(long)]
        to: Option<String>,
        /// Filter entries by a specific actor UUID
        #[arg(long)]
        actor_id: Option<Uuid>,
        /// Filter by correlation UUID
        #[arg(long)]
        correlation_id: Option<Uuid>,
        /// Filter by action kind (stateful or event)
        #[arg(long)]
        action_kind: Option<String>,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// View system-level audit log entries (global settings, CA rotation, etc.)
    System {
        #[command(subcommand)]
        command: AuditLogsSystemCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuditLogsSystemCommands {
    /// List system-level audit log entries
    List {
        /// Filter by actor type (user, api_token, oidc)
        #[arg(long)]
        actor_type: Option<String>,
        /// Filter by semantic action type (for example, plugin_config.create)
        #[arg(long)]
        action_type: Option<String>,
        /// Filter by action outcome (success, failed, denied, validation_failed, partial)
        #[arg(long)]
        outcome: Option<String>,
        /// Filter by semantic target type (for example, plugin_config)
        #[arg(long)]
        target_type: Option<String>,
        /// Filter by semantic target id
        #[arg(long)]
        target_id: Option<String>,
        /// Lower bound timestamp (inclusive), RFC 3339 format
        #[arg(long)]
        from: Option<String>,
        /// Upper bound timestamp (inclusive), RFC 3339 format
        #[arg(long)]
        to: Option<String>,
        /// Filter entries by a specific actor UUID
        #[arg(long)]
        actor_id: Option<Uuid>,
        /// Filter by correlation UUID
        #[arg(long)]
        correlation_id: Option<Uuid>,
        /// Filter by action kind (stateful or event)
        #[arg(long)]
        action_kind: Option<String>,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
}

pub async fn dispatch(command: AuditLogsCommands, ctx: &CliContext) -> Result<()> {
    match command {
        AuditLogsCommands::List {
            actor_type,
            action_type,
            outcome,
            target_type,
            target_id,
            from,
            to,
            actor_id,
            correlation_id,
            action_kind,
            page,
            per_page,
        } => {
            let resp = list(ListParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
                actor_type: actor_type.as_deref(),
                action_type: action_type.as_deref(),
                outcome: outcome.as_deref(),
                target_type: target_type.as_deref(),
                target_id: target_id.as_deref(),
                from: from.as_deref(),
                to: to.as_deref(),
                actor_id,
                correlation_id,
                action_kind,
                page,
                per_page,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        AuditLogsCommands::System { command } => match command {
            AuditLogsSystemCommands::List {
                actor_type,
                action_type,
                outcome,
                target_type,
                target_id,
                from,
                to,
                actor_id,
                correlation_id,
                action_kind,
                page,
                per_page,
            } => {
                let resp = list_system(ListParams {
                    server: ctx.server.as_deref(),
                    token: ctx.token.as_deref(),
                    insecure: ctx.insecure,
                    request_timeout: ctx.request_timeout,
                    actor_type: actor_type.as_deref(),
                    action_type: action_type.as_deref(),
                    outcome: outcome.as_deref(),
                    target_type: target_type.as_deref(),
                    target_id: target_id.as_deref(),
                    from: from.as_deref(),
                    to: to.as_deref(),
                    actor_id,
                    correlation_id,
                    action_kind,
                    page,
                    per_page,
                })
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
        },
    }
    Ok(())
}

// ── Human output ────────────────────────────────────────────────────────────

fn format_occurred_at(dt: &time::OffsetDateTime) -> String {
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}

fn render_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn render_state_changes(out: &mut String, entry: &AuditLogResponse) {
    if entry.action_kind != "stateful" {
        return;
    }
    let (Some(before), Some(after)) = (&entry.before_snapshot, &entry.after_snapshot) else {
        return;
    };
    let before_map = before.as_object();
    let after_map = after.as_object();
    let mut printed_header = false;

    if let Some(after_map) = after_map {
        for (key, after_value) in after_map {
            match before_map.and_then(|m| m.get(key)) {
                None => {
                    if !printed_header {
                        out.push_str("  State changes:\n");
                        printed_header = true;
                    }
                    out.push_str(&format!("    + {key} = {}\n", render_value(after_value)));
                }
                Some(before_value) if before_value != after_value => {
                    if !printed_header {
                        out.push_str("  State changes:\n");
                        printed_header = true;
                    }
                    out.push_str(&format!(
                        "    ~ {key} = {} -> {}\n",
                        render_value(before_value),
                        render_value(after_value)
                    ));
                }
                Some(_) => {}
            }
        }
    }
    if let Some(before_map) = before_map {
        for (key, before_value) in before_map {
            if after_map.is_some_and(|m| m.contains_key(key)) {
                continue;
            }
            if !printed_header {
                out.push_str("  State changes:\n");
                printed_header = true;
            }
            out.push_str(&format!("    - {key} = {}\n", render_value(before_value)));
        }
    }
}

impl HumanOutput for PaginatedResponse<AuditLogResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No audit log entries found.\n".to_string();
        }
        let mut out = format!(
            "{:<27} {:<38} {:<18} {:<24} {}\n",
            "OCCURRED_AT", "ACTION", "OUTCOME", "TARGET", "ACTOR"
        );
        for entry in &self.items {
            out.push_str(&format!(
                "{:<27} {:<38} {:<18} {:<24} {}\n",
                format_occurred_at(&entry.occurred_at),
                entry.action_type,
                entry.outcome,
                entry.target_display.as_deref().unwrap_or("-"),
                entry
                    .actor_display
                    .as_deref()
                    .unwrap_or(entry.actor_type.as_str()),
            ));
            render_state_changes(&mut out, entry);
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
            "{:<27} {:<38} {:<18} {:<24} {}\n",
            "OCCURRED_AT", "ACTION", "OUTCOME", "TARGET", "ACTOR"
        );
        for entry in &self.items {
            out.push_str(&format!(
                "{:<27} {:<38} {:<18} {:<24} {}\n",
                format_occurred_at(&entry.occurred_at),
                entry.action_type,
                entry.outcome,
                entry.target_display.as_deref().unwrap_or("-"),
                entry
                    .actor_display
                    .as_deref()
                    .unwrap_or(entry.actor_type.as_str()),
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
    pub action_type: Option<&'a str>,
    pub outcome: Option<&'a str>,
    pub target_type: Option<&'a str>,
    pub target_id: Option<&'a str>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub actor_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    pub action_kind: Option<String>,
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

    let mut query = AuditLogListParams::default();
    query.actor_type = params.actor_type.map(|s| s.to_string());
    query.action_type = params.action_type.map(|s| s.to_string());
    query.outcome = params.outcome.map(|s| s.to_string());
    query.target_type = params.target_type.map(|s| s.to_string());
    query.target_id = params.target_id.map(|s| s.to_string());
    query.from = params.from.map(|s| s.to_string());
    query.to = params.to.map(|s| s.to_string());
    query.actor_id = params.actor_id;
    query.correlation_id = params.correlation_id;
    query.action_kind = params.action_kind.map(|s| s.to_string());
    query.page = params.page;
    query.per_page = params.per_page;

    client.list_audit_logs(&query).await.context_to()
}

/// List system-level audit log entries (paginated, with optional filters).
pub async fn list_system(
    params: ListParams<'_>,
) -> Result<PaginatedResponse<SystemAuditLogResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    let mut query = AuditLogListParams::default();
    query.actor_type = params.actor_type.map(|s| s.to_string());
    query.action_type = params.action_type.map(|s| s.to_string());
    query.outcome = params.outcome.map(|s| s.to_string());
    query.target_type = params.target_type.map(|s| s.to_string());
    query.target_id = params.target_id.map(|s| s.to_string());
    query.from = params.from.map(|s| s.to_string());
    query.to = params.to.map(|s| s.to_string());
    query.actor_id = params.actor_id;
    query.correlation_id = params.correlation_id;
    query.action_kind = params.action_kind.map(|s| s.to_string());
    query.page = params.page;
    query.per_page = params.per_page;

    client.list_system_audit_logs(&query).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use time::macros::datetime;
    use uuid::Uuid;

    fn sample_tenant_entry() -> AuditLogResponse {
        AuditLogResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            actor_type: "user".to_string(),
            actor_id: Some(
                "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                    .parse::<Uuid>()
                    .unwrap(),
            ),
            actor_display: Some("alice@example.com".to_string()),
            action_type: "plugin_config.create".to_string(),
            target_type: Some("plugin_config".to_string()),
            target_id: Some("019semantic".to_string()),
            target_display: Some("APT Defaults".to_string()),
            outcome: "success".to_string(),
            details_json: Some(json!({ "plugin_type": "package_manager_apt" })),
            action_kind: "event".to_string(),
            before_snapshot: None,
            after_snapshot: None,
            correlation_id: None,
            request_id: Some("req-123".to_string()),
            occurred_at: datetime!(2025-01-01 12:00:00 UTC),
        }
    }

    fn sample_system_entry() -> SystemAuditLogResponse {
        SystemAuditLogResponse {
            id: "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6"
                .parse::<Uuid>()
                .unwrap(),
            actor_id: Some(
                "d1d2d3d4-e1e2-f1f2-a1a2-b1b2b3b4b5b6"
                    .parse::<Uuid>()
                    .unwrap(),
            ),
            actor_type: "user".to_string(),
            actor_display: Some("platform-admin@example.com".to_string()),
            action_type: "system.setting.update".to_string(),
            target_type: Some("global_setting".to_string()),
            target_id: Some("network".to_string()),
            target_display: Some("Network Settings".to_string()),
            outcome: "success".to_string(),
            details_json: Some(json!({ "category": "network" })),
            action_kind: "event".to_string(),
            before_snapshot: None,
            after_snapshot: None,
            correlation_id: None,
            request_id: Some("req-system-123".to_string()),
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
        assert!(s.contains("plugin_config.create"), "action missing");
        assert!(s.contains("success"), "outcome missing");
        assert!(s.contains("APT Defaults"), "target display missing");
        assert!(s.contains("alice@example.com"), "actor display missing");
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
        assert!(s.contains("system.setting.update"), "action missing");
        assert!(s.contains("Network Settings"), "target display missing");
        assert!(s.contains("platform-admin@example.com"), "actor missing");
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
        assert!(
            resp.to_human_string()
                .contains("No system audit log entries")
        );
    }

    #[test]
    fn no_target_shown_as_dash() {
        let mut entry = sample_tenant_entry();
        entry.target_display = None;
        let resp = PaginatedResponse {
            items: vec![entry],
            total: 1,
            page: 1,
            per_page: 25,
            total_pages: 1,
        };
        assert!(
            resp.to_human_string().contains('-'),
            "missing dash for no target"
        );
    }

    fn render_audit_logs_json(items: Vec<AuditLogResponse>) -> String {
        serde_json::to_string(&PaginatedResponse {
            items,
            total: 1,
            page: 1,
            per_page: 25,
            total_pages: 1,
        })
        .expect("json serialization should succeed")
    }

    #[test]
    fn audit_logs_json_output_uses_semantic_fields() {
        let item = AuditLogResponse {
            id: Uuid::new_v4(),
            actor_type: "user".into(),
            actor_id: None,
            actor_display: Some("alice@example.com".into()),
            action_type: "plugin_config.create".into(),
            target_type: Some("plugin_config".into()),
            target_id: Some("019semantic".into()),
            target_display: Some("APT Defaults".into()),
            outcome: "success".into(),
            details_json: Some(json!({ "plugin_type": "package_manager_apt" })),
            action_kind: "event".into(),
            before_snapshot: None,
            after_snapshot: None,
            correlation_id: None,
            request_id: Some("req-123".into()),
            occurred_at: datetime!(2026-04-17 12:00:00 UTC),
        };

        let rendered = render_audit_logs_json(vec![item]);
        assert!(rendered.contains("\"action_type\":\"plugin_config.create\""));
        assert!(rendered.contains("\"target_display\":\"APT Defaults\""));
        assert!(rendered.contains("\"outcome\":\"success\""));
        assert!(!rendered.contains("\"method\""));
        assert!(!rendered.contains("\"path\""));
    }

    #[test]
    fn audit_logs_json_output_includes_v2_fields() {
        let entry = sample_tenant_entry();
        // Override to stateful with snapshots
        let entry = AuditLogResponse {
            action_kind: "stateful".to_string(),
            before_snapshot: Some(serde_json::json!({"enabled": false})),
            after_snapshot: Some(serde_json::json!({"enabled": true})),
            correlation_id: Some(
                "00000000-0000-0000-0000-000000000abc"
                    .parse::<Uuid>()
                    .unwrap(),
            ),
            ..entry
        };
        let json = render_audit_logs_json(vec![entry]);
        assert!(json.contains("\"action_kind\":\"stateful\""));
        assert!(json.contains("\"before_snapshot\""));
        assert!(json.contains("\"after_snapshot\""));
        assert!(json.contains("\"correlation_id\""));
    }

    #[test]
    fn render_state_changes_shows_diff_for_stateful_entry() {
        let mut entry = sample_tenant_entry();
        entry.action_kind = "stateful".to_string();
        entry.before_snapshot = Some(serde_json::json!({"enabled": false, "name": "old"}));
        entry.after_snapshot = Some(serde_json::json!({"enabled": true, "name": "old"}));
        let mut out = String::new();
        render_state_changes(&mut out, &entry);
        assert!(out.contains("State changes:"));
        assert!(out.contains("~ enabled = false -> true"));
        assert!(!out.contains("name")); // unchanged, suppressed
    }

    #[test]
    fn render_state_changes_silent_for_event_entry() {
        let entry = sample_tenant_entry(); // action_kind = "event"
        let mut out = String::new();
        render_state_changes(&mut out, &entry);
        assert!(out.is_empty());
    }
}
