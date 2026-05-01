use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::mpsc;
use uuid::Uuid;

use uptrakit_plugin_infrastructure_registry::{EmailSmtpSettings, PluginOps};

use uptrakit_notification_delivery::NotificationEvent;

/// Bounded capacity for the notification dispatcher channel.
///
/// Limits memory consumption under bulk update completions that generate many
/// simultaneous notification events. When the channel is full, events are
/// dropped and a warning is logged (fire-and-forget semantics).
const NOTIFICATION_DISPATCHER_CAPACITY: usize = 4096;
const SMTP_PREFIX: &str = "smtp.";
const GLOBAL_SMTP_PREFIX: &str = "global_smtp.";
const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";
const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

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
    let tenant_smtp = typed_smtp_settings_or_empty(
        {
            let raw = uptrakit_shared_db::raw_settings::load_settings_by_prefix(
                db,
                tenant_id,
                SMTP_PREFIX,
            )
            .await;
            raw.and_then(|r| {
                uptrakit_shared_db::raw_settings::decode_prefixed_settings(SMTP_PREFIX, &r)
            })
        },
        "tenant",
        Some(tenant_id),
        SMTP_PASSWORD_AAD,
    );

    let global_smtp = typed_smtp_settings_or_empty(
        {
            let raw = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
                db,
                GLOBAL_SMTP_PREFIX,
            )
            .await;
            raw.and_then(|r| {
                uptrakit_shared_db::raw_settings::decode_prefixed_settings(GLOBAL_SMTP_PREFIX, &r)
            })
        },
        "global",
        None,
        GLOBAL_SMTP_PASSWORD_AAD,
    );

    let global_telegram = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
        db,
        GLOBAL_TELEGRAM_PREFIX,
    )
    .await
    .unwrap_or_default();

    let mut global = smtp_settings_to_prefixed_map(GLOBAL_SMTP_PREFIX, &global_smtp);
    for (k, v) in &global_telegram {
        global.insert(k.clone(), v.clone());
    }

    let tenant_map = smtp_settings_to_prefixed_map(SMTP_PREFIX, &tenant_smtp);

    serde_json::json!({ "tenant": tenant_map, "global": global })
}

fn typed_smtp_settings_or_empty(
    result: uptrakit_shared_db::raw_settings::Result<EmailSmtpSettings>,
    scope: &'static str,
    tenant_id: Option<Uuid>,
    password_aad: &str,
) -> EmailSmtpSettings {
    match result {
        Ok(settings) => normalize_smtp_settings(settings, password_aad, scope, tenant_id),
        Err(error) => {
            if let Some(tenant_id) = tenant_id {
                tracing::warn!(
                    error = ?error,
                    %tenant_id,
                    scope,
                    "failed to load typed SMTP settings for notification dispatch; using empty fallback"
                );
            } else {
                tracing::warn!(
                    error = ?error,
                    scope,
                    "failed to load typed SMTP settings for notification dispatch; using empty fallback"
                );
            }
            EmailSmtpSettings::default()
        }
    }
}

fn normalize_smtp_settings(
    settings: EmailSmtpSettings,
    password_aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> EmailSmtpSettings {
    EmailSmtpSettings {
        host: normalize_non_empty_string(settings.host),
        port: settings.port,
        username: normalize_non_empty_string(settings.username),
        password: decode_smtp_password(settings.password, password_aad, scope, tenant_id),
        from_address: normalize_non_empty_string(settings.from_address),
        from_name: normalize_non_empty_string(settings.from_name),
        tls_mode: normalize_non_empty_string(settings.tls_mode),
        helo_host: normalize_non_empty_string(settings.helo_host),
    }
}

fn normalize_non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn decode_smtp_password(
    value: Option<String>,
    aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> Option<String> {
    let raw = normalize_non_empty_string(value)?;

    if uptrakit_crypto::is_encrypted(&raw) {
        return match uptrakit_crypto::decrypt_str(&raw, aad) {
            Ok(value) => normalize_non_empty_string(Some(value)),
            Err(error) => {
                if let Some(tenant_id) = tenant_id {
                    tracing::warn!(
                        error = ?error,
                        %tenant_id,
                        scope,
                        "failed to decrypt SMTP password for notification dispatch; using empty fallback"
                    );
                } else {
                    tracing::warn!(
                        error = ?error,
                        scope,
                        "failed to decrypt SMTP password for notification dispatch; using empty fallback"
                    );
                }
                None
            }
        };
    }

    Some(raw)
}

fn smtp_settings_to_prefixed_map(
    prefix: &str,
    settings: &EmailSmtpSettings,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();

    insert_prefixed_string(&mut map, prefix, "host", settings.host.as_deref());
    insert_prefixed_u16(&mut map, prefix, "port", settings.port);
    insert_prefixed_string(&mut map, prefix, "username", settings.username.as_deref());
    insert_prefixed_string(&mut map, prefix, "password", settings.password.as_deref());
    insert_prefixed_string(
        &mut map,
        prefix,
        "from_address",
        settings.from_address.as_deref(),
    );
    insert_prefixed_string(&mut map, prefix, "from_name", settings.from_name.as_deref());
    insert_prefixed_string(&mut map, prefix, "tls_mode", settings.tls_mode.as_deref());
    insert_prefixed_string(&mut map, prefix, "helo_host", settings.helo_host.as_deref());

    map
}

fn insert_prefixed_string(
    map: &mut serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    suffix: &str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        map.insert(
            format!("{prefix}{suffix}"),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn insert_prefixed_u16(
    map: &mut serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    suffix: &str,
    value: Option<u16>,
) {
    if let Some(value) = value {
        map.insert(format!("{prefix}{suffix}"), serde_json::json!(value));
    }
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
            let message = match uptrakit_notification_delivery::build_delivery_message(
                &event,
                action_token,
                &callback_base_url,
                &channel_model.channel_type,
                channel_model.id,
            ) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        channel_id = %channel_model.id,
                        "failed to build delivery message"
                    );
                    continue;
                }
            };

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
                match uptrakit_notification_delivery::deliver(
                    channel_transport,
                    &config_json,
                    &settings_bag,
                    &message,
                )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_smtp_settings_or_empty_returns_default_on_load_error() {
        let tenant_id = Uuid::now_v7();
        let settings = typed_smtp_settings_or_empty(
            Err(rootcause::report!(
                uptrakit_shared_db::raw_settings::RawSettingsError::Decode("boom".into())
            )),
            "tenant",
            Some(tenant_id),
            SMTP_PASSWORD_AAD,
        );

        assert_eq!(settings, EmailSmtpSettings::default());
    }
}
