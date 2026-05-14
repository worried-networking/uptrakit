//! Response types for the `GET /api/v1/instance/config-state` and
//! `POST /api/v1/instance/config-reload/clear-degraded` endpoints.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Full response body for `GET /api/v1/instance/config-state`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub struct ConfigStateResponse {
    /// Current coordinator state: `"idle"`, `"reloading"`, or `"degraded"`.
    pub coordinator_state: String,
    /// Details when the coordinator is in a degraded state.
    pub degraded: Option<DegradedInfoView>,
    /// Current config file on disk.
    pub file: FileStateView,
    /// Summary of the last successfully applied reload cycle.
    pub last_reload: Option<LastReloadView>,
    /// Redacted snapshot of the active config sections (secrets shown as `"<redacted>"`).
    pub sections: serde_json::Value,
    /// Recent reload lifecycle events (up to 20, newest last).
    pub recent_events: Vec<serde_json::Value>,
}

impl ConfigStateResponse {
    #[must_use]
    pub fn new(
        coordinator_state: String,
        degraded: Option<DegradedInfoView>,
        file: FileStateView,
        last_reload: Option<LastReloadView>,
        sections: serde_json::Value,
        recent_events: Vec<serde_json::Value>,
    ) -> Self {
        Self {
            coordinator_state,
            degraded,
            file,
            last_reload,
            sections,
            recent_events,
        }
    }
}

/// Config file state view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub struct FileStateView {
    /// Absolute path to the TOML config file.
    pub path: String,
    /// Digest of the file as last loaded (hex SHA-256 or size stub).
    pub digest: String,
    /// When the file was last successfully loaded.
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub loaded_at: OffsetDateTime,
    /// Digest of a pending (not yet reloaded) change detected on disk.
    pub pending_digest: Option<String>,
    /// When the pending change was first detected.
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub pending_detected_at: Option<OffsetDateTime>,
}

impl FileStateView {
    #[must_use]
    pub fn new(
        path: String,
        digest: String,
        loaded_at: OffsetDateTime,
        pending_digest: Option<String>,
        pending_detected_at: Option<OffsetDateTime>,
    ) -> Self {
        Self {
            path,
            digest,
            loaded_at,
            pending_digest,
            pending_detected_at,
        }
    }
}

/// Summary of the last successfully applied reload cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub struct LastReloadView {
    /// When the reload completed.
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub completed_at: OffsetDateTime,
    /// Config sections that changed during this reload.
    pub sections: Vec<String>,
    /// Wall-clock milliseconds spent per subsystem.
    pub per_subsystem_ms: BTreeMap<String, u64>,
}

impl LastReloadView {
    #[must_use]
    pub fn new(
        completed_at: OffsetDateTime,
        sections: Vec<String>,
        per_subsystem_ms: BTreeMap<String, u64>,
    ) -> Self {
        Self {
            completed_at,
            sections,
            per_subsystem_ms,
        }
    }
}

/// View of the coordinator's degraded state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub struct DegradedInfoView {
    /// When the coordinator entered the degraded state.
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub since: OffsetDateTime,
    /// Names of the subsystems that failed to revert.
    pub failed_subsystems: Vec<String>,
    /// Human-readable description of what went wrong.
    pub reason: String,
}

impl DegradedInfoView {
    #[must_use]
    pub fn new(since: OffsetDateTime, failed_subsystems: Vec<String>, reason: String) -> Self {
        Self {
            since,
            failed_subsystems,
            reason,
        }
    }
}
