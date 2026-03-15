use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum NotificationsCommands {
    /// Manage notification channels
    Channels {
        #[command(subcommand)]
        command: ChannelsCommands,
    },
    /// Manage notification rules
    Rules {
        #[command(subcommand)]
        command: RulesCommands,
    },
    /// View notification delivery log
    Log {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ChannelsCommands {
    /// List notification channels
    List {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show notification channel details
    Get {
        /// Channel UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Create a new notification channel
    Create {
        /// Channel name
        #[arg(long)]
        name: String,
        /// Channel type (webhook, telegram)
        #[arg(long = "type")]
        channel_type: String,
        /// Channel-specific configuration as JSON string
        #[arg(long)]
        config: String,
    },
    /// Update a notification channel
    Update {
        /// Channel UUID
        id: uptrakit_openapi_client::Uuid,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// Updated configuration as JSON string
        #[arg(long)]
        config: Option<String>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Delete a notification channel
    Delete {
        /// Channel UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Send a test notification through a channel
    Test {
        /// Channel UUID
        id: uptrakit_openapi_client::Uuid,
    },
}

#[derive(Debug, Subcommand)]
pub enum RulesCommands {
    /// List notification rules
    List {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show notification rule details
    Get {
        /// Rule UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Create a new notification rule
    Create {
        /// Channel UUID to deliver notifications through
        #[arg(long)]
        channel_id: uptrakit_openapi_client::Uuid,
        /// Event type (update_available, update_completed, update_failed, new_software_discovered, new_service_enrolled, ca_rotated)
        #[arg(long)]
        event_type: String,
        /// Optionally scope to a specific host
        #[arg(long)]
        host_id: Option<uptrakit_openapi_client::Uuid>,
        /// Optionally scope to a specific software item
        #[arg(long)]
        software_item_id: Option<uptrakit_openapi_client::Uuid>,
        /// Optionally scope to a specific plugin type
        #[arg(long)]
        plugin_type: Option<String>,
    },
    /// Update a notification rule
    Update {
        /// Rule UUID
        id: uptrakit_openapi_client::Uuid,
        /// New event type
        #[arg(long)]
        event_type: Option<String>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Delete a notification rule
    Delete {
        /// Rule UUID
        id: uptrakit_openapi_client::Uuid,
    },
}

pub async fn dispatch(command: NotificationsCommands, ctx: &CliContext) -> Result<()> {
    match command {
        NotificationsCommands::Channels { command } => dispatch_channels(command, ctx).await?,
        NotificationsCommands::Rules { command } => dispatch_rules(command, ctx).await?,
        NotificationsCommands::Log { page, per_page } => {
            let resp = log_list(LogListParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
                page,
                per_page,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

async fn dispatch_channels(command: ChannelsCommands, ctx: &CliContext) -> Result<()> {
    match command {
        ChannelsCommands::List { page, per_page } => {
            let resp = channel_list(ChannelListParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
                page,
                per_page,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ChannelsCommands::Get { id } => {
            let resp = channel_get(ChannelGetParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ChannelsCommands::Create {
            name,
            channel_type,
            config,
        } => {
            if channel_type.trim().is_empty() {
                return Err(report!(CliError::Other(
                    "channel type must not be empty".to_string()
                )));
            }
            let config_value: serde_json::Value = serde_json::from_str(&config)
                .map_err(|e| report!(CliError::Other(format!("invalid JSON for --config: {e}"))))?;
            let resp = channel_create(ChannelCreateParams {
                name,
                channel_type,
                config: config_value,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ChannelsCommands::Update {
            id,
            name,
            config,
            enabled,
        } => {
            let config_value: Option<serde_json::Value> = match config {
                Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                    report!(CliError::Other(format!("invalid JSON for --config: {e}")))
                })?),
                None => None,
            };
            let resp = channel_update(ChannelUpdateParams {
                id: &id,
                name,
                config: config_value,
                enabled,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ChannelsCommands::Delete { id } => {
            let resp = channel_delete(ChannelDeleteParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        ChannelsCommands::Test { id } => {
            let resp = channel_test(ChannelTestParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

async fn dispatch_rules(command: RulesCommands, ctx: &CliContext) -> Result<()> {
    match command {
        RulesCommands::List { page, per_page } => {
            let resp = rule_list(RuleListParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
                page,
                per_page,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        RulesCommands::Get { id } => {
            let resp = rule_get(RuleGetParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        RulesCommands::Create {
            channel_id,
            event_type,
            host_id,
            software_item_id,
            plugin_type,
        } => {
            let event_type: uptrakit_openapi_client::types::notifications::NotificationEventType =
                event_type.parse().map_err(|_| {
                    report!(CliError::Other(format!(
                        "unknown event type: {event_type} (expected update_available, update_completed, update_failed, new_software_discovered, new_service_enrolled, or ca_rotated)"
                    )))
                })?;
            let resp = rule_create(RuleCreateParams {
                channel_id,
                event_type,
                host_id,
                software_item_id,
                plugin_type,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        RulesCommands::Update {
            id,
            event_type,
            enabled,
        } => {
            let event_type: Option<uptrakit_openapi_client::types::notifications::NotificationEventType> =
                match event_type {
                    Some(s) => Some(s.parse().map_err(|_| {
                        report!(CliError::Other(format!(
                            "unknown event type: {s} (expected update_available, update_completed, update_failed, new_software_discovered, new_service_enrolled, or ca_rotated)"
                        )))
                    })?),
                    None => None,
                };
            let resp = rule_update(RuleUpdateParams {
                id: &id,
                event_type,
                enabled,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        RulesCommands::Delete { id } => {
            let resp = rule_delete(RuleDeleteParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::notifications::{
    CreateNotificationChannelRequest, CreateNotificationRuleRequest, NotificationChannelResponse,
    NotificationEventType, NotificationLogResponse, NotificationRuleResponse,
    TestNotificationResponse, UpdateNotificationChannelRequest, UpdateNotificationRuleRequest,
};
use uptrakit_openapi_client::types::pagination::{PaginatedResponse, PaginationParams};

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<NotificationChannelResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No notification channels found.\n".to_string();
        }
        let mut out = format!("{:<38} {:<25} {:<12} ENABLED\n", "ID", "NAME", "TYPE");
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
    pub channel_type: String,
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
    client
        .list_notification_channels(&pagination)
        .await
        .context_to()
}

/// Get a single notification channel by ID.
pub async fn channel_get(params: ChannelGetParams<'_>) -> Result<NotificationChannelResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .get_notification_channel(params.id)
        .await
        .context_to()
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
    client
        .list_notification_rules(&pagination)
        .await
        .context_to()
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
            channel_type: "webhook".to_string(),
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
            status:
                uptrakit_openapi_client::types::notifications::NotificationDeliveryStatus::Delivered,
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
        assert!(
            resp.to_human_string()
                .contains("No notification channels found")
        );
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
        assert!(
            s.contains("Software Item ID:"),
            "software item id label missing"
        );
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
        assert!(
            resp.to_human_string()
                .contains("No notification rules found")
        );
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
        assert!(
            resp.to_human_string()
                .contains("No notification log entries found")
        );
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
        assert!(
            output
                .to_human_string()
                .contains("Notification channel deleted")
        );
    }
}
