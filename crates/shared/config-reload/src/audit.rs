//! Audit events emitted by the reload coordinator.

use std::collections::BTreeMap;

use crate::coordinator::{ReloadPhase, ReloadSource};

/// An event emitted by the [`crate::coordinator::ReloadCoordinator`] during a
/// reload lifecycle.
///
/// Consumers (e.g. the audit-log subsystem) receive these events on an
/// unbounded channel and persist or forward them as needed.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ReloadAuditEvent {
    /// A reload request was received but rejected (e.g. coordinator Degraded).
    Refused {
        /// The source that submitted the refused request.
        source: ReloadSource,
        /// Human-readable reason for rejection.
        reason: String,
    },
    /// A reload request was accepted and is being processed.
    Requested {
        /// The source that submitted the request.
        source: ReloadSource,
    },
    /// The reload cycle completed successfully.
    Applied {
        /// Which config sections were changed.
        sections: Vec<String>,
        /// Wall-clock milliseconds spent per subsystem (apply + health-check).
        per_subsystem_ms: BTreeMap<String, u64>,
    },
    /// The reload cycle failed at the given phase.
    Failed {
        /// The coordinator phase in which the failure occurred.
        phase: ReloadPhase,
        /// The subsystem that failed, if failure was subsystem-specific.
        subsystem: Option<String>,
        /// Human-readable description of the error.
        error: String,
    },
    /// A subsystem was reverted after a failed apply.
    Reverted {
        /// The subsystem that was reverted.
        subsystem: String,
        /// Human-readable reason the revert was triggered.
        reason: String,
    },
}
