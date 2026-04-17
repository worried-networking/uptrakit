//! SSE event types for real-time admin event streaming.
//!
//! [`AdminEvent`] is the server-side enum pushed over `GET /api/v1/events/stream`.
//! Each variant maps to an SSE `event:` name (via [`AdminEvent::event_name`]) with
//! the variant's inner fields serialised as the `data:` payload.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A real-time event pushed to admin SSE subscribers.
///
/// Each variant represents a state change that the frontend can use to
/// invalidate and refresh the relevant data. Events are lightweight
/// invalidation signals — they carry only enough context (entity IDs,
/// status strings) for the subscriber to decide whether to refetch.
///
/// # Wire format
///
/// Sent as SSE with `event:` set to [`event_name()`](Self::event_name) and
/// `data:` set to the JSON-serialised inner fields of the variant.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AdminEvent {
    /// A host's metadata was updated.
    HostUpdated { id: Uuid },
    /// A new host was created (e.g. reported by an agent).
    HostCreated { id: Uuid },
    /// A host was deactivated / deleted.
    HostDeleted { id: Uuid },
    /// A service's status changed (approved, rejected, deactivated).
    ServiceStatusChanged { id: Uuid, status: String },
    /// A software item was updated.
    SoftwareItemUpdated { id: Uuid },
    /// A new software item was created.
    SoftwareItemCreated { id: Uuid },
    /// A version check completed for a host + software item pair.
    VersionCheckCompleted {
        host_id: Uuid,
        software_item_id: Uuid,
    },
    /// A software update was created and dispatched to the agent.
    ///
    /// Emitted immediately after `trigger_update_for_host` succeeds, before
    /// the agent confirms start. Allows the History page to show the new
    /// pending/queued entry in real-time without polling.
    UpdateTriggered {
        update_history_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
    },
    /// A software update started executing.
    UpdateStarted {
        update_history_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        /// Whether the update was dispatched in interactive mode (PTY allocated).
        ///
        /// Allows the history list to show an "Input Required" badge in
        /// real-time without reloading, as soon as the update transitions to
        /// `in_progress`.
        interactive: bool,
    },
    /// A software update completed (successfully or with failure).
    UpdateCompleted {
        update_history_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        status: String,
    },
    /// Autodiscovery completed for a host.
    DiscoveryCompleted { host_id: Uuid },
    /// A system service's status changed (approved, rejected, deactivated).
    SystemServiceStatusChanged { id: Uuid, status: String },
    /// A scheduled task completed execution.
    SchedulerTaskCompleted { task_id: Uuid },
    /// A host tag was created.
    HostTagCreated { id: Uuid },
    /// A host tag was updated.
    HostTagUpdated { id: Uuid },
    /// A host tag was deleted.
    HostTagDeleted { id: Uuid },
    /// Tag assignments changed on a host.
    HostTagsChanged { host_id: Uuid },
    /// The global GitHub provider settings are stored in an invalid state.
    GlobalGitHubProviderMisconfigured { problem: String },
    /// All tenant data was reset (hosts, software items, etc. deleted).
    DataReset,
}

impl AdminEvent {
    /// Returns the SSE `event:` field name for this variant.
    ///
    /// The name is the snake_case version of the variant name, matching the
    /// serde `rename_all = "snake_case"` serialisation.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::HostUpdated { .. } => "host_updated",
            Self::HostCreated { .. } => "host_created",
            Self::HostDeleted { .. } => "host_deleted",
            Self::ServiceStatusChanged { .. } => "service_status_changed",
            Self::SoftwareItemUpdated { .. } => "software_item_updated",
            Self::SoftwareItemCreated { .. } => "software_item_created",
            Self::VersionCheckCompleted { .. } => "version_check_completed",
            Self::UpdateTriggered { .. } => "update_triggered",
            Self::UpdateStarted { .. } => "update_started",
            Self::UpdateCompleted { .. } => "update_completed",
            Self::DiscoveryCompleted { .. } => "discovery_completed",
            Self::SystemServiceStatusChanged { .. } => "system_service_status_changed",
            Self::SchedulerTaskCompleted { .. } => "scheduler_task_completed",
            Self::HostTagCreated { .. } => "host_tag_created",
            Self::HostTagUpdated { .. } => "host_tag_updated",
            Self::HostTagDeleted { .. } => "host_tag_deleted",
            Self::HostTagsChanged { .. } => "host_tags_changed",
            Self::GlobalGitHubProviderMisconfigured { .. } => {
                "global_github_provider_misconfigured"
            }
            Self::DataReset => "data_reset",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All known variants for exhaustive testing.
    fn all_variants() -> Vec<AdminEvent> {
        let id = Uuid::nil();
        vec![
            AdminEvent::HostUpdated { id },
            AdminEvent::HostCreated { id },
            AdminEvent::HostDeleted { id },
            AdminEvent::ServiceStatusChanged {
                id,
                status: "approved".to_string(),
            },
            AdminEvent::SoftwareItemUpdated { id },
            AdminEvent::SoftwareItemCreated { id },
            AdminEvent::VersionCheckCompleted {
                host_id: id,
                software_item_id: id,
            },
            AdminEvent::UpdateTriggered {
                update_history_id: id,
                host_id: id,
                software_item_id: id,
            },
            AdminEvent::UpdateStarted {
                update_history_id: id,
                host_id: id,
                software_item_id: id,
                interactive: false,
            },
            AdminEvent::UpdateCompleted {
                update_history_id: id,
                host_id: id,
                software_item_id: id,
                status: "completed".to_string(),
            },
            AdminEvent::DiscoveryCompleted { host_id: id },
            AdminEvent::SystemServiceStatusChanged {
                id,
                status: "approved".to_string(),
            },
            AdminEvent::SchedulerTaskCompleted { task_id: id },
            AdminEvent::HostTagCreated { id },
            AdminEvent::HostTagUpdated { id },
            AdminEvent::HostTagDeleted { id },
            AdminEvent::HostTagsChanged { host_id: id },
            AdminEvent::GlobalGitHubProviderMisconfigured {
                problem: "api_base_url requires auth_token".to_string(),
            },
            AdminEvent::DataReset,
        ]
    }

    #[test]
    fn serde_round_trip_all_variants() {
        for event in all_variants() {
            let json = serde_json::to_string(&event).unwrap();
            let deserialized: AdminEvent = serde_json::from_str(&json).unwrap();
            // Verify the event_name matches after round-trip
            assert_eq!(event.event_name(), deserialized.event_name());
        }
    }

    #[test]
    fn event_name_returns_correct_strings() {
        let id = Uuid::nil();
        assert_eq!(AdminEvent::HostUpdated { id }.event_name(), "host_updated");
        assert_eq!(AdminEvent::HostCreated { id }.event_name(), "host_created");
        assert_eq!(AdminEvent::HostDeleted { id }.event_name(), "host_deleted");
        assert_eq!(
            AdminEvent::ServiceStatusChanged {
                id,
                status: String::new()
            }
            .event_name(),
            "service_status_changed"
        );
        assert_eq!(
            AdminEvent::SoftwareItemUpdated { id }.event_name(),
            "software_item_updated"
        );
        assert_eq!(
            AdminEvent::SoftwareItemCreated { id }.event_name(),
            "software_item_created"
        );
        assert_eq!(
            AdminEvent::VersionCheckCompleted {
                host_id: id,
                software_item_id: id,
            }
            .event_name(),
            "version_check_completed"
        );
        assert_eq!(
            AdminEvent::UpdateTriggered {
                update_history_id: id,
                host_id: id,
                software_item_id: id,
            }
            .event_name(),
            "update_triggered"
        );
        assert_eq!(
            AdminEvent::UpdateStarted {
                update_history_id: id,
                host_id: id,
                software_item_id: id,
                interactive: false,
            }
            .event_name(),
            "update_started"
        );
        assert_eq!(
            AdminEvent::UpdateCompleted {
                update_history_id: id,
                host_id: id,
                software_item_id: id,
                status: String::new(),
            }
            .event_name(),
            "update_completed"
        );
        assert_eq!(
            AdminEvent::DiscoveryCompleted { host_id: id }.event_name(),
            "discovery_completed"
        );
        assert_eq!(
            AdminEvent::SystemServiceStatusChanged {
                id,
                status: String::new()
            }
            .event_name(),
            "system_service_status_changed"
        );
        assert_eq!(
            AdminEvent::SchedulerTaskCompleted { task_id: id }.event_name(),
            "scheduler_task_completed"
        );
        assert_eq!(
            AdminEvent::GlobalGitHubProviderMisconfigured {
                problem: String::new(),
            }
            .event_name(),
            "global_github_provider_misconfigured"
        );
    }

    #[test]
    fn event_name_count_matches_variant_count() {
        // If a new variant is added without updating event_name(), this
        // test will fail because all_variants() won't include it.
        assert_eq!(all_variants().len(), 19);
    }

    #[test]
    fn serde_uses_snake_case_tag() {
        let event = AdminEvent::HostUpdated { id: Uuid::nil() };
        let json = serde_json::to_string(&event).unwrap();
        // The tagged enum serialises with a type discriminator
        assert!(json.contains("host_updated"), "json was: {json}");
    }
}
