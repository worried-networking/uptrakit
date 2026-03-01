use serde::{Deserialize, Serialize};
use uuid::Uuid;

use uptrakit_web_api_types::notifications::NotificationEventType;

/// Channel-agnostic notification event produced by event handlers.
///
/// The `details` variant is the single source of truth for both the
/// event type and event-specific data.
#[derive(Clone, Debug)]
pub struct NotificationEvent {
    pub tenant_id: Uuid,
    pub host_id: Option<Uuid>,
    pub host_name: Option<String>,
    pub software_item_id: Option<Uuid>,
    pub software_item_name: Option<String>,
    pub plugin_type: Option<String>,
    pub details: NotificationEventDetails,
}

/// Event-specific data. Each variant carries its own typed payload.
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationEventDetails {
    UpdateAvailable {
        installed_version: Option<String>,
        latest_version: String,
    },
    UpdateCompleted {
        from_version: Option<String>,
        to_version: String,
        update_history_id: Uuid,
    },
    UpdateFailed {
        from_version: Option<String>,
        to_version: String,
        error: Option<String>,
        update_history_id: Uuid,
    },
    NewSoftwareDiscovered {
        discovered_count: u32,
    },
    NewServiceEnrolled {
        service_id: Uuid,
        service_label: String,
    },
    CaRotated {
        reason: String,
    },
}

/// Parameters for actionable notifications (only `UpdateAvailable`).
#[derive(Clone, Debug)]
pub struct ActionParams {
    pub software_item_id: Uuid,
    pub host_id: Uuid,
    pub to_version: String,
}

impl NotificationEvent {
    /// Derive the event type from the details variant.
    pub fn event_type(&self) -> NotificationEventType {
        match &self.details {
            NotificationEventDetails::UpdateAvailable { .. } => {
                NotificationEventType::UpdateAvailable
            }
            NotificationEventDetails::UpdateCompleted { .. } => {
                NotificationEventType::UpdateCompleted
            }
            NotificationEventDetails::UpdateFailed { .. } => NotificationEventType::UpdateFailed,
            NotificationEventDetails::NewSoftwareDiscovered { .. } => {
                NotificationEventType::NewSoftwareDiscovered
            }
            NotificationEventDetails::NewServiceEnrolled { .. } => {
                NotificationEventType::NewServiceEnrolled
            }
            NotificationEventDetails::CaRotated { .. } => NotificationEventType::CaRotated,
        }
    }

    /// Derive action parameters from the details variant.
    /// Only `UpdateAvailable` produces actionable notifications.
    pub fn action_params(&self) -> Option<ActionParams> {
        match &self.details {
            NotificationEventDetails::UpdateAvailable { latest_version, .. } => {
                let software_item_id = self.software_item_id?;
                let host_id = self.host_id?;
                Some(ActionParams {
                    software_item_id,
                    host_id,
                    to_version: latest_version.clone(),
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_update_available() {
        let event = NotificationEvent {
            tenant_id: Uuid::nil(),
            host_id: Some(Uuid::nil()),
            host_name: Some("test-host".to_string()),
            software_item_id: Some(Uuid::nil()),
            software_item_name: Some("nginx".to_string()),
            plugin_type: None,
            details: NotificationEventDetails::UpdateAvailable {
                installed_version: Some("1.0".to_string()),
                latest_version: "2.0".to_string(),
            },
        };
        assert_eq!(event.event_type(), NotificationEventType::UpdateAvailable);
        assert!(event.action_params().is_some());
    }

    #[test]
    fn event_type_update_completed() {
        let event = NotificationEvent {
            tenant_id: Uuid::nil(),
            host_id: None,
            host_name: None,
            software_item_id: None,
            software_item_name: None,
            plugin_type: None,
            details: NotificationEventDetails::UpdateCompleted {
                from_version: Some("1.0".to_string()),
                to_version: "2.0".to_string(),
                update_history_id: Uuid::nil(),
            },
        };
        assert_eq!(event.event_type(), NotificationEventType::UpdateCompleted);
        assert!(event.action_params().is_none());
    }

    #[test]
    fn action_params_requires_host_and_software_item() {
        let event = NotificationEvent {
            tenant_id: Uuid::nil(),
            host_id: None, // Missing host_id
            host_name: None,
            software_item_id: Some(Uuid::nil()),
            software_item_name: None,
            plugin_type: None,
            details: NotificationEventDetails::UpdateAvailable {
                installed_version: None,
                latest_version: "2.0".to_string(),
            },
        };
        assert!(event.action_params().is_none());
    }

    #[test]
    fn event_details_serde_round_trip() {
        let details = NotificationEventDetails::UpdateFailed {
            from_version: Some("1.0".to_string()),
            to_version: "2.0".to_string(),
            error: Some("download failed".to_string()),
            update_history_id: Uuid::nil(),
        };
        let json = serde_json::to_string(&details).expect("serialize");
        let parsed: NotificationEventDetails = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            NotificationEventDetails::UpdateFailed { error, .. } => {
                assert_eq!(error.as_deref(), Some("download failed"));
            }
            _ => panic!("wrong variant"),
        }
    }
}
