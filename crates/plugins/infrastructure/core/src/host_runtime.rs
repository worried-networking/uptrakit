//! Host runtime abstraction.
//!
//! [`HostRuntime`] makes the execution environment a first-class concept.
//! Plugins receive `Arc<dyn HostRuntime>` at creation time and downcast to the
//! concrete runtime type they need via `as_any()`.
//!
//! [`PosixHostRuntime`] wraps the existing `CommandExecutor` for POSIX hosts.
//! [`construct_host_runtime`] is the single dispatch point where runtime type
//! selection happens.

use std::sync::Arc;

use rootcause::report;
use uptrakit_command::CommandExecutor;
use uptrakit_shared_types::HostCapabilities;

use crate::error::PluginError;

/// Execution environment provided to plugins at creation time.
///
/// Adding a new host type means implementing this trait in a new struct and
/// adding a match arm to [`construct_host_runtime()`]. The core trait itself is
/// stable — no typed accessors for specific runtimes.
///
/// Plugins obtain their runtime-specific interface by downcasting via `as_any()`.
/// A mismatched runtime returns a clear error at plugin construction time.
pub trait HostRuntime: Send + Sync + 'static {
    /// Host capabilities for runtime compatibility checks.
    fn capabilities(&self) -> &HostCapabilities;

    /// Downcast to the concrete runtime type.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Runtime for POSIX hosts. Wraps the existing `CommandExecutor`.
///
/// Plugins downcast to this to get the executor:
/// ```ignore
/// let posix = runtime.as_any().downcast_ref::<PosixHostRuntime>()
///     .ok_or(...)?;
/// let executor = posix.executor().clone();
/// ```
pub struct PosixHostRuntime {
    executor: Arc<dyn CommandExecutor>,
    capabilities: HostCapabilities,
}

impl PosixHostRuntime {
    /// Create a new POSIX host runtime.
    pub fn new(executor: Arc<dyn CommandExecutor>, capabilities: HostCapabilities) -> Self {
        Self {
            executor,
            capabilities,
        }
    }

    /// Access the POSIX command executor.
    pub fn executor(&self) -> &Arc<dyn CommandExecutor> {
        &self.executor
    }
}

impl HostRuntime for PosixHostRuntime {
    fn capabilities(&self) -> &HostCapabilities {
        &self.capabilities
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Downcast to `PosixHostRuntime` and return the executor, or error.
///
/// Convenience helper for POSIX plugins that need `Arc<dyn CommandExecutor>`.
pub fn require_posix_executor(
    runtime: &dyn HostRuntime,
) -> crate::error::Result<Arc<dyn CommandExecutor>> {
    runtime
        .as_any()
        .downcast_ref::<PosixHostRuntime>()
        .map(|r| Arc::clone(r.executor()))
        .ok_or_else(|| {
            report!(PluginError::Configuration(
                "this plugin requires a POSIX host runtime".to_string()
            ))
        })
}

/// Construct the appropriate `HostRuntime` for a host based on its capabilities.
///
/// Currently always returns `PosixHostRuntime`. When non-POSIX host types are
/// added (e.g., RouterOS), this function dispatches based on `caps.os_family`.
/// This is the SINGLE point where runtime type selection happens.
pub fn construct_host_runtime(
    executor: Arc<dyn CommandExecutor>,
    caps: HostCapabilities,
) -> Arc<dyn HostRuntime> {
    // Future: match on caps.os_family to select the right runtime
    // e.g., Some(OsFamily::RouterOs) => Arc::new(RouterOsHostRuntime::new(session, caps))
    Arc::new(PosixHostRuntime::new(executor, caps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::{OsFamily, host_features};

    #[test]
    fn posix_runtime_downcast() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
        let runtime: Arc<dyn HostRuntime> = Arc::new(PosixHostRuntime::new(executor, caps));

        let posix = runtime.as_any().downcast_ref::<PosixHostRuntime>();
        assert!(posix.is_some());
        assert_eq!(
            posix.unwrap().capabilities().os_family,
            Some(OsFamily::Linux)
        );
    }

    #[test]
    fn require_posix_executor_succeeds() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(Some("linux"), None, None, &["posix_shell".to_string()]);
        let runtime = construct_host_runtime(executor, caps);

        let result = require_posix_executor(runtime.as_ref());
        assert!(result.is_ok());
    }

    #[test]
    fn construct_host_runtime_preserves_capabilities() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(
            Some("linux"),
            Some("Ubuntu 24.04"),
            Some("x86_64"),
            &["posix_shell".to_string(), "systemd".to_string()],
        );
        let runtime = construct_host_runtime(executor, caps);

        assert_eq!(runtime.capabilities().os_family, Some(OsFamily::Linux));
        assert!(
            runtime
                .capabilities()
                .has_feature(host_features::POSIX_SHELL)
        );
        assert!(runtime.capabilities().has_feature(host_features::SYSTEMD));
    }
}
