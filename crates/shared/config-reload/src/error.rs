use std::path::PathBuf;

/// Errors produced by the config-reload subsystem.
///
/// Covers file I/O, TOML parse failures, validation, apply/revert phases,
/// watchdog timeouts, degraded-state detection, and reconciler DB queries.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigReloadError {
    /// Failed to read a TOML config file from disk.
    #[error("failed to read TOML at {path}: {source_msg}")]
    TomlIo {
        /// Path of the file that could not be read.
        path: PathBuf,
        /// Source error message.
        source_msg: String,
    },

    /// Failed to parse the TOML content of a config file.
    #[error("failed to parse TOML at {path}: {source_msg}")]
    TomlParse {
        /// Path of the file that failed to parse.
        path: PathBuf,
        /// Source error message.
        source_msg: String,
    },

    /// Config validation rejected a field value or cross-field constraint.
    #[error("config validation failed: {0}")]
    Validate(String),

    /// A subsystem's apply phase returned an error.
    #[error("apply phase failed for subsystem `{subsystem}`: {message}")]
    ApplyFailed {
        /// Name of the subsystem that failed.
        subsystem: String,
        /// Human-readable failure description.
        message: String,
    },

    /// A subsystem's revert phase returned an error.
    #[error("revert failed for subsystem `{subsystem}`: {message}")]
    RevertFailed {
        /// Name of the subsystem that failed to revert.
        subsystem: String,
        /// Human-readable failure description.
        message: String,
    },

    /// A post-apply health check failed for a subsystem.
    #[error("health check failed for subsystem `{subsystem}`: {message}")]
    HealthFailed {
        /// Name of the subsystem whose health check failed.
        subsystem: String,
        /// Human-readable failure description.
        message: String,
    },

    /// The watchdog for a subsystem did not respond within its deadline.
    #[error("watchdog timed out for subsystem `{subsystem}` after {ms} ms")]
    WatchdogTimeout {
        /// Name of the subsystem that timed out.
        subsystem: String,
        /// Elapsed duration in milliseconds.
        ms: u128,
    },

    /// The coordinator entered the Degraded state after a partial failure.
    #[error("coordinator in Degraded state; failed subsystems: {failed:?}")]
    Degraded {
        /// Names of subsystems that could not be reverted.
        failed: Vec<String>,
    },

    /// A reconciler database query failed.
    #[error("reconciler DB query failed: {0}")]
    Reconciler(String),
}
