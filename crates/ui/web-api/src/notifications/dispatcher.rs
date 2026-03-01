use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::mpsc;
use uuid::Uuid;

use uptrakit_notification_channels::ChannelRegistry;

use super::events::NotificationEvent;

/// Fire-and-forget notification dispatcher.
///
/// Event producers call `dispatch()` to enqueue events. The background
/// loop processes events asynchronously: matching rules, building messages,
/// and delivering through channels. Delivery failures are logged but never
/// surface to event producers.
#[derive(Clone)]
pub struct NotificationDispatcher {
    tx: mpsc::UnboundedSender<NotificationEvent>,
}

impl NotificationDispatcher {
    /// Create a new dispatcher and spawn the background processing loop.
    pub fn new(
        db: DatabaseConnection,
        channel_registry: Arc<ChannelRegistry>,
        callback_base_url: String,
        settings: crate::settings::Settings,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(dispatch_loop(
            db,
            channel_registry,
            callback_base_url,
            settings,
            rx,
        ));
        Self { tx }
    }

    /// Enqueue a notification event for background processing.
    ///
    /// This never blocks and never fails from the caller's perspective.
    /// If the channel is closed (dispatcher shut down), the event is silently dropped.
    pub fn dispatch(&self, event: NotificationEvent) {
        if let Err(e) = self.tx.send(event) {
            tracing::warn!(
                "notification dispatcher channel closed, dropping event: {}",
                e
            );
        }
    }
}

/// Merge global SMTP settings into a per-channel email config object.
///
/// The per-channel email config contains only `to_addresses`. This function
/// adds the SMTP connection and auth fields from the live settings snapshot
/// so that [`EmailChannel::deliver`](uptrakit_notification_channels::EmailChannel::deliver)
/// receives the full merged config.
fn merge_smtp_into_config(
    smtp: &crate::settings::SmtpSettingsSnapshot,
    mut config: serde_json::Value,
) -> serde_json::Value {
    let obj = config.as_object_mut().expect("config is always an object");
    if let Some(ref host) = smtp.host {
        obj.insert("smtp_host".to_string(), serde_json::json!(host));
    }
    obj.insert(
        "smtp_port".to_string(),
        serde_json::json!(smtp.port.unwrap_or(587)),
    );
    if let Some(ref username) = smtp.username {
        obj.insert("smtp_username".to_string(), serde_json::json!(username));
    }
    if let Some(ref password) = smtp.password {
        obj.insert("smtp_password".to_string(), serde_json::json!(password));
    }
    if let Some(ref from_address) = smtp.from_address {
        obj.insert("from_address".to_string(), serde_json::json!(from_address));
    }
    if let Some(ref from_name) = smtp.from_name {
        obj.insert("from_name".to_string(), serde_json::json!(from_name));
    }
    obj.insert(
        "tls_mode".to_string(),
        serde_json::json!(smtp.tls_mode.clone()),
    );
    config
}

/// Public (crate-visible) re-export of [`merge_smtp_into_config`] for use in route handlers
/// (e.g. the `test_channel` endpoint) that need to perform the same SMTP merge logic.
pub(crate) fn merge_smtp_into_config_pub(
    smtp: &crate::settings::SmtpSettingsSnapshot,
    config: serde_json::Value,
) -> serde_json::Value {
    merge_smtp_into_config(smtp, config)
}

async fn dispatch_loop(
    db: DatabaseConnection,
    channel_registry: Arc<ChannelRegistry>,
    callback_base_url: String,
    settings: crate::settings::Settings,
    mut rx: mpsc::UnboundedReceiver<NotificationEvent>,
) {
    use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{notification_channel, notification_log, notification_rule};

    while let Some(event) = rx.recv().await {
        let event_type = event.event_type();
        let event_type_str = event_type.as_str().to_string();
        let tenant_id = event.tenant_id;

        // Load matching rules
        let rules = match notification_rule::Entity::find()
            .filter(notification_rule::Column::TenantId.eq(tenant_id))
            .filter(notification_rule::Column::EventType.eq(&event_type_str))
            .filter(notification_rule::Column::Enabled.eq(true))
            .all(&db)
            .await
        {
            Ok(rules) => rules,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    %tenant_id,
                    event_type = %event_type_str,
                    "failed to load notification rules"
                );
                continue;
            }
        };

        for rule in rules {
            // Scope filtering: if rule specifies a filter, it must match
            if let Some(rule_host_id) = rule.host_id
                && event.host_id != Some(rule_host_id)
            {
                continue;
            }
            if let Some(ref rule_software_id) = rule.software_item_id
                && event.software_item_id.as_ref() != Some(rule_software_id)
            {
                continue;
            }
            if let Some(ref rule_plugin_type) = rule.plugin_type
                && event.plugin_type.as_ref() != Some(rule_plugin_type)
            {
                continue;
            }

            // Load the channel
            let channel_model = match notification_channel::Entity::find_by_id(rule.channel_id)
                .filter(notification_channel::Column::TenantId.eq(tenant_id))
                .filter(notification_channel::Column::Enabled.eq(true))
                .one(&db)
                .await
            {
                Ok(Some(c)) => c,
                Ok(None) => continue, // Channel disabled or deleted
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        channel_id = %rule.channel_id,
                        "failed to load notification channel"
                    );
                    continue;
                }
            };

            // Look up channel implementation
            let channel_impl = match channel_registry.get(&channel_model.channel_type) {
                Some(c) => c,
                None => {
                    tracing::warn!(
                        channel_type = %channel_model.channel_type,
                        "no channel implementation for type"
                    );
                    continue;
                }
            };

            // Parse config JSON
            let config_json: serde_json::Value =
                match serde_json::from_str(channel_model.config.expose_secret()) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            channel_id = %channel_model.id,
                            "failed to parse channel config JSON"
                        );
                        continue;
                    }
                };

            // For email channels, merge the global SMTP settings into the
            // per-channel config (which only stores `to_addresses`).
            let config_json = if channel_model.channel_type == "email" {
                let smtp = settings.smtp();
                if !smtp.is_configured() {
                    tracing::warn!(
                        channel_id = %channel_model.id,
                        "skipping email notification: SMTP settings not configured"
                    );
                    continue;
                }
                merge_smtp_into_config(&smtp, config_json)
            } else {
                config_json
            };

            // Generate action token if the event is actionable
            let action_token = event.action_params().map(|_| Uuid::now_v7());

            // Build the channel-agnostic message
            let message = super::message_builder::build_delivery_message(
                &event,
                action_token,
                &callback_base_url,
                channel_model.id,
            );

            // Serialize event payload for the log
            let event_payload = serde_json::to_value(&event.details).unwrap_or_default();

            // Insert log entry as pending
            let log_id = Uuid::now_v7();
            let now = OffsetDateTime::now_utc();
            let log_entry = notification_log::ActiveModel {
                id: Set(log_id),
                tenant_id: Set(tenant_id),
                channel_id: Set(channel_model.id),
                rule_id: Set(rule.id),
                event_type: Set(event_type_str.clone()),
                event_payload: Set(event_payload),
                status: Set("pending".to_string()),
                error_message: Set(None),
                action_token: Set(action_token),
                action_taken: Set(None),
                created_at: Set(now),
                delivered_at: Set(None),
            };

            if let Err(e) = notification_log::Entity::insert(log_entry).exec(&db).await {
                tracing::error!(
                    error = %e,
                    log_id = %log_id,
                    "failed to insert notification log entry"
                );
                continue;
            }

            // Spawn delivery task
            let db_clone = db.clone();
            let channel_impl = channel_impl.clone();
            tokio::spawn(async move {
                match channel_impl.deliver(&config_json, &message).await {
                    Ok(()) => {
                        let now = OffsetDateTime::now_utc();
                        let update = notification_log::ActiveModel {
                            id: Set(log_id),
                            status: Set("delivered".to_string()),
                            delivered_at: Set(Some(now)),
                            ..Default::default()
                        };
                        if let Err(e) =
                            notification_log::Entity::update(update).exec(&db_clone).await
                        {
                            tracing::error!(
                                error = %e,
                                log_id = %log_id,
                                "failed to update notification log to delivered"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            log_id = %log_id,
                            error = %e,
                            "notification delivery failed"
                        );
                        let update = notification_log::ActiveModel {
                            id: Set(log_id),
                            status: Set("failed".to_string()),
                            error_message: Set(Some(e.to_string())),
                            ..Default::default()
                        };
                        if let Err(db_err) =
                            notification_log::Entity::update(update).exec(&db_clone).await
                        {
                            tracing::error!(
                                error = %db_err,
                                log_id = %log_id,
                                "failed to update notification log to failed"
                            );
                        }
                    }
                }
            });
        }
    }
}
