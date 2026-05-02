//! Typed SSE streaming method for admin events.
//!
//! Provides [`UptrakitClient::stream_events`] which connects to the
//! `GET /api/v1/events/stream` endpoint and returns a typed stream of
//! admin events for the authenticated user's tenant.

use crate::sse::{self, RawSseEvent, SseError};
use crate::{Result, UptrakitClient};
use rootcause::prelude::*;
use uuid::Uuid;

/// A typed SSE event from the admin events stream.
///
/// Mirrors [`AdminEvent`](uptrakit_web_api_types::events::AdminEvent) variants
/// with an additional [`Unknown`](Self::Unknown) catch-all for forward
/// compatibility when the server emits new event types.
#[derive(Debug, Clone)]
pub enum AdminSseEvent {
    /// A host's metadata was updated.
    HostUpdated { id: Uuid },
    /// A new host was created.
    HostCreated { id: Uuid },
    /// A host was deactivated / deleted.
    HostDeleted { id: Uuid },
    /// A service's status changed.
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
        interactive: bool,
    },
    /// A software update completed.
    UpdateCompleted {
        update_history_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        status: String,
    },
    /// Autodiscovery completed for a host.
    DiscoveryCompleted { host_id: Uuid },
    /// Host packages changed.
    HostPackagesChanged { host_id: Uuid },
    /// A batch host package update completed.
    BatchHostPackageUpdateCompleted { host_id: Uuid },
    /// A system service's status changed.
    SystemServiceStatusChanged { id: Uuid, status: String },
    /// A scheduled task completed execution.
    SchedulerTaskCompleted { task_id: Uuid },
    /// All tenant data was reset.
    DataReset,
    /// An unrecognised event type from a newer server version.
    Unknown { event_type: String, data: String },
}

/// Errors specific to admin event streaming.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("SSE transport error: {0}")]
    Sse(#[from] SseError),

    #[error("failed to parse SSE event data: {0}")]
    Parse(#[from] serde_json::Error),
}

impl UptrakitClient {
    /// Connect to the admin events SSE stream and return a stream of typed events.
    ///
    /// The returned stream yields [`AdminSseEvent`] values for the authenticated
    /// user's tenant. The stream stays open indefinitely (server pushes events as
    /// state changes occur) and should be cancelled by the caller when no longer
    /// needed.
    ///
    /// Uses an 86400s (24h) timeout, matching other long-lived SSE connections.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn stream_events(
        &self,
    ) -> Result<impl futures_util::Stream<Item = std::result::Result<AdminSseEvent, StreamError>>>
    {
        let url = format!("{}{}", self.base_url, crate::paths::events::STREAM);

        let req = self
            .http
            .get(&url)
            .bearer_auth(self.token_or_err()?)
            .header("Accept", "text/event-stream")
            .timeout(std::time::Duration::from_secs(86400));

        let resp = req.send().await.context_to()?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!(crate::ClientError::NotAuthenticated);
        }
        if status.is_client_error() || status.is_server_error() {
            let text = resp.text().await.context_to()?;
            let message = crate::extract_error_message(&text);
            bail!(crate::ClientError::Api { status, message });
        }

        let raw_stream = sse::parse_sse_stream(resp);

        let typed_stream = futures_util::StreamExt::filter_map(raw_stream, |result| async move {
            match result {
                Ok(event) => Some(parse_typed_event(event)),
                Err(e) => Some(Err(StreamError::Sse(e))),
            }
        });

        Ok(typed_stream)
    }
}

/// Helper for parsing a JSON `data` field with a single `id` key.
fn parse_id(data: &str) -> std::result::Result<Uuid, serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct Id {
        id: Uuid,
    }
    serde_json::from_str::<Id>(data).map(|v| v.id)
}

/// Helper for parsing a JSON `data` field with `id` and `status` keys.
fn parse_id_status(data: &str) -> std::result::Result<(Uuid, String), serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct IdStatus {
        id: Uuid,
        status: String,
    }
    serde_json::from_str::<IdStatus>(data).map(|v| (v.id, v.status))
}

/// Helper for parsing a JSON `data` field with a single `host_id` key.
fn parse_host_id(data: &str) -> std::result::Result<Uuid, serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct HostId {
        host_id: Uuid,
    }
    serde_json::from_str::<HostId>(data).map(|v| v.host_id)
}

/// Parse a raw SSE event into a typed [`AdminSseEvent`].
///
/// Unknown event types are returned as [`AdminSseEvent::Unknown`] for forward
/// compatibility — the client never drops events from a newer server.
fn parse_typed_event(event: RawSseEvent) -> std::result::Result<AdminSseEvent, StreamError> {
    match event.event_type.as_str() {
        "host_updated" => Ok(AdminSseEvent::HostUpdated {
            id: parse_id(&event.data)?,
        }),
        "host_created" => Ok(AdminSseEvent::HostCreated {
            id: parse_id(&event.data)?,
        }),
        "host_deleted" => Ok(AdminSseEvent::HostDeleted {
            id: parse_id(&event.data)?,
        }),
        "service_status_changed" => {
            let (id, status) = parse_id_status(&event.data)?;
            Ok(AdminSseEvent::ServiceStatusChanged { id, status })
        }
        "software_item_updated" => Ok(AdminSseEvent::SoftwareItemUpdated {
            id: parse_id(&event.data)?,
        }),
        "software_item_created" => Ok(AdminSseEvent::SoftwareItemCreated {
            id: parse_id(&event.data)?,
        }),
        "version_check_completed" => {
            #[derive(serde::Deserialize)]
            struct Payload {
                host_id: Uuid,
                software_item_id: Uuid,
            }
            let p: Payload = serde_json::from_str(&event.data)?;
            Ok(AdminSseEvent::VersionCheckCompleted {
                host_id: p.host_id,
                software_item_id: p.software_item_id,
            })
        }
        "update_triggered" => {
            #[derive(serde::Deserialize)]
            struct Payload {
                update_history_id: Uuid,
                host_id: Uuid,
                software_item_id: Uuid,
            }
            let p: Payload = serde_json::from_str(&event.data)?;
            Ok(AdminSseEvent::UpdateTriggered {
                update_history_id: p.update_history_id,
                host_id: p.host_id,
                software_item_id: p.software_item_id,
            })
        }
        "update_started" => {
            #[derive(serde::Deserialize)]
            struct Payload {
                update_history_id: Uuid,
                host_id: Uuid,
                software_item_id: Uuid,
                #[serde(default)]
                interactive: bool,
            }
            let p: Payload = serde_json::from_str(&event.data)?;
            Ok(AdminSseEvent::UpdateStarted {
                update_history_id: p.update_history_id,
                host_id: p.host_id,
                software_item_id: p.software_item_id,
                interactive: p.interactive,
            })
        }
        "update_completed" => {
            #[derive(serde::Deserialize)]
            struct Payload {
                update_history_id: Uuid,
                host_id: Uuid,
                software_item_id: Uuid,
                status: String,
            }
            let p: Payload = serde_json::from_str(&event.data)?;
            Ok(AdminSseEvent::UpdateCompleted {
                update_history_id: p.update_history_id,
                host_id: p.host_id,
                software_item_id: p.software_item_id,
                status: p.status,
            })
        }
        "discovery_completed" => Ok(AdminSseEvent::DiscoveryCompleted {
            host_id: parse_host_id(&event.data)?,
        }),
        "host_packages_changed" => Ok(AdminSseEvent::HostPackagesChanged {
            host_id: parse_host_id(&event.data)?,
        }),
        "batch_host_package_update_completed" => {
            Ok(AdminSseEvent::BatchHostPackageUpdateCompleted {
                host_id: parse_host_id(&event.data)?,
            })
        }
        "system_service_status_changed" => {
            let (id, status) = parse_id_status(&event.data)?;
            Ok(AdminSseEvent::SystemServiceStatusChanged { id, status })
        }
        "scheduler_task_completed" => {
            #[derive(serde::Deserialize)]
            struct Payload {
                task_id: Uuid,
            }
            let p: Payload = serde_json::from_str(&event.data)?;
            Ok(AdminSseEvent::SchedulerTaskCompleted { task_id: p.task_id })
        }
        "data_reset" => Ok(AdminSseEvent::DataReset),
        _ => Ok(AdminSseEvent::Unknown {
            event_type: event.event_type,
            data: event.data,
        }),
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_err()) is idiomatic in tests"
    )]

    use super::*;
    use crate::sse::RawSseEvent;

    fn make_event(event_type: &str, data: &str) -> RawSseEvent {
        RawSseEvent {
            event_type: event_type.to_string(),
            data: data.to_string(),
            id: None,
        }
    }

    #[test]
    fn parse_host_updated() {
        let event = make_event(
            "host_updated",
            r#"{"id":"550e8400-e29b-41d4-a716-446655440000"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(result, AdminSseEvent::HostUpdated { id } if !id.is_nil()));
    }

    #[test]
    fn parse_host_created() {
        let event = make_event(
            "host_created",
            r#"{"id":"550e8400-e29b-41d4-a716-446655440000"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(result, AdminSseEvent::HostCreated { .. }));
    }

    #[test]
    fn parse_host_deleted() {
        let event = make_event(
            "host_deleted",
            r#"{"id":"550e8400-e29b-41d4-a716-446655440000"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(result, AdminSseEvent::HostDeleted { .. }));
    }

    #[test]
    fn parse_service_status_changed() {
        let event = make_event(
            "service_status_changed",
            r#"{"id":"550e8400-e29b-41d4-a716-446655440000","status":"approved"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(
            matches!(result, AdminSseEvent::ServiceStatusChanged { status, .. } if status == "approved")
        );
    }

    #[test]
    fn parse_software_item_updated() {
        let event = make_event(
            "software_item_updated",
            r#"{"id":"550e8400-e29b-41d4-a716-446655440000"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(result, AdminSseEvent::SoftwareItemUpdated { .. }));
    }

    #[test]
    fn parse_software_item_created() {
        let event = make_event(
            "software_item_created",
            r#"{"id":"550e8400-e29b-41d4-a716-446655440000"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(result, AdminSseEvent::SoftwareItemCreated { .. }));
    }

    #[test]
    fn parse_version_check_completed() {
        let event = make_event(
            "version_check_completed",
            r#"{"host_id":"550e8400-e29b-41d4-a716-446655440001","software_item_id":"550e8400-e29b-41d4-a716-446655440002"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(
            result,
            AdminSseEvent::VersionCheckCompleted { .. }
        ));
    }

    #[test]
    fn parse_update_triggered() {
        let event = make_event(
            "update_triggered",
            r#"{"update_history_id":"550e8400-e29b-41d4-a716-446655440001","host_id":"550e8400-e29b-41d4-a716-446655440002","software_item_id":"550e8400-e29b-41d4-a716-446655440003"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(result, AdminSseEvent::UpdateTriggered { .. }));
    }

    #[test]
    fn parse_update_started() {
        let event = make_event(
            "update_started",
            r#"{"update_history_id":"550e8400-e29b-41d4-a716-446655440001","host_id":"550e8400-e29b-41d4-a716-446655440002","software_item_id":"550e8400-e29b-41d4-a716-446655440003","interactive":true}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(
            result,
            AdminSseEvent::UpdateStarted {
                interactive: true,
                ..
            }
        ));
    }

    #[test]
    fn parse_update_started_without_interactive_defaults_false() {
        // Older server versions may not send the `interactive` field.
        let event = make_event(
            "update_started",
            r#"{"update_history_id":"550e8400-e29b-41d4-a716-446655440001","host_id":"550e8400-e29b-41d4-a716-446655440002","software_item_id":"550e8400-e29b-41d4-a716-446655440003"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(
            result,
            AdminSseEvent::UpdateStarted {
                interactive: false,
                ..
            }
        ));
    }

    #[test]
    fn parse_update_completed() {
        let event = make_event(
            "update_completed",
            r#"{"update_history_id":"550e8400-e29b-41d4-a716-446655440001","host_id":"550e8400-e29b-41d4-a716-446655440002","software_item_id":"550e8400-e29b-41d4-a716-446655440003","status":"completed"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(
            matches!(result, AdminSseEvent::UpdateCompleted { status, .. } if status == "completed")
        );
    }

    #[test]
    fn parse_discovery_completed() {
        let event = make_event(
            "discovery_completed",
            r#"{"host_id":"550e8400-e29b-41d4-a716-446655440000"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(result, AdminSseEvent::DiscoveryCompleted { .. }));
    }

    #[test]
    fn parse_host_packages_changed() {
        let event = make_event(
            "host_packages_changed",
            r#"{"host_id":"550e8400-e29b-41d4-a716-446655440000"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(result, AdminSseEvent::HostPackagesChanged { .. }));
    }

    #[test]
    fn parse_batch_host_package_update_completed() {
        let event = make_event(
            "batch_host_package_update_completed",
            r#"{"host_id":"550e8400-e29b-41d4-a716-446655440000"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(
            result,
            AdminSseEvent::BatchHostPackageUpdateCompleted { .. }
        ));
    }

    #[test]
    fn parse_system_service_status_changed() {
        let event = make_event(
            "system_service_status_changed",
            r#"{"id":"550e8400-e29b-41d4-a716-446655440000","status":"rejected"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(
            matches!(result, AdminSseEvent::SystemServiceStatusChanged { status, .. } if status == "rejected")
        );
    }

    #[test]
    fn parse_scheduler_task_completed() {
        let event = make_event(
            "scheduler_task_completed",
            r#"{"task_id":"550e8400-e29b-41d4-a716-446655440000"}"#,
        );
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(
            result,
            AdminSseEvent::SchedulerTaskCompleted { .. }
        ));
    }

    #[test]
    fn parse_data_reset() {
        let event = make_event("data_reset", "{}");
        let result = parse_typed_event(event).unwrap();
        assert!(matches!(result, AdminSseEvent::DataReset));
    }

    #[test]
    fn parse_unknown_event_returns_unknown() {
        let event = make_event("future_event", r#"{"foo":"bar"}"#);
        let result = parse_typed_event(event).unwrap();
        assert!(
            matches!(result, AdminSseEvent::Unknown { event_type, .. } if event_type == "future_event")
        );
    }

    #[test]
    fn parse_malformed_data_returns_error() {
        let event = make_event("host_updated", "not json");
        assert!(parse_typed_event(event).is_err());
    }
}
