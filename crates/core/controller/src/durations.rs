//! Named constants for recurring durations used across the controller.
//!
//! Replaces scattered hardcoded values in `main.rs`, `pki.rs`, and `crl_manager.rs`.

use std::time::Duration;

/// CA rotation window: rotate when the CA certificate expires within this many days.
pub const CA_ROTATION_WINDOW_DAYS: i64 = 183;

/// Server certificate renewal window: renew when the cert expires within this many days.
pub const SERVER_CERT_RENEWAL_WINDOW_DAYS: i64 = 30;

/// Server certificate validity period in days.
pub const SERVER_CERT_VALIDITY_DAYS: i64 = 90;

/// Interval for CRL version-gated polling (60 seconds).
pub const CRL_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Interval for checking settings/CA version changes across instances (30 seconds).
pub const SETTINGS_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Interval for checking server certificate renewal eligibility (24 hours).
pub const SERVER_CERT_RENEWAL_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 3600);

/// Interval for cleaning up expired auth state: OIDC flows, rate limits, etc. (5 minutes).
pub const AUTH_CLEANUP_INTERVAL: Duration = Duration::from_secs(300);

/// Maximum time to wait for each background task during graceful shutdown (5 seconds).
pub const BACKGROUND_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Duration over which to scatter `ServerRestarting` notifications to avoid thundering herd
/// (5 seconds).
pub const RESTART_NOTIFICATION_SCATTER: Duration = Duration::from_secs(5);

/// Polling interval when waiting for connected services to drain during shutdown (250 ms).
pub const SERVICE_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(250);
