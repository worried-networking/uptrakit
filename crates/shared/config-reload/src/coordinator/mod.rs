//! Coordinator types: state, phases, sources, and the coordinator handle.

mod state_machine;

pub use state_machine::ReloadCoordinator;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::config::Scope;

// ── ReloadSource ─────────────────────────────────────────────────────────────

/// Trigger source for a reload request.
///
/// Wire-exposed via audit-log JSON. Unknown sources received from newer
/// coordinator versions become [`ReloadSource::Other`] for forward
/// compatibility.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ReloadSource {
    /// Process received `SIGHUP`.
    Sighup,
    /// Filesystem watcher detected a change on the given path.
    FileWatch {
        /// The path whose modification triggered the reload.
        path: PathBuf,
    },
    /// A DB settings-version bump was detected for the given scope/sections.
    DbBump {
        /// The scope (global or a specific tenant) that changed.
        scope: Scope,
        /// Which config sections changed.
        sections: Vec<String>,
    },
    /// Initial load at process boot.
    Boot,
    /// An unknown source received from a newer coordinator version.
    Other(String),
}

// Custom serde for ReloadSource: serialize to a JSON object with a `kind` field;
// unknown `kind` values deserialize to `Other(String)`.
impl Serialize for ReloadSource {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Self::Sighup => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("kind", "sighup")?;
                map.end()
            }
            Self::FileWatch { path } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "file_watch")?;
                map.serialize_entry("path", path)?;
                map.end()
            }
            Self::DbBump { scope, sections } => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("kind", "db_bump")?;
                map.serialize_entry("scope", scope)?;
                map.serialize_entry("sections", sections)?;
                map.end()
            }
            Self::Boot => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("kind", "boot")?;
                map.end()
            }
            Self::Other(s) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("kind", s.as_str())?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ReloadSource {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct ReloadSourceVisitor;

        impl<'de> Visitor<'de> for ReloadSourceVisitor {
            type Value = ReloadSource;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a reload source object with a `kind` field")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                // Collect all key-value pairs first.
                let mut kind: Option<String> = None;
                let mut path: Option<PathBuf> = None;
                let mut scope: Option<Scope> = None;
                let mut sections: Option<Vec<String>> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "kind" => kind = Some(map.next_value()?),
                        "path" => path = Some(map.next_value()?),
                        "scope" => scope = Some(map.next_value()?),
                        "sections" => sections = Some(map.next_value()?),
                        _ => {
                            // Consume unknown fields.
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                let kind = kind.ok_or_else(|| de::Error::missing_field("kind"))?;
                match kind.as_str() {
                    "sighup" => Ok(ReloadSource::Sighup),
                    "file_watch" => {
                        let path = path.ok_or_else(|| de::Error::missing_field("path"))?;
                        Ok(ReloadSource::FileWatch { path })
                    }
                    "db_bump" => {
                        let scope = scope.ok_or_else(|| de::Error::missing_field("scope"))?;
                        let sections =
                            sections.ok_or_else(|| de::Error::missing_field("sections"))?;
                        Ok(ReloadSource::DbBump { scope, sections })
                    }
                    "boot" => Ok(ReloadSource::Boot),
                    other => Ok(ReloadSource::Other(other.to_string())),
                }
            }
        }

        deserializer.deserialize_map(ReloadSourceVisitor)
    }
}

// ── ReloadPhase ──────────────────────────────────────────────────────────────

/// Coordinator phase recorded in `ConfigReloadFailed` audit events.
///
/// Wire-exposed. Unknown phase strings from newer coordinator versions
/// become [`ReloadPhase::Other`] for forward compatibility.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReloadPhase {
    /// Config validation before any state mutation.
    Validate,
    /// Applying the new config to live subsystems.
    Apply,
    /// Post-apply health-check window.
    Watchdog,
    /// Process re-exec / binary replacement.
    Reexec,
    /// An unknown phase received from a newer coordinator version.
    Other(String),
}

impl ReloadPhase {
    /// Returns the string representation.
    ///
    /// For [`ReloadPhase::Other`], returns the inner string as-is.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Validate => "validate",
            Self::Apply => "apply",
            Self::Watchdog => "watchdog",
            Self::Reexec => "reexec",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for ReloadPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for ReloadPhase {
    fn from(s: String) -> Self {
        match s.as_str() {
            "validate" => Self::Validate,
            "apply" => Self::Apply,
            "watchdog" => Self::Watchdog,
            "reexec" => Self::Reexec,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for ReloadPhase {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ReloadPhase {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(ReloadPhase::from)
    }
}

// ── ReloadRequest ─────────────────────────────────────────────────────────────

/// A request to reload configuration.
#[derive(Clone, Debug)]
pub struct ReloadRequest {
    /// What triggered this reload.
    pub source: ReloadSource,
    /// When the request was created.
    pub timestamp: OffsetDateTime,
}

// ── CoordinatorState ──────────────────────────────────────────────────────────

/// Observable state of the [`ReloadCoordinator`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoordinatorState {
    /// No reload in progress; all subsystems healthy.
    Idle,
    /// A reload cycle is currently running.
    Reloading,
    /// One or more subsystems failed to revert after a failed apply.
    /// Manual intervention or a process restart is required.
    Degraded(DegradedInfo),
}

/// Details attached to a [`CoordinatorState::Degraded`] state.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct DegradedInfo {
    /// When the coordinator entered the Degraded state.
    pub since: OffsetDateTime,
    /// Names of the subsystems that failed to revert.
    pub failed_subsystems: Vec<String>,
    /// Human-readable description of what went wrong.
    pub reason: String,
}

impl DegradedInfo {
    /// Create a new [`DegradedInfo`].
    #[must_use]
    pub fn new(
        since: OffsetDateTime,
        failed_subsystems: Vec<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            since,
            failed_subsystems,
            reason: reason.into(),
        }
    }
}

// ── ReloadCoordinatorHandle ───────────────────────────────────────────────────

/// Handle for external introspection and request submission.
///
/// Cheaply cloneable. Backed by an [`arc_swap::ArcSwap`] for lock-free state
/// reads and a `tokio::sync::mpsc` channel for reload requests.
#[derive(Clone)]
pub struct ReloadCoordinatorHandle {
    pub(crate) state: std::sync::Arc<arc_swap::ArcSwap<CoordinatorState>>,
    pub(crate) tx: tokio::sync::mpsc::Sender<ReloadRequest>,
}

impl ReloadCoordinatorHandle {
    /// Returns a snapshot of the current coordinator state.
    #[must_use]
    pub fn state(&self) -> CoordinatorState {
        (**self.state.load()).clone()
    }

    /// Returns a clone of the underlying request sender.
    #[must_use]
    pub fn sender(&self) -> tokio::sync::mpsc::Sender<ReloadRequest> {
        self.tx.clone()
    }

    /// Enqueue a reload request.
    ///
    /// # Errors
    ///
    /// Returns an error if the coordinator's receiver has been dropped (i.e.
    /// the coordinator task has exited).
    pub async fn enqueue(
        &self,
        request: ReloadRequest,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<ReloadRequest>> {
        self.tx.send(request).await
    }

    /// Clear the Degraded state if the coordinator is currently degraded.
    ///
    /// This is a direct state transition — the operator asserts that the
    /// underlying issue has been resolved. No health re-check is performed here;
    /// that requires the full list of reloadables held by the state machine.
    /// If the coordinator is [`CoordinatorState::Idle`] or
    /// [`CoordinatorState::Reloading`], this is a no-op.
    ///
    /// # Errors
    ///
    /// Currently infallible; returns `Ok(())` always. The signature is
    /// `Result` so that future versions can propagate channel send errors
    /// without a breaking change.
    pub async fn clear_degraded(&self) -> Result<(), rootcause::Report> {
        let current = (**self.state.load()).clone();
        if matches!(current, CoordinatorState::Degraded(_)) {
            self.state
                .store(std::sync::Arc::new(CoordinatorState::Idle));
        }
        Ok(())
    }
}
