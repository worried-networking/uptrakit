//! Status snapshot types for the `GET /api/v1/instance/config-state` endpoint.

use std::collections::BTreeMap;

/// Snapshot of the config file on disk.
///
/// Populated at boot from the TOML file path passed to the controller, and
/// updated by the `reload_audit_bridge` task whenever a reload cycle applies
/// a new file.
#[derive(Clone, Debug)]
pub struct ConfigFileState {
    /// Absolute path to the active TOML config file.
    pub path: String,
    /// Hex-encoded SHA-256 digest (or a size-based stub if `sha2` is unavailable).
    pub digest: String,
    /// When this file was last successfully loaded.
    pub loaded_at: time::OffsetDateTime,
    /// Digest of a file change detected but not yet reloaded.
    pub pending_digest: Option<String>,
    /// When the pending change was first detected.
    pub pending_detected_at: Option<time::OffsetDateTime>,
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
pub struct LastReloadInfo {
    /// When the reload cycle completed (all subsystems applied + health-checked).
    pub completed_at: time::OffsetDateTime,
    /// Config sections that changed during this reload.
    pub sections: Vec<String>,
    /// Wall-clock milliseconds spent per subsystem name.
    pub per_subsystem_ms: BTreeMap<String, u64>,
}
