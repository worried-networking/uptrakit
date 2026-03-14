use uptrakit_notification_plugin_core::{DeliveryMessage, MessageAction, escape_html};
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
        vec![MessageAction::new(
            format!("Install {}", params.to_version),
            callback_url,
            token.to_string(),
        )]
    } else {
        vec![]
    };

    DeliveryMessage::new(title, body, Some(body_html), event_payload, actions)
}

fn build_content(event: &NotificationEvent) -> (String, String, String) {
    let host_label = event.host_name.as_deref().unwrap_or("unknown host");
    let software_label = event
        .software_item_name
        .as_deref()
        .unwrap_or("unknown software");

    // HTML-escaped versions of user-controlled labels for safe body_html interpolation.
    let host_html = escape_html(host_label);
    let software_html = escape_html(software_label);

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
            let installed_html = escape_html(installed);
            let latest_html = escape_html(latest_version);
            let body_html = format!(
                "A new version of <b>{software_html}</b> is available on <b>{host_html}</b>.\n\
                 Installed: <code>{installed_html}</code>\n\
                 Available: <code>{latest_html}</code>"
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
            let from_html = escape_html(from);
            let to_html = escape_html(to_version);
            let body_html = format!(
                "<b>{software_html}</b> on <b>{host_html}</b> has been updated successfully.\n\
                 From: <code>{from_html}</code>\n\
                 To: <code>{to_html}</code>"
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
            let from_html = escape_html(from);
            let to_html = escape_html(to_version);
            let err_html = escape_html(err);
            let body_html = format!(
                "<b>{software_html}</b> on <b>{host_html}</b> failed to update.\n\
                 From: <code>{from_html}</code>\n\
                 To: <code>{to_html}</code>\n\
                 Error: <code>{err_html}</code>"
            );
            (title, body, body_html)
        }
        NotificationEventDetails::NewSoftwareDiscovered { discovered_count } => {
            let title = format!("New Software Discovered on {host_label}");
            let body =
                format!("{discovered_count} new software item(s) discovered on {host_label}.");
            let body_html = format!(
                "<b>{discovered_count}</b> new software item(s) discovered on <b>{host_html}</b>."
            );
            (title, body, body_html)
        }
        NotificationEventDetails::NewServiceEnrolled { service_label, .. } => {
            let title = format!("New Service Enrolled: {service_label}");
            let body = format!("Service \"{service_label}\" has been enrolled and approved.");
            let service_html = escape_html(service_label);
            let body_html =
                format!("Service <b>{service_html}</b> has been enrolled and approved.");
            (title, body, body_html)
        }
        NotificationEventDetails::CaRotated { reason } => {
            let title = "CA Certificate Rotated".to_string();
            let body = format!("The CA certificate has been rotated. Reason: {reason}");
            let reason_html = escape_html(reason);
            let body_html =
                format!("The CA certificate has been rotated. Reason: <code>{reason_html}</code>");
            (title, body, body_html)
        }
        NotificationEventDetails::BatchUpdateCompleted {
            total_count,
            completed_count,
            ..
        } => {
            let title = "Batch Update Completed".to_string();
            let body = format!(
                "Batch update completed successfully. {completed_count}/{total_count} updates completed."
            );
            let body_html = format!(
                "Batch update completed successfully. <b>{completed_count}/{total_count}</b> updates completed."
            );
            (title, body, body_html)
        }
        NotificationEventDetails::BatchUpdatePartiallyCompleted {
            total_count,
            completed_count,
            failed_count,
            ..
        } => {
            let title = "Batch Update Partially Completed".to_string();
            let body = format!(
                "Batch update partially completed. {completed_count}/{total_count} succeeded, {failed_count} failed."
            );
            let body_html = format!(
                "Batch update partially completed. <b>{completed_count}/{total_count}</b> succeeded, <b>{failed_count}</b> failed."
            );
            (title, body, body_html)
        }
        NotificationEventDetails::StdinAttention { hint, .. } => {
            let title = "Update Waiting for Input".to_string();
            let hint_msg = hint.as_deref().unwrap_or("No additional details available");
            let body = format!("An interactive update is waiting for input. Hint: {hint_msg}");
            let body_html = format!(
                "An interactive update is waiting for input. Hint: <b>{}</b>",
                escape_html(hint_msg)
            );
            (title, body, body_html)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_update_available_escapes_html_in_body_html() {
        let event = NotificationEvent {
            tenant_id: Uuid::nil(),
            host_id: Some(Uuid::nil()),
            host_name: Some("<img src=x onerror=alert(1)>".to_string()),
            software_item_id: Some(Uuid::nil()),
            software_item_name: Some("<script>alert('xss')</script>".to_string()),
            plugin_type: None,
            details: NotificationEventDetails::UpdateAvailable {
                installed_version: Some("1.0 & \"old\"".to_string()),
                latest_version: "2.0 <beta>".to_string(),
            },
        };

        let msg = build_delivery_message(&event, None, "https://example.com", Uuid::nil());

        // body_html must not contain raw HTML tags from user input
        let html = msg.body_html.as_deref().unwrap();
        assert!(
            !html.contains("<script>"),
            "body_html must escape <script> tags, got: {html}"
        );
        assert!(
            !html.contains("<img"),
            "body_html must escape <img> tags, got: {html}"
        );
        assert!(
            html.contains("&lt;script&gt;"),
            "body_html must contain escaped script tag, got: {html}"
        );
        assert!(
            html.contains("&amp;"),
            "body_html must escape ampersands, got: {html}"
        );

        // Plain text body must preserve the original values unescaped
        assert!(msg.body.contains("<script>alert('xss')</script>"));
        assert!(msg.body.contains("<img src=x onerror=alert(1)>"));
    }

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
