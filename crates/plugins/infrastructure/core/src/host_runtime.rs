//! Host runtime abstraction.
//!
//! [`HostRuntime`] makes the execution environment a first-class concept.
//! Plugins receive `Arc<dyn HostRuntime>` at creation time and call
//! `runtime.executor()` to obtain the command executor.
//!
//! [`StandardHostRuntime`] is the standard implementation wrapping a
//! `CommandExecutor` and `HostCapabilities`. It is platform-neutral — the
//! same struct is used for POSIX, Windows, and other host types.
//!
//! [`construct_host_runtime`] is the single dispatch point where runtime
//! type selection happens on the agent side.

use std::sync::Arc;

use uptrakit_command::CommandExecutor;
use uptrakit_shared_types::HostCapabilities;

/// Execution environment provided to plugins at creation time.
///
/// Plugins obtain the command executor via [`executor()`](HostRuntime::executor).
/// Host compatibility is enforced at dispatch time by `HostRequirements`, not
/// at construction time — plugins do not need to downcast to a specific runtime.
pub trait HostRuntime: Send + Sync + 'static {
    /// Host capabilities for runtime compatibility checks.
    fn capabilities(&self) -> &HostCapabilities;

    /// Downcast to the concrete runtime type.
    fn as_any(&self) -> &dyn std::any::Any;

    /// The command executor for this runtime.
    fn executor(&self) -> Arc<dyn CommandExecutor>;

    /// Returns the controller's self-metadata provider, if available.
    /// Only the controller-standalone overrides this — standalone agents return `None`.
    fn metadata_provider(
        &self,
    ) -> Option<Arc<dyn crate::service_metadata::ServiceMetadataProvider>> {
        None
    }
}

/// Standard host runtime wrapping a command executor and capabilities.
///
/// Platform-neutral — used for POSIX, Windows (future), and other host types.
/// The executor implementation determines the actual command execution strategy
/// (local shell, SSH, device API, etc.).
pub struct StandardHostRuntime {
    executor: Arc<dyn CommandExecutor>,
    capabilities: HostCapabilities,
}

impl StandardHostRuntime {
    /// Create a new standard host runtime.
    pub fn new(executor: Arc<dyn CommandExecutor>, capabilities: HostCapabilities) -> Self {
        Self {
            executor,
            capabilities,
        }
    }
}

impl HostRuntime for StandardHostRuntime {
    fn capabilities(&self) -> &HostCapabilities {
        &self.capabilities
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executor(&self) -> Arc<dyn CommandExecutor> {
        Arc::clone(&self.executor)
    }
}

/// A [`HostRuntime`] wrapper that injects a [`ServiceMetadataProvider`] so
/// plugins running inside a controller process can introspect the controller
/// binary itself.
///
/// All other [`HostRuntime`] methods delegate transparently to the wrapped
/// inner runtime.
pub struct MetadataAwareHostRuntime {
    inner: Arc<dyn HostRuntime>,
    provider: Arc<dyn crate::service_metadata::ServiceMetadataProvider>,
}

impl MetadataAwareHostRuntime {
    /// Wrap `inner` with a fixed metadata `provider`.
    ///
    /// Returns an `Arc` so callers can use the result directly as
    /// `Arc<dyn HostRuntime>`.
    pub fn new(
        inner: Arc<dyn HostRuntime>,
        provider: Arc<dyn crate::service_metadata::ServiceMetadataProvider>,
    ) -> Arc<Self> {
        Arc::new(Self { inner, provider })
    }
}

impl HostRuntime for MetadataAwareHostRuntime {
    fn capabilities(&self) -> &HostCapabilities {
        self.inner.capabilities()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executor(&self) -> Arc<dyn CommandExecutor> {
        self.inner.executor()
    }

    fn metadata_provider(
        &self,
    ) -> Option<Arc<dyn crate::service_metadata::ServiceMetadataProvider>> {
        Some(Arc::clone(&self.provider))
    }
}

/// Construct the appropriate `HostRuntime` for a host based on its capabilities.
///
/// Currently always returns [`StandardHostRuntime`]. When non-standard host
/// types are added (e.g., RouterOS), this function dispatches based on
/// `caps.os_family`. This is the SINGLE point where runtime type selection
/// happens on the agent side.
pub fn construct_host_runtime(
    executor: Arc<dyn CommandExecutor>,
    caps: HostCapabilities,
) -> Arc<dyn HostRuntime> {
    // Future: match on caps.os_family to select the right runtime
    // e.g., Some(OsFamily::RouterOs) => Arc::new(RouterOsHostRuntime::new(session, caps))
    Arc::new(StandardHostRuntime::new(executor, caps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::{OsFamily, host_features};

    #[test]
    fn standard_runtime_executor_returns_executor() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
        let runtime: Arc<dyn HostRuntime> = Arc::new(StandardHostRuntime::new(executor, caps));
        let _exec = runtime.executor();
    }

    #[test]
    fn standard_runtime_downcast() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
        let runtime: Arc<dyn HostRuntime> = Arc::new(StandardHostRuntime::new(executor, caps));

        let std_rt = runtime.as_any().downcast_ref::<StandardHostRuntime>();
        assert!(std_rt.is_some());
        assert_eq!(
            std_rt.unwrap().capabilities().os_family,
            Some(OsFamily::Linux)
        );
    }

    #[test]
    fn executor_via_construct_host_runtime() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(Some("linux"), None, None, &["posix_shell".to_string()]);
        let runtime = construct_host_runtime(executor, caps);
        let _exec = runtime.executor();
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

    #[test]
    fn test_standard_host_runtime_metadata_provider_returns_none() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let runtime = construct_host_runtime(executor, Default::default());
        assert!(runtime.metadata_provider().is_none());
    }
}
