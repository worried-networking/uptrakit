use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::PluginCapability;
use uptrakit_plugin_infrastructure_core::command::CommandExecutor;

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
    pub(crate) executor: Arc<dyn CommandExecutor>,
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
    /// With the `daemon` feature enabled, connects to the Docker daemon via
    /// bollard. Without it, uses [`NoopDockerClient`] so the plugin can still
    /// serve registry-only capabilities (e.g. `ControllerSideFetchReleases`).
    pub async fn new(config: DockerConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        // Always create a NoopDockerClient and a None proxy handle as
        // starting values. When the `daemon` feature is enabled,
        // upgrade_to_daemon_client replaces them with a real
        // BollardDockerClient (and optionally a proxy handle), consuming
        // the stub values in the process.
        let docker_client: Arc<dyn DockerClient> = Arc::new(NoopDockerClient);
        let proxy_handle: OpaqueHandle = None;
        #[cfg(feature = "daemon")]
        let (docker_client, proxy_handle) =
            Self::upgrade_to_daemon_client(docker_client, proxy_handle, &config, &executor).await?;
        Self::init(config, executor, docker_client, proxy_handle)
    }

    /// Internal constructor that accepts any [`DockerClient`] implementation.
    pub(crate) fn init(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
        #[cfg_attr(not(feature = "daemon"), allow(unused_variables))] proxy_handle: OpaqueHandle,
    ) -> Result<Self> {
        config.validate()?;

        let registry_client: Arc<dyn RegistryClientOps> =
            Arc::new(RegistryClient::new(config.auth.clone())?);

        Ok(Self {
            config,
            registry_client,
            docker_client: parking_lot::Mutex::new(docker_client),
            executor,
            #[cfg(feature = "daemon")]
            proxy_handle: parking_lot::Mutex::new(proxy_handle),
            #[cfg(feature = "daemon")]
            detected_runtime: parking_lot::Mutex::new(None),
            #[cfg(feature = "daemon")]
            credential_cache: crate::credentials::CredentialCache::new(),
        })
    }

    /// Compile-time capabilities for the Docker plugin.
    ///
    /// Read directly by the registry macro for sync capability queries.
    /// All capabilities are declared unconditionally so the controller's
    /// `discovery_plugins()` always includes Docker in discovery assignments.
    /// The actual `discover_software()` and `detect_host_compatibility()`
    /// implementations are gated behind `#[cfg(feature = "daemon")]` on the
    /// trait impl — the controller never calls them; it only sends the
    /// assignment to the agent over WebSocket.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::ControllerSideFetchReleases,
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::DetectHostCompatibility,
    ];

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
        config.validate()?;
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
}

// ── PluginBase + subtrait implementations ────────────────────────────────

uptrakit_plugin_infrastructure_core::impl_plugin_base_config!(
    DockerPlugin,
    DockerConfig,
    "releases_docker",
    {
        fn capabilities(&self) -> Vec<uptrakit_plugin_infrastructure_core::PluginCapability> {
            Self::CAPABILITIES.to_vec()
        }

        fn as_discovery(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::DiscoveryPlugin> {
            Some(self)
        }
        fn as_version_detector(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::VersionDetectorPlugin> {
            Some(self)
        }
        fn as_release_fetcher(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin> {
            Some(self)
        }
        fn as_update_executor(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin> {
            Some(self)
        }
    }
);

#[cfg(all(test, feature = "daemon"))]
#[path = "tests.rs"]
mod tests;
