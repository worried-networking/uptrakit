// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! Behavioral profiles derived from a service's persisted capability set.
//!
//! [`ServiceProfile`] is a derived enum — never stored in the database. It is
//! computed from `BTreeSet<Capability>` via [`ServiceProfile::from_capabilities`]
//! and drives controller-side behavioral defaults (ping interval, shutdown
//! timeout, human-readable label).
use crate::generated::wire::Capability;
use std::collections::BTreeSet;
/// Behavioral profile derived from a service's capability set.
///
/// | Profile | Key capability | Services |
/// | --- | --- | --- |
/// | `UpdateTracker` | `Capability::UpdateTracking` | MQTT service |
/// | `Agent` | `Capability::SoftwareDiscovery` | Local agent, SSH agent |
/// | `Scheduler` | `Capability::Scheduler` | External task scheduler |
///
/// `Unknown` is the fallback for unrecognized capability combinations.
/// `ServiceProfile` is never persisted — it is always derived from capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServiceProfile {
    /// Update-tracking service (has `UpdateTracking` capability).
    UpdateTracker,
    /// Agent service — local or SSH-backed (has `SoftwareDiscovery` capability).
    Agent,
    /// External task scheduler service (has `Scheduler` capability).
    Scheduler,
    /// Unrecognized capability combination.
    Unknown,
}
impl ServiceProfile {
    /// Derive the behavioral profile from a capability set.
    ///
    /// Precedence: `UpdateTracker` > `Scheduler` > `Agent` > `Unknown`.
    pub fn from_capabilities(caps: &BTreeSet<Capability>) -> Self {
        if caps.contains(&Capability::UpdateTracking) {
            Self::UpdateTracker
        } else if caps.contains(&Capability::Scheduler) {
            Self::Scheduler
        } else if caps.contains(&Capability::SoftwareDiscovery) {
            Self::Agent
        } else {
            Self::Unknown
        }
    }
    /// Default ping interval in seconds for this profile.
    ///
    /// - `UpdateTracker`: 15 seconds (MQTT lease heartbeat).
    /// - `Scheduler`: 60 seconds (less latency-sensitive).
    /// - `Agent` / `Unknown`: 300 seconds (5 minutes).
    pub const fn default_ping_interval_secs(&self) -> u32 {
        match self {
            Self::UpdateTracker => 15,
            Self::Scheduler => 60,
            Self::Agent | Self::Unknown => 300,
        }
    }
    /// Shutdown timeout in seconds, if applicable.
    ///
    /// - `UpdateTracker`: `None` (no graceful shutdown timeout).
    /// - `Scheduler`: `Some(30)` (allow claim release).
    /// - `Agent` / `Unknown`: `Some(120)` (2 minutes).
    pub const fn shutdown_timeout_secs(&self) -> Option<u32> {
        match self {
            Self::UpdateTracker => None,
            Self::Scheduler => Some(30),
            Self::Agent | Self::Unknown => Some(120),
        }
    }
    /// Human-readable label for display in the API and UI.
    ///
    /// For `Agent` profiles, pass `has_ssh_remote = true` to distinguish
    /// SSH-backed agents from local agents.
    pub const fn service_label(&self, has_ssh_remote: bool) -> &'static str {
        match self {
            Self::UpdateTracker => "Update Tracker",
            Self::Scheduler => "Scheduler",
            Self::Agent if has_ssh_remote => "SSH Agent",
            Self::Agent => "Agent",
            Self::Unknown => "Unknown",
        }
    }
}
/// Parse a JSON array string into a capability set.
///
/// The JSON is expected to be an array of snake_case strings
/// (e.g. `["software_discovery","update_hooks","graceful_shutdown"]`).
/// Returns an empty set on parse failure.
pub fn parse_capabilities(json: &str) -> BTreeSet<Capability> {
    serde_json::from_str::<Vec<Capability>>(json)
        .unwrap_or_default()
        .into_iter()
        .collect()
}
/// Serialize a capability set into a JSON array string.
///
/// Produces a sorted JSON array of snake_case strings
/// (e.g. `["graceful_shutdown","software_discovery","update_hooks"]`).
/// `BTreeSet` iteration order guarantees deterministic output.
pub fn serialize_capabilities(caps: &BTreeSet<Capability>) -> String {
    let vec: Vec<&Capability> = caps.iter().collect();
    serde_json::to_string(&vec).unwrap_or_else(|_| "[]".to_string())
}
