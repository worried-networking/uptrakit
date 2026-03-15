use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::mpsc;
use uuid::Uuid;

use uptrakit_plugin_infrastructure_registry::PluginOps;

use super::events::NotificationEvent;
use crate::settings::Settings;

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
        settings: crate::settings::Settings,
    ) -> Self {
        let (tx, rx) = mpsc::channel(NOTIFICATION_DISPATCHER_CAPACITY);
        tokio::spawn(dispatch_loop(
            db,
            notification_ops,
            callback_base_url,
            settings,
            rx,
        ));
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

/// Build a generic settings bag from the cached [`Settings`] snapshot.
///
/// The resulting JSON has `{ "tenant": { ... }, "global": { ... } }` with
/// dot-prefixed keys matching the setting names used by each notification
/// plugin's `deliver()` implementation. Plugins extract only the keys they
/// recognise and ignore the rest, so every plugin receives the full bag.
pub(crate) fn build_settings_bag(settings: &Settings) -> serde_json::Value {
    let snap = settings.snapshot();

    let mut tenant = serde_json::Map::new();
    // SMTP tenant settings
    if let Some(ref h) = snap.smtp.host {
        tenant.insert("smtp.host".into(), serde_json::json!(h));
    }
    if let Some(p) = snap.smtp.port {
        tenant.insert("smtp.port".into(), serde_json::json!(p));
    }
    if let Some(ref u) = snap.smtp.username {
        tenant.insert("smtp.username".into(), serde_json::json!(u));
    }
    if let Some(ref pw) = snap.smtp.password {
        tenant.insert("smtp.password".into(), serde_json::json!(pw));
    }
    if let Some(ref f) = snap.smtp.from_address {
        tenant.insert("smtp.from_address".into(), serde_json::json!(f));
    }
    if let Some(ref n) = snap.smtp.from_name {
        tenant.insert("smtp.from_name".into(), serde_json::json!(n));
    }
    tenant.insert(
        "smtp.tls_mode".into(),
        serde_json::json!(snap.smtp.tls_mode),
    );

    let mut global = serde_json::Map::new();
    // Global SMTP settings
    if let Some(ref h) = snap.global_smtp.host {
        global.insert("global_smtp.host".into(), serde_json::json!(h));
    }
    if let Some(p) = snap.global_smtp.port {
        global.insert("global_smtp.port".into(), serde_json::json!(p));
    }
    if let Some(ref u) = snap.global_smtp.username {
        global.insert("global_smtp.username".into(), serde_json::json!(u));
    }
    if let Some(ref pw) = snap.global_smtp.password {
        global.insert("global_smtp.password".into(), serde_json::json!(pw));
    }
    if let Some(ref f) = snap.global_smtp.from_address {
        global.insert("global_smtp.from_address".into(), serde_json::json!(f));
    }
    if let Some(ref n) = snap.global_smtp.from_name {
        global.insert("global_smtp.from_name".into(), serde_json::json!(n));
    }
    global.insert(
        "global_smtp.tls_mode".into(),
        serde_json::json!(snap.global_smtp.tls_mode),
    );
    if let Some(ref h) = snap.global_smtp.helo_host {
        global.insert("global_smtp.helo_host".into(), serde_json::json!(h));
    }
    // Global Telegram settings
    if let Some(ref t) = snap.global_telegram.bot_token {
        global.insert("global_telegram.bot_token".into(), serde_json::json!(t));
    }

    serde_json::json!({ "tenant": tenant, "global": global })
}

#[tracing::instrument(skip_all)]
async fn dispatch_loop(
    db: DatabaseConnection,
    notification_ops: Arc<dyn PluginOps>,
    callback_base_url: String,
    settings: crate::settings::Settings,
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
            let channel_plugin =
                match notification_ops.notification_transport(&channel_model.channel_type) {
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

            // Build a generic settings bag from the cached settings.
            // Each plugin's `deliver()` extracts only the keys it needs.
            let settings_bag = build_settings_bag(&settings);

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
            let channel_plugin = channel_plugin.clone();
            tokio::spawn(async move {
                let Some(transport) = channel_plugin.as_notification_transport() else {
                    tracing::error!(
                        log_id = %log_id,
                        "plugin does not implement NotificationTransportPlugin"
                    );
                    return;
                };
                match transport
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
