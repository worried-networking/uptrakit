use std::time::Duration;

// Per spec §9.1. TODO: expose as TOML keys if operator demand surfaces.

/// Watchdog deadline for the DB connection pool health check.
pub const WATCHDOG_DB_POOL: Duration = Duration::from_secs(15);
/// Watchdog deadline for the NATS connection health check.
pub const WATCHDOG_NATS: Duration = Duration::from_secs(10);
/// Watchdog deadline for the HTTPS listener health check.
pub const WATCHDOG_HTTPS: Duration = Duration::from_secs(5);
/// Watchdog deadline for the PKI listener health check.
pub const WATCHDOG_PKI: Duration = Duration::from_secs(5);
/// Watchdog deadline for the plugin registry health check.
pub const WATCHDOG_PLUGINS: Duration = Duration::from_secs(30);
/// Watchdog deadline for the audit log subsystem health check.
pub const WATCHDOG_AUDIT: Duration = Duration::from_secs(5);
/// Watchdog deadline for the zeroconf subsystem health check.
pub const WATCHDOG_ZEROCONF: Duration = Duration::from_secs(5);
/// Watchdog deadline for the embedded-services health check.
pub const WATCHDOG_EMBEDDED: Duration = Duration::from_secs(30);

/// Maximum time to drain in-flight HTTPS requests before forcing shutdown.
pub const HTTPS_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum time to drain in-flight PKI requests before forcing shutdown.
pub const PKI_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Debounce window for filesystem watch events before triggering a reload.
pub const FILE_WATCH_DEBOUNCE: Duration = Duration::from_millis(500);
/// Polling interval for the reconciler DB settings-version check.
pub const RECONCILER_POLL: Duration = Duration::from_secs(2);
