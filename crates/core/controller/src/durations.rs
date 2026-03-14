//! Named constants for recurring durations used across the controller.
//!
//! Replaces scattered hardcoded values in `main.rs`, `pki.rs`, and `crl_manager.rs`.

use std::time::Duration;

/// CA rotation window: rotate when the CA certificate expires within this many days.
pub(crate) const CA_ROTATION_WINDOW_DAYS: i64 = 183;

/// Server certificate renewal window: renew when the cert expires within this many days.
pub(crate) const SERVER_CERT_RENEWAL_WINDOW_DAYS: i64 = 30;

/// Server certificate validity period in days.
pub(crate) const SERVER_CERT_VALIDITY_DAYS: i64 = 90;

/// Interval for checking settings/CA version changes across instances (30 seconds).
pub(crate) const SETTINGS_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Interval for checking server certificate renewal eligibility (24 hours).
pub(crate) const SERVER_CERT_RENEWAL_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 3600);

/// Interval for cleaning up expired auth state: OIDC flows, rate limits, etc. (5 minutes).
pub(crate) const AUTH_CLEANUP_INTERVAL: Duration = Duration::from_secs(300);

/// Maximum time to wait for each background task during graceful shutdown (5 seconds).
pub(crate) const BACKGROUND_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum time to wait for the scheduler task during graceful shutdown (60 seconds).
///
/// The scheduler runs timed SQL queries and may be mid-execution when a shutdown signal
/// arrives. 60 seconds allows a running task-claim cycle to finish cleanly rather than
/// aborting mid-transaction and leaving stale `locked_by` rows.
#[cfg(feature = "embedded-scheduler")]
pub(crate) const SCHEDULER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(60);

/// Duration over which to scatter `ServerRestarting` notifications to avoid thundering herd
/// (5 seconds).
pub(crate) const RESTART_NOTIFICATION_SCATTER: Duration = Duration::from_secs(5);

/// Polling interval when waiting for connected services to drain during shutdown (250 ms).
pub(crate) const SERVICE_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(250);
