use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, HostRequirements, HostRuntime, PluginFamily, declare_plugin,
};

use crate::config::DockerConfig;
use crate::docker_client::{DockerClient, NoopDockerClient};
use crate::error::Result;
use crate::registry::{RegistryClient, RegistryClientOps};

/// Type-erased RAII handle kept alive alongside the Docker client.
pub(crate) type OpaqueHandle = Option<Box<dyn std::any::Any + Send + Sync>>;

/// Docker plugin implementation.
///
/// Tracks container image updates by monitoring the SHA-256 manifest digest
/// of a specific tag (e.g. `latest`). When the remote digest differs from the
/// locally installed digest, an update is available.
///
/// Also supports autodiscovery of running/stopped containers via Bollard.
pub struct DockerPlugin {
    pub(crate) config: DockerConfig,
    pub(crate) registry_client: Arc<dyn RegistryClientOps>,
    pub(crate) docker_client: parking_lot::Mutex<Arc<dyn DockerClient>>,
    /// Command executor for agent-side operations. `None` when instantiated
    /// on the controller (where only `ReleaseFetcher` runs).
    pub(crate) executor: Option<Arc<dyn CommandExecutor>>,
    /// RAII handle for the Docker socket proxy (Unix-only, daemon feature).
    ///
    /// When an executor supports stdio tunnels and no explicit `docker_host`
    /// is configured, a [`crate::docker_proxy::DockerSocketProxy`] is started
    /// and stored here. The proxy is stopped and the socket removed when the
    /// plugin is dropped.
    #[cfg(feature = "daemon")]
    pub(crate) proxy_handle: parking_lot::Mutex<OpaqueHandle>,
    /// Container runtime detected during `detect_host_compatibility` (Auto mode).
    #[cfg(feature = "daemon")]
    pub(crate) detected_runtime: parking_lot::Mutex<Option<crate::config::ContainerRuntime>>,
    /// Cache of resolved system credentials (keyed by registry hostname).
    #[cfg(feature = "daemon")]
    pub(crate) credential_cache: crate::credentials::CredentialCache,
}

impl DockerPlugin {
    /// Create a new `DockerPlugin` from the given configuration.
    ///
    /// The HTTP registry client is built eagerly. The Docker daemon client
    /// starts as [`NoopDockerClient`] and is upgraded to a real Bollard client
    /// via [`Self::ensure_daemon_client()`] on first use (requires async).
    /// The POSIX executor is obtained from the runtime if available (agent-side);
    /// on the controller side it will be `None`.
    pub fn new(
        config: DockerConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        config.validate_inner().map_err(|e| e.to_string())?;

        let registry_client: Arc<dyn RegistryClientOps> =
            Arc::new(RegistryClient::new(config.auth.clone()).map_err(|e| e.to_string())?);

        let executor = Some(runtime.executor());

        let docker_client: Arc<dyn DockerClient> = Arc::new(NoopDockerClient);

        Ok(Self {
            config,
            registry_client,
            docker_client: parking_lot::Mutex::new(docker_client),
            executor,
            #[cfg(feature = "daemon")]
            proxy_handle: parking_lot::Mutex::new(None),
            #[cfg(feature = "daemon")]
            detected_runtime: parking_lot::Mutex::new(None),
            #[cfg(feature = "daemon")]
            credential_cache: crate::credentials::CredentialCache::new(),
        })
    }

    /// Upgrade the Docker client from a noop stub to a real Bollard client.
    ///
    /// This is called lazily from role methods that need the Docker daemon.
    /// With the `daemon` feature enabled, connects to the Docker daemon via
    /// bollard. Without it, the [`NoopDockerClient`] remains.
    #[cfg(feature = "daemon")]
    #[allow(dead_code)]
    pub(crate) async fn ensure_daemon_client(&self) -> Result<()> {
        let Some(ref executor) = self.executor else {
            // No executor means controller-side; NoopDockerClient is fine.
            return Ok(());
        };

        let stub: Arc<dyn DockerClient> = Arc::new(NoopDockerClient);
        let proxy_stub: OpaqueHandle = None;
        let (client, proxy_handle) =
            Self::upgrade_to_daemon_client(stub, proxy_stub, &self.config, executor).await?;

        *self.docker_client.lock() = client;
        *self.proxy_handle.lock() = proxy_handle;
        Ok(())
    }

    /// Internal constructor that accepts any [`DockerClient`] implementation.
    ///
    /// Used by the old async `new_async()` path and by test constructors.
    pub(crate) fn init(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
        #[cfg_attr(not(feature = "daemon"), allow(unused_variables))] proxy_handle: OpaqueHandle,
    ) -> Result<Self> {
        config.validate_inner()?;

        let registry_client: Arc<dyn RegistryClientOps> =
            Arc::new(RegistryClient::new(config.auth.clone())?);

        Ok(Self {
            config,
            registry_client,
            docker_client: parking_lot::Mutex::new(docker_client),
            executor: Some(executor),
            #[cfg(feature = "daemon")]
            proxy_handle: parking_lot::Mutex::new(proxy_handle),
            #[cfg(feature = "daemon")]
            detected_runtime: parking_lot::Mutex::new(None),
            #[cfg(feature = "daemon")]
            credential_cache: crate::credentials::CredentialCache::new(),
        })
    }

    /// Async constructor that connects to the Docker daemon immediately.
    ///
    /// Retained for backward compatibility with code paths that need a fully
    /// initialized daemon connection at construction time.
    #[allow(dead_code)]
    pub(crate) async fn new_async(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> Result<Self> {
        let docker_client: Arc<dyn DockerClient> = Arc::new(NoopDockerClient);
        let proxy_handle: OpaqueHandle = None;
        #[cfg(feature = "daemon")]
        let (docker_client, proxy_handle) =
            Self::upgrade_to_daemon_client(docker_client, proxy_handle, &config, &executor).await?;
        Self::init(config, executor, docker_client, proxy_handle)
    }

    /// Test constructor that injects a custom [`DockerClient`].
    ///
    /// Uses [`MockRegistryClient::default()`] for the registry client so that
    /// tests never make real network calls to Docker Hub.
    #[cfg(all(test, feature = "daemon"))]
    pub(crate) fn new_for_test(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
    ) -> Result<Self> {
        use crate::registry::MockRegistryClient;
        Self::new_for_test_with_registry(
            config,
            executor,
            docker_client,
            Arc::new(MockRegistryClient::default()),
        )
    }

    /// Test constructor that injects both a custom [`DockerClient`] and a
    /// custom [`RegistryClientOps`] implementation, allowing registry calls
    /// to be mocked without network access.
    #[cfg(all(test, feature = "daemon"))]
    pub(crate) fn new_for_test_with_registry(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
        registry_client: Arc<dyn RegistryClientOps>,
    ) -> Result<Self> {
        config.validate_inner()?;
        Ok(Self {
            config,
            registry_client,
            docker_client: parking_lot::Mutex::new(docker_client),
            executor: Some(executor),
            #[cfg(feature = "daemon")]
            proxy_handle: parking_lot::Mutex::new(None),
            #[cfg(feature = "daemon")]
            detected_runtime: parking_lot::Mutex::new(None),
            #[cfg(feature = "daemon")]
            credential_cache: crate::credentials::CredentialCache::new(),
        })
    }

    /// Returns `true` when `labels` passes the configured include/exclude filters.
    ///
    /// - `include_labels`: ALL specified labels must be present with matching values.
    /// - `exclude_labels`: if ANY specified label matches, the container is excluded.
    /// - Empty maps mean no filter (all containers pass).
    pub(crate) fn container_passes_label_filter(
        &self,
        labels: &std::collections::HashMap<String, String>,
    ) -> bool {
        // Include filter: every required label must be present with the right value.
        for (key, value) in &self.config.include_labels {
            if labels.get(key).map(|v| v.as_str()) != Some(value.as_str()) {
                return false;
            }
        }
        // Exclude filter: reject if any excluded label matches.
        for (key, value) in &self.config.exclude_labels {
            if labels.get(key).map(|v| v.as_str()) == Some(value.as_str()) {
                return false;
            }
        }
        true
    }

    /// Return a reference to the command executor, or an error if unavailable.
    ///
    /// The executor is `None` when the plugin is instantiated on the
    /// controller (where only `ReleaseFetcher` runs). Agent-side roles
    /// that need the executor call this helper.
    #[cfg(feature = "daemon")]
    pub(crate) fn require_executor(
        &self,
    ) -> std::result::Result<&Arc<dyn CommandExecutor>, crate::error::DockerError> {
        self.executor.as_ref().ok_or_else(|| {
            crate::error::DockerError::Configuration(
                "POSIX executor not available (controller-side instance)".to_string(),
            )
        })
    }

    /// Return extension manifests for the Docker plugin.
    pub fn extension_manifests_static() -> Vec<uptrakit_extension_framework::ExtensionManifest> {
        crate::extensions::extension_manifests()
    }

    /// Return extension action definitions for the Docker plugin.
    pub fn extension_actions_static() -> Vec<uptrakit_extension_framework::ActionDef> {
        crate::extensions::extension_actions()
    }
}

/// Extension action handler wrapper for the `declare_plugin!` macro.
///
/// This function matches the `ExtensionActionHandler` type signature, which
/// receives `descriptor::ExtensionActionContext` (with `db: &dyn Any`).
/// The downcast to `&DatabaseConnection` happens inside
/// `crate::extensions::handle_action`.
fn docker_handle_extension_action<'a>(
    ctx: &'a uptrakit_plugin_infrastructure_core::descriptor::ExtensionActionContext<'a>,
    extension_id: &'a str,
    action_id: &'a str,
    params: serde_json::Value,
) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>> {
    Box::pin(crate::extensions::handle_action(
        ctx,
        extension_id,
        action_id,
        params,
    ))
}

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(DockerPlugin, DockerConfig, "releases_docker", {
    display_name: "Docker",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    roles: [Discoverer, VersionDetector, ReleaseFetcher, UpdateExecutor]
    , extra_capabilities: [
        uptrakit_plugin_infrastructure_core::PluginCapability::ControllerSideFetchReleases,
        uptrakit_plugin_infrastructure_core::PluginCapability::DetectHostCompatibility,
    ]
    , owned_extension_ids: &["docker."]
    , extensions: {
        manifests: DockerPlugin::extension_manifests_static,
        actions: DockerPlugin::extension_actions_static,
        handle_action: docker_handle_extension_action,
    }
});

#[cfg(all(test, feature = "daemon"))]
#[path = "tests.rs"]
mod tests;
