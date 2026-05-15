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
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for event type strings received from a newer
/// server that this client does not yet recognise. Serde deserialization is
/// infallible: an unknown variant becomes `Other(variant_name)` rather than a
/// parse error, allowing older consumers to survive rolling upgrades without
/// failing.
#[derive(Clone, Debug, Serialize)]
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
        /// Trigger status: "pending" (agent connected) or "queued" (agent offline).
        status: String,
    },
    /// Controller pre-update protection started for a software update.
    ///
    /// Emitted by the orchestrator when protection (snapshot/backup) begins.
    /// The frontend transitions the update record to In Progress state on receipt.
    UpdateProtectionStarted {
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
    /// The surface provider registry changed (provider joined or left).
    ///
    /// Carries no payload — coarse invalidation signal. The frontend re-fetches
    /// `GET /api/v1/surfaces` and provider availability on receipt.
    SurfacesChanged,
    /// An unknown event variant received from a newer peer.
    ///
    /// The inner string is the raw variant name as it appeared on the wire.
    /// Deserialization is infallible — unknown variants are captured here rather
    /// than causing a parse error, so older consumers survive rolling upgrades.
    Other(String),
}

impl AdminEvent {
    /// Returns the SSE `event:` field name for this variant.
    ///
    /// The name is the snake_case version of the variant name, matching the
    /// serde `rename_all = "snake_case"` serialisation. For `Other`, returns
    /// the raw variant string as received on the wire.
    pub fn event_name(&self) -> &str {
        match self {
            Self::HostUpdated { .. } => "host_updated",
            Self::HostCreated { .. } => "host_created",
            Self::HostDeleted { .. } => "host_deleted",
            Self::ServiceStatusChanged { .. } => "service_status_changed",
            Self::SoftwareItemUpdated { .. } => "software_item_updated",
            Self::SoftwareItemCreated { .. } => "software_item_created",
            Self::VersionCheckCompleted { .. } => "version_check_completed",
            Self::UpdateTriggered { .. } => "update_triggered",
            Self::UpdateProtectionStarted { .. } => "update_protection_started",
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
            Self::SurfacesChanged => "surfaces_changed",
            Self::Other(v) => v.as_str(),
        }
    }
}

// ── Custom Deserialize for wire forward-compatibility ─────────────────────────
//
// `AdminEvent` uses serde's externally-tagged format (default for enums):
//   - struct variants:  `{"host_updated": {"id": "..."}}`
//   - unit variants:    `"data_reset"` (bare string)
//
// We cannot use `#[derive(Deserialize)]` directly because serde's derived impl
// would return an error for unknown variant keys. Instead we deserialize into a
// `serde_json::Value` first, extract the variant key, and if unknown return
// `Other(key)` rather than failing. This preserves rolling-upgrade safety.
impl<'de> Deserialize<'de> for AdminEvent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;

        // Unit variants serialize as bare strings.
        if let serde_json::Value::String(ref s) = value {
            return match s.as_str() {
                "data_reset" => Ok(Self::DataReset),
                "surfaces_changed" => Ok(Self::SurfacesChanged),
                other => {
                    tracing::debug!(variant = other, "received unknown AdminEvent variant");
                    Ok(Self::Other(other.to_string()))
                }
            };
        }

        // Struct variants serialize as `{"variant_name": {...fields...}}`.
        let obj = match value {
            serde_json::Value::Object(map) => map,
            _ => {
                return Err(serde::de::Error::custom(
                    "expected string or object for AdminEvent",
                ));
            }
        };

        let (key, inner) = match obj.into_iter().next() {
            Some(pair) => pair,
            None => {
                return Err(serde::de::Error::custom(
                    "expected non-empty object for AdminEvent",
                ));
            }
        };

        match key.as_str() {
            "host_updated" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                }
                let Inner { id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::HostUpdated { id })
            }
            "host_created" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                }
                let Inner { id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::HostCreated { id })
            }
            "host_deleted" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                }
                let Inner { id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::HostDeleted { id })
            }
            "service_status_changed" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                    status: String,
                }
                let Inner { id, status } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::ServiceStatusChanged { id, status })
            }
            "software_item_updated" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                }
                let Inner { id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::SoftwareItemUpdated { id })
            }
            "software_item_created" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                }
                let Inner { id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::SoftwareItemCreated { id })
            }
            "version_check_completed" => {
                #[derive(Deserialize)]
                struct Inner {
                    host_id: Uuid,
                    software_item_id: Uuid,
                }
                let Inner {
                    host_id,
                    software_item_id,
                } = serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::VersionCheckCompleted {
                    host_id,
                    software_item_id,
                })
            }
            "update_triggered" => {
                fn default_pending_status() -> String {
                    "pending".into()
                }
                #[derive(Deserialize)]
                struct Inner {
                    update_history_id: Uuid,
                    host_id: Uuid,
                    software_item_id: Uuid,
                    #[serde(default = "default_pending_status")]
                    status: String,
                }
                let Inner {
                    update_history_id,
                    host_id,
                    software_item_id,
                    status,
                } = serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::UpdateTriggered {
                    update_history_id,
                    host_id,
                    software_item_id,
                    status,
                })
            }
            "update_protection_started" => {
                #[derive(Deserialize)]
                struct Inner {
                    update_history_id: Uuid,
                    host_id: Uuid,
                    software_item_id: Uuid,
                }
                let Inner {
                    update_history_id,
                    host_id,
                    software_item_id,
                } = serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::UpdateProtectionStarted {
                    update_history_id,
                    host_id,
                    software_item_id,
                })
            }
            "update_started" => {
                #[derive(Deserialize)]
                struct Inner {
                    update_history_id: Uuid,
                    host_id: Uuid,
                    software_item_id: Uuid,
                    interactive: bool,
                }
                let Inner {
                    update_history_id,
                    host_id,
                    software_item_id,
                    interactive,
                } = serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::UpdateStarted {
                    update_history_id,
                    host_id,
                    software_item_id,
                    interactive,
                })
            }
            "update_completed" => {
                #[derive(Deserialize)]
                struct Inner {
                    update_history_id: Uuid,
                    host_id: Uuid,
                    software_item_id: Uuid,
                    status: String,
                }
                let Inner {
                    update_history_id,
                    host_id,
                    software_item_id,
                    status,
                } = serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::UpdateCompleted {
                    update_history_id,
                    host_id,
                    software_item_id,
                    status,
                })
            }
            "discovery_completed" => {
                #[derive(Deserialize)]
                struct Inner {
                    host_id: Uuid,
                }
                let Inner { host_id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::DiscoveryCompleted { host_id })
            }
            "system_service_status_changed" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                    status: String,
                }
                let Inner { id, status } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::SystemServiceStatusChanged { id, status })
            }
            "scheduler_task_completed" => {
                #[derive(Deserialize)]
                struct Inner {
                    task_id: Uuid,
                }
                let Inner { task_id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::SchedulerTaskCompleted { task_id })
            }
            "host_tag_created" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                }
                let Inner { id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::HostTagCreated { id })
            }
            "host_tag_updated" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                }
                let Inner { id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::HostTagUpdated { id })
            }
            "host_tag_deleted" => {
                #[derive(Deserialize)]
                struct Inner {
                    id: Uuid,
                }
                let Inner { id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::HostTagDeleted { id })
            }
            "host_tags_changed" => {
                #[derive(Deserialize)]
                struct Inner {
                    host_id: Uuid,
                }
                let Inner { host_id } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::HostTagsChanged { host_id })
            }
            // Note: serde's snake_case renaming converts "GitHub" → "git_hub",
            // so the wire key is "global_git_hub_provider_misconfigured".
            "global_git_hub_provider_misconfigured" => {
                #[derive(Deserialize)]
                struct Inner {
                    problem: String,
                }
                let Inner { problem } =
                    serde_json::from_value(inner).map_err(serde::de::Error::custom)?;
                Ok(Self::GlobalGitHubProviderMisconfigured { problem })
            }
            other => {
                tracing::debug!(variant = other, "received unknown AdminEvent variant");
                Ok(Self::Other(other.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All known (non-Other) variants for exhaustive testing.
    const KNOWN_VARIANTS: &[&str] = &[
        "host_updated",
        "host_created",
        "host_deleted",
        "service_status_changed",
        "software_item_updated",
        "software_item_created",
        "version_check_completed",
        "update_triggered",
        "update_protection_started",
        "update_started",
        "update_completed",
        "discovery_completed",
        "system_service_status_changed",
        "scheduler_task_completed",
        "host_tag_created",
        "host_tag_updated",
        "host_tag_deleted",
        "host_tags_changed",
        "global_git_hub_provider_misconfigured",
        "data_reset",
        "surfaces_changed",
    ];

    /// All known variants as `AdminEvent` instances for exhaustive testing.
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
                status: "pending".to_string(),
            },
            AdminEvent::UpdateProtectionStarted {
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
            AdminEvent::SurfacesChanged,
        ]
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
                status: "pending".to_string(),
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
        // Variant guard: if a new variant is added without updating KNOWN_VARIANTS
        // and all_variants(), this test will fail.
        assert_eq!(all_variants().len(), KNOWN_VARIANTS.len());
    }

    #[test]
    fn update_protection_started_event_name() {
        let id = Uuid::nil();
        let event = AdminEvent::UpdateProtectionStarted {
            update_history_id: id,
            host_id: id,
            software_item_id: id,
        };
        assert_eq!(event.event_name(), "update_protection_started");
    }

    /// Verify OUR custom `Deserialize` impl: unknown variants deserialize to
    /// `Other(String)` rather than returning an error, enabling rolling upgrades.
    #[test]
    fn unknown_variant_deserializes_to_other() {
        // Struct-style unknown variant (object form)
        let json = r#"{"future_variant":{"host_id":"00000000-0000-0000-0000-000000000000"}}"#;
        let event: AdminEvent = serde_json::from_str(json).expect("should accept unknown variant");
        assert!(
            matches!(event, AdminEvent::Other(ref v) if v == "future_variant"),
            "expected Other(\"future_variant\"), got: {event:?}"
        );
    }

    /// Verify unit-style unknown variants also deserialize to `Other(String)`.
    #[test]
    fn unknown_unit_variant_deserializes_to_other() {
        let json = r#""brand_new_unit_event""#;
        let event: AdminEvent = serde_json::from_str(json).expect("should accept unknown variant");
        assert!(
            matches!(event, AdminEvent::Other(ref v) if v == "brand_new_unit_event"),
            "expected Other(\"brand_new_unit_event\"), got: {event:?}"
        );
    }

    /// Verify that all known variants round-trip through serialize → deserialize
    /// and that event_name() is preserved. This tests OUR custom Deserialize impl,
    /// not serde's generic derive behavior.
    #[test]
    fn known_variants_round_trip_through_custom_deserialize() {
        for event in all_variants() {
            let json = serde_json::to_string(&event).expect("serialization should succeed");
            let deserialized: AdminEvent =
                serde_json::from_str(&json).expect("deserialization should succeed");
            assert_eq!(
                event.event_name(),
                deserialized.event_name(),
                "event_name mismatch after round-trip for: {json}"
            );
            // Deserialized known variants must NOT produce Other(_).
            assert!(
                !matches!(deserialized, AdminEvent::Other(_)),
                "known variant round-tripped to Other: {json}"
            );
        }
    }

    #[test]
    fn update_triggered_missing_status_defaults_to_pending() {
        let json = r#"{"update_triggered":{"update_history_id":"00000000-0000-0000-0000-000000000000","host_id":"00000000-0000-0000-0000-000000000000","software_item_id":"00000000-0000-0000-0000-000000000000"}}"#;
        let event: AdminEvent =
            serde_json::from_str(json).expect("backward-compat deserialization");
        assert!(
            matches!(event, AdminEvent::UpdateTriggered { status: ref s, .. } if s == "pending"),
            "expected UpdateTriggered with pending status, got: {event:?}"
        );
    }

    /// Verify that `Other(String)` event_name() returns the raw variant string.
    #[test]
    fn other_event_name_returns_raw_string() {
        let event = AdminEvent::Other("some_future_event".to_string());
        assert_eq!(event.event_name(), "some_future_event");
    }
}
