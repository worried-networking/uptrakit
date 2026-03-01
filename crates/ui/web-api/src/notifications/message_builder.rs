use uptrakit_notification_channels::{DeliveryMessage, MessageAction};
use uuid::Uuid;

use super::events::{NotificationEvent, NotificationEventDetails};

/// Build a channel-agnostic `DeliveryMessage` from a `NotificationEvent`.
///
/// This is the single translation point between the event model and
/// the delivery model. Channel implementations never see `NotificationEvent`.
pub fn build_delivery_message(
    event: &NotificationEvent,
    action_token: Option<Uuid>,
    callback_base_url: &str,
    channel_id: Uuid,
) -> DeliveryMessage {
    let (title, body, body_html) = build_content(event);
    let event_payload = serde_json::to_value(&event.details).unwrap_or_default();

    let actions = if let (Some(params), Some(token)) = (event.action_params(), action_token) {
        let callback_url = format!(
            "{}/api/v1/notifications/callback/telegram/{}",
            callback_base_url.trim_end_matches('/'),
            channel_id
        );
        vec![MessageAction {
            label: format!("Install {}", params.to_version),
            callback_url,
            token: token.to_string(),
        }]
    } else {
        vec![]
    };

    DeliveryMessage {
        title,
        body,
        body_html: Some(body_html),
        event_payload,
        actions,
    }
}

fn build_content(event: &NotificationEvent) -> (String, String, String) {
    let host_label = event.host_name.as_deref().unwrap_or("unknown host");
    let software_label = event
        .software_item_name
        .as_deref()
        .unwrap_or("unknown software");

    match &event.details {
        NotificationEventDetails::UpdateAvailable {
            installed_version,
            latest_version,
        } => {
            let title = format!("Update Available: {software_label}");
            let installed = installed_version.as_deref().unwrap_or("unknown");
            let body = format!(
                "A new version of {software_label} is available on {host_label}.\n\
                 Installed: {installed}\n\
                 Available: {latest_version}"
            );
            let body_html = format!(
                "A new version of <b>{software_label}</b> is available on <b>{host_label}</b>.\n\
                 Installed: <code>{installed}</code>\n\
                 Available: <code>{latest_version}</code>"
            );
            (title, body, body_html)
        }
        NotificationEventDetails::UpdateCompleted {
            from_version,
            to_version,
            ..
        } => {
            let title = format!("Update Completed: {software_label}");
            let from = from_version.as_deref().unwrap_or("unknown");
            let body = format!(
                "{software_label} on {host_label} has been updated successfully.\n\
                 From: {from}\n\
                 To: {to_version}"
            );
            let body_html = format!(
                "<b>{software_label}</b> on <b>{host_label}</b> has been updated successfully.\n\
                 From: <code>{from}</code>\n\
                 To: <code>{to_version}</code>"
            );
            (title, body, body_html)
        }
        NotificationEventDetails::UpdateFailed {
            from_version,
            to_version,
            error,
            ..
        } => {
            let title = format!("Update Failed: {software_label}");
            let from = from_version.as_deref().unwrap_or("unknown");
            let err = error.as_deref().unwrap_or("no error details");
            let body = format!(
                "{software_label} on {host_label} failed to update.\n\
                 From: {from}\n\
                 To: {to_version}\n\
                 Error: {err}"
            );
            let body_html = format!(
                "<b>{software_label}</b> on <b>{host_label}</b> failed to update.\n\
                 From: <code>{from}</code>\n\
                 To: <code>{to_version}</code>\n\
                 Error: <code>{err}</code>"
            );
            (title, body, body_html)
        }
        NotificationEventDetails::NewSoftwareDiscovered { discovered_count } => {
            let title = format!("New Software Discovered on {host_label}");
            let body =
                format!("{discovered_count} new software item(s) discovered on {host_label}.");
            let body_html = format!(
                "<b>{discovered_count}</b> new software item(s) discovered on <b>{host_label}</b>."
            );
            (title, body, body_html)
        }
        NotificationEventDetails::NewServiceEnrolled {
            service_label, ..
        } => {
            let title = format!("New Service Enrolled: {service_label}");
            let body = format!("Service \"{service_label}\" has been enrolled and approved.");
            let body_html =
                format!("Service <b>{service_label}</b> has been enrolled and approved.");
            (title, body, body_html)
        }
        NotificationEventDetails::CaRotated { reason } => {
            let title = "CA Certificate Rotated".to_string();
            let body = format!("The CA certificate has been rotated. Reason: {reason}");
            let body_html =
                format!("The CA certificate has been rotated. Reason: <code>{reason}</code>");
            (title, body, body_html)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_update_available_message() {
        let event = NotificationEvent {
            tenant_id: Uuid::nil(),
            host_id: Some(Uuid::nil()),
            host_name: Some("web-server".to_string()),
            software_item_id: Some(Uuid::nil()),
            software_item_name: Some("nginx".to_string()),
            plugin_type: None,
            details: NotificationEventDetails::UpdateAvailable {
                installed_version: Some("1.24.0".to_string()),
                latest_version: "1.25.0".to_string(),
            },
        };

        let msg = build_delivery_message(
            &event,
            Some(Uuid::nil()),
            "https://example.com",
            Uuid::nil(),
        );

        assert_eq!(msg.title, "Update Available: nginx");
        assert!(msg.body.contains("1.25.0"));
        assert_eq!(msg.actions.len(), 1);
        assert!(msg.actions[0].label.contains("1.25.0"));
    }

    #[test]
    fn build_ca_rotated_message_has_no_actions() {
        let event = NotificationEvent {
            tenant_id: Uuid::nil(),
            host_id: None,
            host_name: None,
            software_item_id: None,
            software_item_name: None,
            plugin_type: None,
            details: NotificationEventDetails::CaRotated {
                reason: "scheduled rotation".to_string(),
            },
        };

        let msg = build_delivery_message(&event, None, "https://example.com", Uuid::nil());
        assert_eq!(msg.title, "CA Certificate Rotated");
        assert!(msg.actions.is_empty());
    }
}
