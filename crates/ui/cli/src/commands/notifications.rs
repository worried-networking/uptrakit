use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::notifications::{
    CreateNotificationChannelRequest, CreateNotificationRuleRequest, NotificationChannelResponse,
    NotificationChannelType, NotificationEventType, NotificationLogResponse,
    NotificationRuleResponse, TestNotificationResponse, UpdateNotificationChannelRequest,
    UpdateNotificationRuleRequest,
};
use uptrakit_openapi_client::types::pagination::{PaginatedResponse, PaginationParams};

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<NotificationChannelResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No notification channels found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<25} {:<12} ENABLED\n",
            "ID", "NAME", "TYPE"
        );
        for ch in &self.items {
            out.push_str(&format!(
                "{:<38} {:<25} {:<12} {}\n",
                ch.id, ch.name, ch.channel_type, ch.enabled
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for NotificationChannelResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:         {}\n", self.id));
        out.push_str(&format!("Name:       {}\n", self.name));
        out.push_str(&format!("Type:       {}\n", self.channel_type));
        out.push_str(&format!("Enabled:    {}\n", self.enabled));
        out.push_str(&format!(
            "Config:     {}\n",
            serde_json::to_string_pretty(&self.config).unwrap_or_else(|_| self.config.to_string())
        ));
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

impl HumanOutput for TestNotificationResponse {
    fn to_human_string(&self) -> String {
        if self.success {
            format!("Test succeeded: {}\n", self.message)
        } else {
            format!("Test failed: {}\n", self.message)
        }
    }
}

impl HumanOutput for PaginatedResponse<NotificationRuleResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No notification rules found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<38} {:<25} ENABLED\n",
            "ID", "CHANNEL", "EVENT TYPE"
        );
        for r in &self.items {
            out.push_str(&format!(
                "{:<38} {:<38} {:<25} {}\n",
                r.id, r.channel_id, r.event_type, r.enabled
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for NotificationRuleResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:               {}\n", self.id));
        out.push_str(&format!("Channel ID:       {}\n", self.channel_id));
        out.push_str(&format!("Event Type:       {}\n", self.event_type));
        out.push_str(&format!("Enabled:          {}\n", self.enabled));
        if let Some(ref host_id) = self.host_id {
            out.push_str(&format!("Host ID:          {host_id}\n"));
        }
        if let Some(ref sw_id) = self.software_item_id {
            out.push_str(&format!("Software Item ID: {sw_id}\n"));
        }
        if let Some(ref pt) = self.plugin_type {
            out.push_str(&format!("Plugin Type:      {pt}\n"));
        }
        out.push_str(&format!(
            "Created:          {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out
    }
}

impl HumanOutput for PaginatedResponse<NotificationLogResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No notification log entries found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<25} {:<12} CREATED\n",
            "ID", "EVENT TYPE", "STATUS"
        );
        for entry in &self.items {
            let created = entry
                .created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| entry.created_at.to_string());
            out.push_str(&format!(
                "{:<38} {:<25} {:<12} {}\n",
                entry.id, entry.event_type, entry.status, created
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

/// Returned by delete operations that have no server response body.
#[derive(Debug, Serialize)]
pub struct DeletedOutput {
    pub message: String,
}

impl HumanOutput for DeletedOutput {
    fn to_human_string(&self) -> String {
        format!("{}\n", self.message)
    }
}

// ── Channel params ──────────────────────────────────────────────────────────

pub struct ChannelListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

pub struct ChannelGetParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ChannelCreateParams<'a> {
    pub name: String,
    pub channel_type: NotificationChannelType,
    pub config: serde_json::Value,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ChannelUpdateParams<'a> {
    pub id: &'a Uuid,
    pub name: Option<String>,
    pub config: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ChannelDeleteParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ChannelTestParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Rule params ─────────────────────────────────────────────────────────────

pub struct RuleListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

pub struct RuleGetParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct RuleCreateParams<'a> {
    pub channel_id: Uuid,
    pub event_type: NotificationEventType,
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub plugin_type: Option<String>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct RuleUpdateParams<'a> {
    pub id: &'a Uuid,
    pub event_type: Option<NotificationEventType>,
    pub enabled: Option<bool>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct RuleDeleteParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Log params ──────────────────────────────────────────────────────────────

pub struct LogListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

// ── Channel commands ────────────────────────────────────────────────────────

/// List notification channels (paginated).
pub async fn channel_list(
    params: ChannelListParams<'_>,
) -> Result<PaginatedResponse<NotificationChannelResponse>> {
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
    client.list_notification_channels(&pagination).await.context_to()
}

/// Get a single notification channel by ID.
pub async fn channel_get(params: ChannelGetParams<'_>) -> Result<NotificationChannelResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.get_notification_channel(params.id).await.context_to()
}

/// Create a new notification channel.
pub async fn channel_create(
    params: ChannelCreateParams<'_>,
) -> Result<NotificationChannelResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateNotificationChannelRequest {
        name: params.name,
        channel_type: params.channel_type,
        config: params.config,
        enabled: true,
    };
    client.create_notification_channel(&req).await.context_to()
}

/// Update an existing notification channel.
pub async fn channel_update(
    params: ChannelUpdateParams<'_>,
) -> Result<NotificationChannelResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateNotificationChannelRequest {
        name: params.name,
        config: params.config,
        enabled: params.enabled,
    };
    client
        .update_notification_channel(params.id, &req)
        .await
        .context_to()
}

/// Delete a notification channel.
pub async fn channel_delete(params: ChannelDeleteParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .delete_notification_channel(params.id)
        .await
        .context_to()?;
    Ok(DeletedOutput {
        message: "Notification channel deleted.".to_string(),
    })
}

/// Send a test notification through a channel.
pub async fn channel_test(params: ChannelTestParams<'_>) -> Result<TestNotificationResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .test_notification_channel(params.id)
        .await
        .context_to()
}

// ── Rule commands ───────────────────────────────────────────────────────────

/// List notification rules (paginated).
pub async fn rule_list(
    params: RuleListParams<'_>,
) -> Result<PaginatedResponse<NotificationRuleResponse>> {
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
    client.list_notification_rules(&pagination).await.context_to()
}

/// Get a single notification rule by ID.
pub async fn rule_get(params: RuleGetParams<'_>) -> Result<NotificationRuleResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.get_notification_rule(params.id).await.context_to()
}

/// Create a new notification rule.
pub async fn rule_create(params: RuleCreateParams<'_>) -> Result<NotificationRuleResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateNotificationRuleRequest {
        channel_id: params.channel_id,
        event_type: params.event_type,
        host_id: params.host_id,
        software_item_id: params.software_item_id,
        plugin_type: params.plugin_type,
        enabled: true,
    };
    client.create_notification_rule(&req).await.context_to()
}

/// Update an existing notification rule.
pub async fn rule_update(params: RuleUpdateParams<'_>) -> Result<NotificationRuleResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateNotificationRuleRequest {
        event_type: params.event_type,
        host_id: None,
        software_item_id: None,
        plugin_type: None,
        enabled: params.enabled,
    };
    client
        .update_notification_rule(params.id, &req)
        .await
        .context_to()
}

/// Delete a notification rule.
pub async fn rule_delete(params: RuleDeleteParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .delete_notification_rule(params.id)
        .await
        .context_to()?;
    Ok(DeletedOutput {
        message: "Notification rule deleted.".to_string(),
    })
}

// ── Log commands ────────────────────────────────────────────────────────────

/// List notification log entries (paginated).
pub async fn log_list(
    params: LogListParams<'_>,
) -> Result<PaginatedResponse<NotificationLogResponse>> {
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
    client.list_notification_log(&pagination).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_uuid() -> Uuid {
        "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
            .parse::<Uuid>()
            .unwrap()
    }

    fn sample_channel() -> NotificationChannelResponse {
        NotificationChannelResponse {
            id: sample_uuid(),
            name: "My Webhook".to_string(),
            channel_type: NotificationChannelType::Webhook,
            config: serde_json::json!({"url": "https://example.com/hook"}),
            enabled: true,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 12:00:00 UTC),
        }
    }

    fn sample_rule() -> NotificationRuleResponse {
        NotificationRuleResponse {
            id: sample_uuid(),
            channel_id: sample_uuid(),
            event_type: NotificationEventType::UpdateAvailable,
            host_id: None,
            software_item_id: None,
            plugin_type: None,
            enabled: true,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
        }
    }

    fn sample_log_entry() -> NotificationLogResponse {
        NotificationLogResponse {
            id: sample_uuid(),
            channel_id: sample_uuid(),
            rule_id: sample_uuid(),
            event_type: NotificationEventType::UpdateCompleted,
            event_payload: serde_json::json!({"version": "1.2.3"}),
            status: uptrakit_openapi_client::types::notifications::NotificationDeliveryStatus::Delivered,
            error_message: None,
            action_token: None,
            action_taken: None,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            delivered_at: Some(datetime!(2025-01-01 00:00:01 UTC)),
        }
    }

    // ── Channel output tests ────────────────────────────────────────────

    #[test]
    fn channel_detail_human_output_contains_key_fields() {
        let ch = sample_channel();
        let s = ch.to_human_string();
        assert!(s.contains("My Webhook"), "name missing");
        assert!(s.contains("webhook"), "type missing");
        assert!(s.contains("true"), "enabled missing");
        assert!(s.contains("example.com"), "config missing");
    }

    #[test]
    fn paginated_channels_empty() {
        let resp: PaginatedResponse<NotificationChannelResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No notification channels found"));
    }

    #[test]
    fn paginated_channels_has_header_and_row() {
        let resp = PaginatedResponse {
            items: vec![sample_channel()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("NAME"), "header missing");
        assert!(s.contains("My Webhook"), "channel row missing");
    }

    // ── Rule output tests ───────────────────────────────────────────────

    #[test]
    fn rule_detail_human_output_contains_key_fields() {
        let r = sample_rule();
        let s = r.to_human_string();
        assert!(s.contains("update_available"), "event type missing");
        assert!(s.contains("true"), "enabled missing");
    }

    #[test]
    fn rule_detail_with_optional_fields() {
        let mut r = sample_rule();
        r.host_id = Some(sample_uuid());
        r.software_item_id = Some(sample_uuid());
        r.plugin_type = Some("releases_github".to_string());
        let s = r.to_human_string();
        assert!(s.contains("Host ID:"), "host id label missing");
        assert!(s.contains("Software Item ID:"), "software item id label missing");
        assert!(s.contains("releases_github"), "plugin type missing");
    }

    #[test]
    fn paginated_rules_empty() {
        let resp: PaginatedResponse<NotificationRuleResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No notification rules found"));
    }

    #[test]
    fn paginated_rules_has_header_and_row() {
        let resp = PaginatedResponse {
            items: vec![sample_rule()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("EVENT TYPE"), "header missing");
        assert!(s.contains("update_available"), "rule row missing");
    }

    // ── Log output tests ────────────────────────────────────────────────

    #[test]
    fn paginated_log_empty() {
        let resp: PaginatedResponse<NotificationLogResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No notification log entries found"));
    }

    #[test]
    fn paginated_log_has_header_and_row() {
        let resp = PaginatedResponse {
            items: vec![sample_log_entry()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("EVENT TYPE"), "header missing");
        assert!(s.contains("update_completed"), "log row missing");
        assert!(s.contains("delivered"), "status missing");
    }

    // ── Test notification output tests ──────────────────────────────────

    #[test]
    fn test_notification_success_output() {
        let resp = TestNotificationResponse {
            success: true,
            message: "Notification delivered".to_string(),
        };
        let s = resp.to_human_string();
        assert!(s.contains("Test succeeded"), "success prefix missing");
        assert!(s.contains("Notification delivered"), "message missing");
    }

    #[test]
    fn test_notification_failure_output() {
        let resp = TestNotificationResponse {
            success: false,
            message: "Connection refused".to_string(),
        };
        let s = resp.to_human_string();
        assert!(s.contains("Test failed"), "failure prefix missing");
        assert!(s.contains("Connection refused"), "message missing");
    }

    // ── Deleted output tests ────────────────────────────────────────────

    #[test]
    fn deleted_output_human() {
        let output = DeletedOutput {
            message: "Notification channel deleted.".to_string(),
        };
        assert!(output.to_human_string().contains("Notification channel deleted"));
    }
}
