use std::time::Duration;

use uuid::Uuid;

/// Stream name in JetStream.
pub const STREAM_NAME: &str = "UPTRAKIT_EVENTS";

/// Subject prefix for all events.
pub const SUBJECT_PREFIX: &str = "uptrakit.events";

/// Maximum age for messages in the stream (24 hours).
pub const STREAM_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Determine the NATS subject for a message based on routing metadata.
pub fn determine(target_service_id: Option<Uuid>, target_capability: Option<&str>) -> String {
    match (target_service_id, target_capability) {
        (Some(id), _) => format!("{SUBJECT_PREFIX}.service.{id}"),
        (None, Some(cap)) => {
            if cap == "controller" {
                format!("{SUBJECT_PREFIX}.controller")
            } else {
                format!("{SUBJECT_PREFIX}.capability.{cap}")
            }
        }
        (None, None) => format!("{SUBJECT_PREFIX}.broadcast"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determine_broadcast() {
        assert_eq!(determine(None, None), "uptrakit.events.broadcast");
    }

    #[test]
    fn determine_service() {
        let id = Uuid::nil();
        assert_eq!(
            determine(Some(id), None),
            format!("uptrakit.events.service.{id}")
        );
    }

    #[test]
    fn determine_capability() {
        assert_eq!(
            determine(None, Some("mqtt_bridge")),
            "uptrakit.events.capability.mqtt_bridge"
        );
    }

    #[test]
    fn determine_controller() {
        assert_eq!(
            determine(None, Some("controller")),
            "uptrakit.events.controller"
        );
    }

    #[test]
    fn determine_service_takes_precedence_over_capability() {
        let id = Uuid::nil();
        assert_eq!(
            determine(Some(id), Some("mqtt_bridge")),
            format!("uptrakit.events.service.{id}")
        );
    }
}
