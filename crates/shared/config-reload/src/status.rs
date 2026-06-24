//! Status snapshot types for the `GET /api/v1/instance/config-state` endpoint.

use std::collections::BTreeMap;

/// Snapshot of the config file on disk.
///
/// Populated at boot from the TOML file path passed to the controller, and
/// updated by the `reload_audit_bridge` task whenever a reload cycle applies
/// a new file.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ConfigFileState {
    /// Absolute path to the active TOML config file.
    pub path: String,
    /// Canonical `sha256:<hex>` digest of the active config file.
    ///
    /// This value is set at boot and updated after each successful reload.
    /// On a transient applied-path re-read error the field retains its previous
    /// value (never blanked). A *persistent* re-read failure — e.g. the config
    /// file is moved or removed after the controller started — therefore leaves a
    /// stale digest displayed here. The only signal is repeated
    /// `applied digest re-read failed` warn log lines; operators should treat
    /// sustained warns as "displayed digest may be stale."
    pub digest: String,
    /// When this file was last successfully loaded.
    pub loaded_at: time::OffsetDateTime,
    /// `sha256:<hex>` of a detected-but-unapplied change; `None` when no change
    /// is pending or the changed file could not be read.
    pub pending_digest: Option<String>,
    /// When the pending change was first detected.
    pub pending_detected_at: Option<time::OffsetDateTime>,
}

impl ConfigFileState {
    #[must_use]
    pub fn new(
        path: String,
        digest: String,
        loaded_at: time::OffsetDateTime,
        pending_digest: Option<String>,
        pending_detected_at: Option<time::OffsetDateTime>,
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

impl Default for ConfigFileState {
    fn default() -> Self {
        Self {
            path: String::new(),
            digest: String::new(),
            loaded_at: time::OffsetDateTime::UNIX_EPOCH,
            pending_digest: None,
            pending_detected_at: None,
        }
    }
}

/// Summary of the last completed reload cycle.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LastReloadInfo {
    /// When the reload cycle completed (all subsystems applied + health-checked).
    pub completed_at: time::OffsetDateTime,
    /// Config sections that changed during this reload.
    pub sections: Vec<String>,
    /// Wall-clock milliseconds spent per subsystem name.
    pub per_subsystem_ms: BTreeMap<String, u64>,
}

impl LastReloadInfo {
    #[must_use]
    pub fn new(
        completed_at: time::OffsetDateTime,
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
