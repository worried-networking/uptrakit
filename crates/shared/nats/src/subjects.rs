use std::time::Duration;

use uuid::Uuid;

/// Stream name in JetStream.
pub const STREAM_NAME: &str = "UPTRAKIT_EVENTS";

/// Subject prefix for all events.
pub const SUBJECT_PREFIX: &str = "uptrakit.events";

/// Maximum age for messages in the stream (24 hours).
pub const STREAM_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Subject prefix for ephemeral (non-JetStream) batch-progress events.
///
/// Each in-progress batch publishes transient progress events to
/// `uptrakit.batch_progress.<batch_id>`.  These events are NOT persisted in
/// JetStream — use core NATS `publish`/`subscribe` for cross-instance fan-out.
pub const BATCH_PROGRESS_PREFIX: &str = "uptrakit.batch_progress";

/// Build the core NATS subject for a given batch's progress events.
///
/// Format: `uptrakit.batch_progress.<batch_id>`
pub fn batch_progress(batch_id: &Uuid) -> String {
    format!("{BATCH_PROGRESS_PREFIX}.{batch_id}")
}

/// The capability routing string that directs a message to the controller.
///
/// Used in `determine()` to distinguish controller-bound messages from
/// capability-bound messages. Extracted as a named constant to avoid
/// magic string literals scattered across routing logic.
const CONTROLLER_ROUTING_CAP: &str = "controller";

/// Determine the NATS subject for a message based on routing metadata.
pub fn determine(target_service_id: Option<Uuid>, target_capability: Option<&str>) -> String {
    match (target_service_id, target_capability) {
        (Some(id), _) => format!("{SUBJECT_PREFIX}.service.{id}"),
        (None, Some(cap)) => {
            if cap == CONTROLLER_ROUTING_CAP {
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
    fn batch_progress_subject_format() {
        let id = Uuid::nil();
        assert_eq!(batch_progress(&id), format!("uptrakit.batch_progress.{id}"));
    }

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
            determine(None, Some("update_tracking")),
            "uptrakit.events.capability.update_tracking"
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
            determine(Some(id), Some("update_tracking")),
            format!("uptrakit.events.service.{id}")
        );
    }
}
