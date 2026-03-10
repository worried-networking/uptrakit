use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::mpsc;
use uuid::Uuid;

use uptrakit_notification_plugin_registry::NotificationOps;

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
        notification_ops: Arc<dyn NotificationOps>,
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

/// Convert web-api's SmtpSettingsSnapshot to the email plugin's SmtpSettingsSnapshot
/// and merge global SMTP settings into a per-channel email config object.
#[cfg(feature = "notifications-email")]
#[tracing::instrument(skip_all)]
fn merge_smtp_into_config(
    smtp: &crate::settings::SmtpSettingsSnapshot,
    config: serde_json::Value,
) -> serde_json::Value {
    let plugin_smtp = to_plugin_smtp_snapshot(smtp);
    uptrakit_notification_plugin_registry::merge_smtp_into_config(&plugin_smtp, config)
}

/// Public (crate-visible) re-export for use in route handlers (e.g. `test_channel`).
#[cfg(feature = "notifications-email")]
#[tracing::instrument(skip_all)]
pub(crate) fn merge_smtp_into_config_pub(
    smtp: &crate::settings::SmtpSettingsSnapshot,
    config: serde_json::Value,
) -> serde_json::Value {
    merge_smtp_into_config(smtp, config)
}

/// Convert web-api's SmtpSettingsSnapshot to the email plugin's version.
#[cfg(feature = "notifications-email")]
fn to_plugin_smtp_snapshot(
    smtp: &crate::settings::SmtpSettingsSnapshot,
) -> uptrakit_notification_plugin_registry::SmtpSettingsSnapshot {
    uptrakit_notification_plugin_registry::SmtpSettingsSnapshot {
        host: smtp.host.clone(),
        port: smtp.port,
        username: smtp.username.clone(),
        password: smtp.password.clone(),
        from_address: smtp.from_address.clone(),
        from_name: smtp.from_name.clone(),
        tls_mode: smtp.tls_mode.clone(),
    }
}

#[tracing::instrument(skip_all)]
async fn dispatch_loop(
    db: DatabaseConnection,
    notification_ops: Arc<dyn NotificationOps>,
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
            let channel_impl = match notification_ops.get(&channel_model.channel_type) {
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
                #[cfg(feature = "notifications-email")]
                {
                    let smtp = settings.smtp();
                    if !smtp.is_configured() {
                        tracing::warn!(
                            channel_id = %channel_model.id,
                            "skipping email notification: SMTP settings not configured"
                        );
                        continue;
                    }
                    merge_smtp_into_config(&smtp, config_json)
                }
                #[cfg(not(feature = "notifications-email"))]
                {
                    // Email plugin not compiled in — the channel won't be in the
                    // registry so delivery will be skipped below, but handle it
                    // defensively.
                    let _ = &settings;
                    tracing::warn!(
                        channel_id = %channel_model.id,
                        "email notification requested but email plugin is not enabled"
                    );
                    continue;
                }
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

#[cfg(all(test, feature = "notifications-email"))]
mod tests {
    use super::*;
    use crate::settings::SmtpSettingsSnapshot;

    fn make_smtp(
        host: Option<&str>,
        port: Option<u16>,
        from: Option<&str>,
    ) -> SmtpSettingsSnapshot {
        SmtpSettingsSnapshot {
            host: host.map(|s| s.to_string()),
            port,
            username: None,
            password: None,
            from_address: from.map(|s| s.to_string()),
            from_name: None,
            tls_mode: "starttls".to_string(),
        }
    }

    // ── merge_smtp_into_config ────────────────────────────────────────────────

    /// The SMTP host and default port (587) are injected into the config object.
    #[test]
    fn merge_smtp_sets_host_and_default_port() {
        let smtp = make_smtp(Some("mail.example.com"), None, Some("noreply@example.com"));
        let config = serde_json::json!({ "to_addresses": ["user@test.local"] });
        let merged = merge_smtp_into_config(&smtp, config);

        assert_eq!(merged["smtp_host"], "mail.example.com");
        assert_eq!(
            merged["smtp_port"], 587,
            "default port must be 587 when port is None"
        );
        assert_eq!(merged["from_address"], "noreply@example.com");
        // Original fields are preserved.
        assert!(merged["to_addresses"].is_array());
    }

    /// When a non-default port is set it is used verbatim.
    #[test]
    fn merge_smtp_uses_explicit_port() {
        let smtp = make_smtp(
            Some("smtp.corp.internal"),
            Some(465),
            Some("alerts@corp.internal"),
        );
        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&smtp, config);

        assert_eq!(merged["smtp_port"], 465);
    }

    /// Username, password, from_name, and tls_mode are all propagated.
    #[test]
    fn merge_smtp_propagates_all_optional_fields() {
        let mut smtp = make_smtp(Some("smtp.example.com"), None, Some("from@example.com"));
        smtp.username = Some("smtpuser".to_string());
        smtp.password = Some("secret".to_string());
        smtp.from_name = Some("Uptrakit Alerts".to_string());
        smtp.tls_mode = "tls".to_string();

        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&smtp, config);

        assert_eq!(merged["smtp_username"], "smtpuser");
        assert_eq!(merged["smtp_password"], "secret");
        assert_eq!(merged["from_name"], "Uptrakit Alerts");
        assert_eq!(merged["tls_mode"], "tls");
    }

    /// When no host is set the `smtp_host` key is not inserted.
    #[test]
    fn merge_smtp_omits_host_when_none() {
        let smtp = make_smtp(None, None, None);
        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&smtp, config);

        assert!(
            merged.get("smtp_host").is_none(),
            "smtp_host must not be set when host is None"
        );
    }

    /// Non-email channel configs are returned unchanged by the caller-side guard
    /// in `dispatch_loop`.  We test this by verifying `merge_smtp_into_config`
    /// does not inspect the channel type — the guard is purely at the call site.
    /// The function is only called for email channels, so this test documents the
    /// assumption: passing a non-email config object still works (no panic).
    #[test]
    fn merge_smtp_works_on_any_json_object() {
        let smtp = make_smtp(Some("smtp.example.com"), None, Some("from@example.com"));
        // A webhook-style config with no email fields.
        let config = serde_json::json!({ "url": "https://webhook.example.com" });
        let merged = merge_smtp_into_config(&smtp, config);

        // SMTP fields are merged in but the original webhook field is still there.
        assert_eq!(merged["url"], "https://webhook.example.com");
        assert_eq!(merged["smtp_host"], "smtp.example.com");
    }
}
