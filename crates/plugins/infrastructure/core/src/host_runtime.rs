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

/// Construct a [`StandardHostRuntime`] for local/POSIX hosts.
///
/// This function is used by `agent-runtime` for the embedded agent path where
/// all hosts are local POSIX machines. For remote SSH hosts (including RouterOS),
/// `agent-ssh` builds the appropriate runtime in `client::build_host_runtime`.
pub fn construct_host_runtime(
    executor: Arc<dyn CommandExecutor>,
    caps: HostCapabilities,
) -> Arc<dyn HostRuntime> {
    Arc::new(StandardHostRuntime::new(executor, caps))
}

// ── RouterOS runtime ─────────────────────────────────────────────────

/// Typed RouterOS CLI methods used by the RouterOS plugin.
///
/// Defined here (in `plugin-infrastructure-core`) so that the plugin crate
/// can depend on it without depending on `agent-ssh`. `RouterOsSshExecutor`
/// in `agent-ssh` implements this trait.
///
/// All methods return raw stdout from the RouterOS CLI. Parsing is the
/// caller's responsibility.
#[async_trait::async_trait]
pub trait RouterOsExecutor: Send + Sync + 'static {
    /// `/system resource print`
    async fn resource_print(&self) -> std::result::Result<String, crate::PluginError>;

    /// `/system routerboard print`
    async fn routerboard_print(&self) -> std::result::Result<String, crate::PluginError>;

    /// `/system license print`
    async fn license_print(&self) -> std::result::Result<String, crate::PluginError>;

    /// `/system package update check-for-updates` — triggers an async background
    /// check on the router. RouterOS caches the result; subsequent `package_update_print`
    /// calls will show `latest-version` once the check completes. Callers must
    /// wait (poll or fixed delay) before calling `package_install`/`package_download`.
    async fn check_for_updates(&self) -> std::result::Result<(), crate::PluginError>;

    /// `/system package update print`
    async fn package_update_print(&self) -> std::result::Result<String, crate::PluginError>;

    /// `/system package update install` — downloads + reboots.
    async fn package_install(&self) -> std::result::Result<(), crate::PluginError>;

    /// `/system package update download` — downloads without rebooting.
    async fn package_download(&self) -> std::result::Result<(), crate::PluginError>;
}

/// Host runtime for MikroTik RouterOS devices.
///
/// Carries the typed RouterOS CLI executor and the `allow_reboot` flag read
/// from `routeros_host_config` by `agent-ssh` at construction time.
///
/// The RouterOS plugin downcasts `runtime.as_any()` to this type to access
/// both the executor and the flag. No DB access is needed in the plugin crate.
pub struct RouterOsHostRuntime {
    routeros_exec: Arc<dyn RouterOsExecutor>,
    capabilities: HostCapabilities,
    /// Whether the RouterOS `reboot` policy was granted at bootstrap time.
    /// Hard gate: reboot is impossible without the policy on the router group.
    pub allow_reboot: bool,
}

impl RouterOsHostRuntime {
    /// Create a new RouterOS host runtime.
    pub fn new(
        routeros_exec: Arc<dyn RouterOsExecutor>,
        caps: HostCapabilities,
        allow_reboot: bool,
    ) -> Self {
        Self {
            routeros_exec,
            capabilities: caps,
            allow_reboot,
        }
    }

    /// The typed RouterOS CLI executor.
    pub fn routeros_executor(&self) -> Arc<dyn RouterOsExecutor> {
        Arc::clone(&self.routeros_exec)
    }
}

impl HostRuntime for RouterOsHostRuntime {
    fn capabilities(&self) -> &HostCapabilities {
        &self.capabilities
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executor(&self) -> Arc<dyn CommandExecutor> {
        // RouterOS does not use the generic CommandExecutor interface.
        // Return a NoopCommandExecutor so misuse is visible in traces
        // without crashing the process.
        tracing::error!("RouterOsHostRuntime::executor() called — use routeros_executor() instead");
        Arc::new(uptrakit_command::NoopCommandExecutor)
    }
}

/// Construct a `RouterOsHostRuntime` for a RouterOS host.
///
/// Called by `agent-ssh` instead of `construct_host_runtime` when the host
/// has a `routeros_host_config` DB row.
pub fn construct_routeros_host_runtime(
    routeros_exec: Arc<dyn RouterOsExecutor>,
    caps: HostCapabilities,
    allow_reboot: bool,
) -> Arc<dyn HostRuntime> {
    Arc::new(RouterOsHostRuntime::new(routeros_exec, caps, allow_reboot))
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

    #[test]
    fn routeros_runtime_executor_returns_noop() {
        struct MockExec;
        #[async_trait::async_trait]
        impl RouterOsExecutor for MockExec {
            async fn resource_print(&self) -> std::result::Result<String, crate::PluginError> {
                Ok(String::new())
            }
            async fn routerboard_print(&self) -> std::result::Result<String, crate::PluginError> {
                Ok(String::new())
            }
            async fn license_print(&self) -> std::result::Result<String, crate::PluginError> {
                Ok(String::new())
            }
            async fn check_for_updates(&self) -> std::result::Result<(), crate::PluginError> {
                Ok(())
            }
            async fn package_update_print(
                &self,
            ) -> std::result::Result<String, crate::PluginError> {
                Ok(String::new())
            }
            async fn package_install(&self) -> std::result::Result<(), crate::PluginError> {
                Ok(())
            }
            async fn package_download(&self) -> std::result::Result<(), crate::PluginError> {
                Ok(())
            }
        }

        let exec: Arc<dyn RouterOsExecutor> = Arc::new(MockExec);
        let caps = HostCapabilities::new(Some("routeros"), None, None, &[]);
        let runtime = RouterOsHostRuntime::new(exec, caps, true);
        // executor() must not panic
        let _noop = runtime.executor();
        assert!(runtime.allow_reboot);
    }

    #[test]
    fn construct_routeros_host_runtime_downcast() {
        struct MockExec;
        #[async_trait::async_trait]
        impl RouterOsExecutor for MockExec {
            async fn resource_print(&self) -> std::result::Result<String, crate::PluginError> {
                Ok(String::new())
            }
            async fn routerboard_print(&self) -> std::result::Result<String, crate::PluginError> {
                Ok(String::new())
            }
            async fn license_print(&self) -> std::result::Result<String, crate::PluginError> {
                Ok(String::new())
            }
            async fn check_for_updates(&self) -> std::result::Result<(), crate::PluginError> {
                Ok(())
            }
            async fn package_update_print(
                &self,
            ) -> std::result::Result<String, crate::PluginError> {
                Ok(String::new())
            }
            async fn package_install(&self) -> std::result::Result<(), crate::PluginError> {
                Ok(())
            }
            async fn package_download(&self) -> std::result::Result<(), crate::PluginError> {
                Ok(())
            }
        }
        let runtime = construct_routeros_host_runtime(
            Arc::new(MockExec),
            HostCapabilities::new(Some("routeros"), None, None, &[]),
            false,
        );
        let downcast = runtime.as_any().downcast_ref::<RouterOsHostRuntime>();
        assert!(downcast.is_some());
        assert!(!downcast.unwrap().allow_reboot);
    }
}
