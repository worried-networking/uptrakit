use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::mpsc;
use uuid::Uuid;

use uptrakit_plugin_infrastructure_registry::PluginOps;

use super::events::NotificationEvent;

/// Bounded capacity for the notification dispatcher channel.
///
/// Limits memory consumption under bulk update completions that generate many
/// simultaneous notification events. When the channel is full, events are
/// dropped and a warning is logged (fire-and-forget semantics).
const NOTIFICATION_DISPATCHER_CAPACITY: usize = 4096;

/// Fire-and-forget notification dispatcher.
///
/// Event producers call `dispatch()` to enqueue events. The background
/// loop processes events asynchronously: matching rules, building messages,
/// and delivering through channels. Delivery failures are logged but never
/// surface to event producers.
#[derive(Clone)]
pub struct NotificationDispatcher {
    tx: mpsc::Sender<NotificationEvent>,
}

impl NotificationDispatcher {
    /// Create a new dispatcher and spawn the background processing loop.
    pub fn new(
        db: DatabaseConnection,
        notification_ops: Arc<dyn PluginOps>,
        callback_base_url: String,
    ) -> Self {
        let (tx, rx) = mpsc::channel(NOTIFICATION_DISPATCHER_CAPACITY);
        tokio::spawn(dispatch_loop(db, notification_ops, callback_base_url, rx));
        Self { tx }
    }

    /// Enqueue a notification event for background processing.
    ///
    /// This never blocks and never fails from the caller's perspective.
    /// If the channel is full, the event is dropped and a warning is logged.
    /// If the channel is closed (dispatcher shut down), the event is silently dropped.
    #[tracing::instrument(skip_all)]
    pub fn dispatch(&self, event: NotificationEvent) {
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    "notification dispatcher channel full (capacity: {NOTIFICATION_DISPATCHER_CAPACITY}), dropping event"
                );
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("notification dispatcher channel closed, dropping event");
            }
        }
    }
}

#[cfg(test)]
impl NotificationDispatcher {
    /// Create a dispatcher whose events are sent to the returned receiver
    /// instead of spawning the background dispatch loop. Use in unit tests to
    /// observe dispatched [`NotificationEvent`]s without a real database.
    pub fn test_channel() -> (Self, mpsc::Receiver<NotificationEvent>) {
        let (tx, rx) = mpsc::channel(64);
        (Self { tx }, rx)
    }
}

/// Build a generic settings bag by loading SMTP and Telegram settings
/// directly from the database.
///
/// The resulting JSON has `{ "tenant": { ... }, "global": { ... } }` with
/// dot-prefixed keys matching the setting names used by each notification
/// plugin's `deliver()` implementation. Plugins extract only the keys they
/// recognise and ignore the rest, so every plugin receives the full bag.
pub(crate) async fn build_settings_bag(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> serde_json::Value {
    let tenant =
        uptrakit_web_api_auth::settings_store::load_settings_by_prefix(db, tenant_id, "smtp.")
            .await
            .unwrap_or_default();

    let global_smtp =
        uptrakit_web_api_auth::settings_store::load_global_settings_by_prefix(db, "global_smtp.")
            .await
            .unwrap_or_default();

    let global_telegram = uptrakit_web_api_auth::settings_store::load_global_settings_by_prefix(
        db,
        "global_telegram.",
    )
    .await
    .unwrap_or_default();

    let mut global = serde_json::Map::new();
    for (k, v) in &global_smtp {
        global.insert(k.clone(), v.clone());
    }
    for (k, v) in &global_telegram {
        global.insert(k.clone(), v.clone());
    }

    let mut tenant_map = serde_json::Map::new();
    for (k, v) in &tenant {
        tenant_map.insert(k.clone(), v.clone());
    }

    serde_json::json!({ "tenant": tenant_map, "global": global })
}

#[tracing::instrument(skip_all)]
async fn dispatch_loop(
    db: DatabaseConnection,
    notification_ops: Arc<dyn PluginOps>,
    callback_base_url: String,
    mut rx: mpsc::Receiver<NotificationEvent>,
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
            let channel_type_id =
                uptrakit_shared_types::PluginTypeId::new(&channel_model.channel_type);
            let channel_transport = match notification_ops.transport(&channel_type_id) {
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

            // Build a generic settings bag from the database.
            // Each plugin's `deliver()` extracts only the keys it needs.
            let settings_bag = build_settings_bag(&db, tenant_id).await;

            // Generate action token if the event is actionable
            let action_token = event.action_params().map(|_| Uuid::now_v7());

            // Build the channel-agnostic message
            let message = super::message_builder::build_delivery_message(
                &event,
                action_token,
                &callback_base_url,
                &channel_model.channel_type,
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
            let channel_transport = channel_transport.clone();
            tokio::spawn(async move {
                match channel_transport
                    .deliver(&config_json, &settings_bag, &message)
                    .await
                {
                    Ok(()) => {
                        let now = OffsetDateTime::now_utc();
                        let update = notification_log::ActiveModel {
                            id: Set(log_id),
                            status: Set("delivered".to_string()),
                            delivered_at: Set(Some(now)),
                            ..Default::default()
                        };
                        if let Err(e) = notification_log::Entity::update(update)
                            .exec(&db_clone)
                            .await
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
                        if let Err(db_err) = notification_log::Entity::update(update)
                            .exec(&db_clone)
                            .await
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
