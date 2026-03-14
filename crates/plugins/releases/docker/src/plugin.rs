use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use serde_json::json;
use uptrakit_plugin_infrastructure_core::mpsc;

use uptrakit_plugin_infrastructure_core::command::{
    CommandExecutor, CommandSpec, send_output, shell_escape,
};
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, DiscoveredSoftware, OutputStreamType, PluginCapability,
    PluginError, ReleaseInfo, UpdateOutputLine, UpstreamRelease, Version,
};

/// Type-erased RAII handle kept alive alongside the Docker client.
type OpaqueHandle = Option<Box<dyn std::any::Any + Send + Sync>>;

#[cfg(feature = "daemon")]
use crate::config::ContainerRuntime;
use crate::config::DockerConfig;
#[cfg(feature = "daemon")]
use crate::docker_client::BollardDockerClient;
use crate::docker_client::{DockerClient, NoopDockerClient};
use crate::error::Result;
use crate::image_ref::ImageRef;
use crate::registry::RegistryClient;
#[cfg(feature = "daemon")]
use std::time::Duration;
#[cfg(feature = "daemon")]
use uptrakit_plugin_infrastructure_core::HostCompatibility;

/// Docker plugin implementation.
///
/// Tracks container image updates by monitoring the SHA-256 manifest digest
/// of a specific tag (e.g. `latest`). When the remote digest differs from the
/// locally installed digest, an update is available.
///
/// Also supports autodiscovery of running/stopped containers via Bollard.
pub struct DockerPlugin {
    config: DockerConfig,
    registry_client: RegistryClient,
    docker_client: parking_lot::Mutex<Arc<dyn DockerClient>>,
    executor: Arc<dyn CommandExecutor>,
    /// RAII handle for the Docker socket proxy (Unix-only, daemon feature).
    ///
    /// When an executor supports stdio tunnels and no explicit `docker_host`
    /// is configured, a [`crate::docker_proxy::DockerSocketProxy`] is started
    /// and stored here. The proxy is stopped and the socket removed when the
    /// plugin is dropped.
    #[cfg(feature = "daemon")]
    proxy_handle: parking_lot::Mutex<OpaqueHandle>,
    /// Container runtime detected during `detect_host_compatibility` (Auto mode).
    #[cfg(feature = "daemon")]
    detected_runtime: parking_lot::Mutex<Option<ContainerRuntime>>,
    /// Cache of resolved system credentials (keyed by registry hostname).
    #[cfg(feature = "daemon")]
    credential_cache: crate::credentials::CredentialCache,
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

    /// Replace a stub Docker client with a real [`BollardDockerClient`].
    ///
    /// When the executor supports stdio tunnels (e.g. SSH) and no explicit
    /// `docker_host` is configured, a [`crate::docker_proxy::DockerSocketProxy`]
    /// is started and the client connects to the local proxy socket. Otherwise
    /// falls through to the standard bollard connection logic.
    ///
    /// The `_stub` parameter is the [`NoopDockerClient`] created unconditionally
    /// in [`Self::new`]. Accepting it here ensures the initial binding is read,
    /// suppressing `unused_assignments` and `dead_code` lints, while making it
    /// explicit that the daemon path fully replaces the stub.
    #[cfg(feature = "daemon")]
    async fn upgrade_to_daemon_client(
        _stub: Arc<dyn DockerClient>,
        _proxy_stub: OpaqueHandle,
        config: &DockerConfig,
        executor: &Arc<dyn CommandExecutor>,
    ) -> Result<(Arc<dyn DockerClient>, OpaqueHandle)> {
        // When the executor supports stdio tunnels and no explicit docker_host
        // is configured, start a local Unix socket proxy. This avoids bollard's
        // SSH codepath (which spawns a second SSH connection via the system ssh
        // binary) and instead tunnels Docker API traffic over the existing
        // russh session.
        #[cfg(unix)]
        if executor.supports_stdio_tunnel() && config.docker_host.is_none() {
            let dial_cmd = match config.container_runtime {
                ContainerRuntime::Docker => "docker system dial-stdio",
                ContainerRuntime::Podman => "podman system dial-stdio",
                ContainerRuntime::Auto => "docker system dial-stdio", // overridden after detection
            };
            let proxy =
                crate::docker_proxy::DockerSocketProxy::start(Arc::clone(executor), dial_cmd)
                    .await?;
            let uri = proxy.socket_uri();
            tracing::info!(
                proxy_socket = %uri,
                "Docker socket proxy started; connecting bollard via proxy"
            );
            let client = Arc::new(BollardDockerClient::new(Some(&uri), None, None)?);
            let handle: Box<dyn std::any::Any + Send + Sync> = Box::new(proxy);
            return Ok((client, Some(handle)));
        }

        let client = Arc::new(BollardDockerClient::new(
            config.docker_host.as_deref(),
            config.ssh_key_path.as_deref(),
            config.tls.as_ref(),
        )?);
        Ok((client, None))
    }

    /// Returns the `dial-stdio` command string for the configured/detected runtime.
    ///
    /// Explicit `Docker`/`Podman` config always wins. In `Auto` mode the
    /// previously detected runtime is used (defaulting to `docker` if
    /// detection has not yet run).
    #[cfg(feature = "daemon")]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn effective_dial_stdio_command(&self) -> &'static str {
        match self.config.container_runtime {
            ContainerRuntime::Docker => "docker system dial-stdio",
            ContainerRuntime::Podman => "podman system dial-stdio",
            ContainerRuntime::Auto => match *self.detected_runtime.lock() {
                Some(ContainerRuntime::Podman) => "podman system dial-stdio",
                _ => "docker system dial-stdio",
            },
        }
    }

    /// Internal constructor that accepts any [`DockerClient`] implementation.
    fn init(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
        #[cfg_attr(not(feature = "daemon"), allow(unused_variables))] proxy_handle: OpaqueHandle,
    ) -> Result<Self> {
        config.validate()?;

        let registry_client = RegistryClient::new(config.auth.clone())?;

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
    #[cfg(all(test, feature = "daemon"))]
    pub(crate) fn new_for_test(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
    ) -> Result<Self> {
        Self::init(config, executor, docker_client, None)
    }

    /// Returns `true` when `labels` passes the configured include/exclude filters.
    ///
    /// - `include_labels`: ALL specified labels must be present with matching values.
    /// - `exclude_labels`: if ANY specified label matches, the container is excluded.
    /// - Empty maps mean no filter (all containers pass).
    fn container_passes_label_filter(
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

    /// Returns the effective registry authentication for the given image reference.
    ///
    /// Resolution order:
    /// 1. Explicit `config.auth` (always wins).
    /// 2. System credentials from `~/.docker/config.json` when `use_system_credentials` is true.
    /// 3. `None` (unauthenticated).
    #[cfg(feature = "daemon")]
    async fn effective_auth(&self, image: &str) -> Option<crate::config::DockerAuth> {
        // Explicit auth always wins.
        if self.config.auth.is_some() {
            return self.config.auth.clone();
        }

        if !self.config.use_system_credentials {
            return None;
        }

        // Determine whether we're accessing a remote host.
        let is_remote = self.executor.supports_stdio_tunnel();

        // Parse registry from the image reference.
        let registry = image
            .parse::<crate::image_ref::ImageRef>()
            .map(|r| r.server_address())
            .unwrap_or_else(|_| image.to_string());

        crate::credentials::resolve_system_credentials(
            &registry,
            &self.executor,
            is_remote,
            &self.credential_cache,
        )
        .await
    }

    /// Probe the executor for the available container runtime and, when running
    /// over an SSH stdio tunnel, restart the proxy with the detected command.
    ///
    /// Returns `Some(runtime)` when a runtime is found, `None` when neither
    /// Docker nor Podman is available, or an `Err` on unexpected failure.
    #[cfg(feature = "daemon")]
    async fn detect_and_apply_runtime(&self) -> crate::error::Result<Option<ContainerRuntime>> {
        const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

        // Helper: run a shell command via the executor and return true if exit 0.
        let probe = |cmd: &'static str| {
            let executor = Arc::clone(&self.executor);
            async move {
                tokio::time::timeout(
                    PROBE_TIMEOUT,
                    executor.execute_quiet(&CommandSpec::shell(cmd)),
                )
                .await
                .ok()
                .and_then(|r| r.ok())
                .map(|o| o.exit_code == 0)
                .unwrap_or(false)
            }
        };

        let runtime = if probe("command -v docker >/dev/null 2>&1").await {
            tracing::debug!("detected Docker runtime");
            ContainerRuntime::Docker
        } else if probe("command -v podman >/dev/null 2>&1").await {
            tracing::debug!("detected Podman runtime");
            ContainerRuntime::Podman
        } else {
            return Ok(None);
        };

        // When the executor supports stdio tunnels and no explicit docker_host
        // is configured, restart the proxy with the detected runtime's command
        // so all subsequent bollard calls use the correct binary.
        #[cfg(unix)]
        if self.executor.supports_stdio_tunnel() && self.config.docker_host.is_none() {
            let dial_cmd = match runtime {
                ContainerRuntime::Docker => "docker system dial-stdio",
                ContainerRuntime::Podman => "podman system dial-stdio",
                ContainerRuntime::Auto => "docker system dial-stdio",
            };

            tracing::info!(
                runtime = ?runtime,
                cmd = %dial_cmd,
                "restarting Docker socket proxy with detected runtime"
            );

            let proxy =
                crate::docker_proxy::DockerSocketProxy::start(Arc::clone(&self.executor), dial_cmd)
                    .await
                    .map_err(|e| {
                        rootcause::report!(crate::error::DockerError::DaemonConnection(
                            e.to_string()
                        ))
                    })?;
            let uri = proxy.socket_uri();
            let new_client = Arc::new(BollardDockerClient::new(Some(&uri), None, None)?)
                as Arc<dyn DockerClient>;
            let handle: Box<dyn std::any::Any + Send + Sync> = Box::new(proxy);

            *self.docker_client.lock() = new_client;
            *self.proxy_handle.lock() = Some(handle);
        }

        Ok(Some(runtime))
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

#[async_trait]
impl uptrakit_plugin_infrastructure_core::DiscoveryPlugin for DockerPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<DiscoveredSoftware>> {
        use std::collections::HashMap;
        use uptrakit_plugin_infrastructure_core::{DiscoveryTarget, PluginRole, PluginType};

        let client = Arc::clone(&*self.docker_client.lock());
        let containers = client.list_containers(true).await.map_err(|e| {
            uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(e.to_string())
        })?;

        // Always emit a DiscoveryTarget so the controller can find-or-create
        // a "Docker" plugin config and the role assignments.

        // Inspect each unique image ref only once.  Multiple containers using
        // the same image share the same digest, so there is no point calling
        // the Docker daemon more than once per image.
        let mut digest_cache: HashMap<String, Option<crate::docker_client::LocalImageDigest>> =
            HashMap::new();

        let mut discoveries = Vec::new();

        for container in containers {
            let raw_image = container.image.trim();
            if raw_image.is_empty() {
                continue;
            }

            // Skip bare SHA refs — they have no registry provenance.
            if raw_image.starts_with("sha256:") {
                continue;
            }

            // Parse the image ref (may or may not have a tag).
            let ir: ImageRef = match raw_image.parse() {
                Ok(r) => r,
                Err(_) => continue,
            };

            // Take the first container name, stripping any leading '/'.
            let container_name = match container.names.first() {
                Some(n) => n.trim_start_matches('/').to_string(),
                None => continue,
            };
            if container_name.is_empty() {
                continue;
            }

            // Apply label filter when labels are populated (may be empty from list_containers).
            if !self.container_passes_label_filter(&container.labels) {
                continue;
            }

            // Inspect the image once, then reuse the cached result for every
            // subsequent container that references the same image.
            let digest_info = match digest_cache.entry(ir.full_ref.clone()) {
                std::collections::hash_map::Entry::Occupied(e) => match e.get().clone() {
                    Some(d) => d,
                    // Already determined: no registry digest (locally built). Skip.
                    None => continue,
                },
                std::collections::hash_map::Entry::Vacant(e) => {
                    let client = Arc::clone(&*self.docker_client.lock());
                    let outcome = match client.inspect_image(&ir.full_ref).await {
                        Ok(Some(d)) => {
                            tracing::debug!(
                                image = %ir.full_ref,
                                digest = %d.digest,
                                "inspected image for discovery"
                            );
                            Some(d)
                        }
                        Ok(None) => {
                            tracing::debug!(
                                image = %ir.full_ref,
                                "skipping locally built image (no RepoDigests)"
                            );
                            None
                        }
                        Err(err) => {
                            tracing::warn!(
                                image = %ir.full_ref,
                                error = %err,
                                "failed to inspect image during discovery"
                            );
                            None
                        }
                    };
                    let digest_opt = outcome.clone();
                    e.insert(outcome);
                    match digest_opt {
                        Some(d) => d,
                        None => continue,
                    }
                }
            };

            // Image-level package identifier: shared by all containers using the same image.
            let pkg_id = ir.full_ref.clone();

            // Software item name: just the image reference.
            let name = ir.full_ref.clone();

            // Container-qualified identifier used for per-container plugin operations.
            // Stored in host_software_item_plugin.package_identifier so execute_update
            // can target the specific container.
            let plugin_pkg_id = format!("{}#{}", ir.full_ref, container_name);

            // Compute platform from the installed image's inspect data.
            let platform = crate::config::form_platform_string(
                digest_info.os.as_deref(),
                digest_info.architecture.as_deref(),
                digest_info.variant.as_deref(),
            );
            let config_override = platform.as_deref().map(|p| json!({"platform": p}));

            let targets = vec![DiscoveryTarget {
                plugin_type: PluginType::ReleasesDocker,
                plugin_config: json!({}),
                plugin_config_name: "Docker".to_string(),
                roles: vec![
                    PluginRole::DetectVersion,
                    PluginRole::FetchReleases,
                    PluginRole::ExecuteUpdate,
                ],
                package_identifier: None,
                config_override,
                execution_site: None,
            }];

            discoveries.push(DiscoveredSoftware {
                package_identifier: pkg_id,
                name,
                installed_version: digest_info.digest,
                targets,
                extra: Some(json!({ "container": container_name })),
                qualifier: Some(container_name.clone()),
                plugin_package_identifier: Some(plugin_pkg_id),
                featured: true,
            });
        }

        tracing::debug!(count = discoveries.len(), "docker autodiscovery completed");
        Ok(discoveries)
    }

    #[cfg(feature = "daemon")]
    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<
        uptrakit_plugin_infrastructure_core::HostCompatibility,
    > {
        // When container_runtime is Auto, probe the executor to discover which
        // runtime (Docker or Podman) is available. For SSH executors that support
        // stdio tunnels we also restart the proxy with the correct command so
        // all subsequent daemon operations use the right runtime.
        if self.config.container_runtime == ContainerRuntime::Auto {
            match self.detect_and_apply_runtime().await {
                Ok(Some(rt)) => {
                    *self.detected_runtime.lock() = Some(rt);
                }
                Ok(None) => {
                    return Ok(HostCompatibility::Incompatible(
                        "no container runtime (Docker or Podman) found on this host".to_string(),
                    ));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "runtime detection failed; proceeding with current client");
                }
            }
        }

        const COMPAT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
        let client = Arc::clone(&*self.docker_client.lock());
        match tokio::time::timeout(COMPAT_PROBE_TIMEOUT, client.ping()).await {
            Ok(Ok(())) => Ok(HostCompatibility::Compatible),
            Ok(Err(e)) => Ok(HostCompatibility::Incompatible(format!(
                "Docker daemon not accessible: {e}"
            ))),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "Docker daemon ping timed out".to_string(),
            )),
        }
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetectorPlugin for DockerPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(
        &self,
        package_identifier: &str,
    ) -> uptrakit_plugin_infrastructure_core::Result<Option<Version>> {
        let ir: ImageRef =
            package_identifier
                .parse()
                .map_err(|e: crate::image_ref::ParseImageRefError| {
                    PluginError::PluginInternal(e.to_string())
                })?;

        let tag = self.config.resolved_tracked_tag(&ir.tag);
        let full_ref = format!("{}:{tag}", ir.image);

        let client = Arc::clone(&*self.docker_client.lock());
        match client.inspect_image(&full_ref).await {
            Ok(Some(digest_info)) => {
                tracing::debug!(
                    digest = %digest_info.digest,
                    image = %ir.image,
                    "detected installed digest"
                );

                // When a platform is known (from config override or auto-detected from
                // the local image), compare platform-specific digests to avoid false
                // positives when only a different platform is updated.
                let platform = self.config.platform.clone().or_else(|| {
                    crate::config::form_platform_string(
                        digest_info.os.as_deref(),
                        digest_info.architecture.as_deref(),
                        digest_info.variant.as_deref(),
                    )
                });

                if let Some(ref p) = platform {
                    match self
                        .registry_client
                        .get_platform_manifest_digest(&ir.registry, &ir.repository, tag, p)
                        .await
                    {
                        Ok(Some(platform_digest)) => {
                            tracing::debug!(
                                platform = %p,
                                digest = %platform_digest,
                                image = %ir.image,
                                "resolved platform-specific digest"
                            );
                            return Ok(Some(Version::new(&platform_digest)));
                        }
                        Ok(None) => {
                            // Platform removed from the manifest list.
                            return Err(
                                uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(
                                    crate::error::DockerError::PlatformNotAvailable {
                                        platform: p.clone(),
                                        image: ir.image.clone(),
                                        tag: tag.to_string(),
                                    }
                                    .to_string(),
                                )
                                .into(),
                            );
                        }
                        Err(e) => {
                            // Transient registry failure — fall back to the image index digest
                            // so that a network hiccup does not block version detection.
                            tracing::warn!(
                                error = %e,
                                image = %ir.image,
                                "failed to fetch platform manifest digest; \
                                 falling back to image index digest"
                            );
                        }
                    }
                }

                Ok(Some(Version::new(&digest_info.digest)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(
                uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(e.to_string())
                    .into(),
            ),
        }
    }

    /// Batch installed-version detection with image-level deduplication.
    ///
    /// Multiple containers that share the same image (after `tracked_tag`
    /// resolution) are inspected only once, avoiding redundant Docker daemon
    /// calls for the common case where several items use e.g. `nginx:latest`.
    #[tracing::instrument(skip_all)]
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<BatchDetectResult>> {
        use std::collections::HashMap;

        // Daemon inspect cache: "image:tag" → Result<Option<LocalImageDigest>, String>.
        let mut inspect_cache: HashMap<
            String,
            std::result::Result<Option<crate::docker_client::LocalImageDigest>, String>,
        > = HashMap::new();

        // Platform digest cache: "image:tag::platform" → Option<String>.
        let mut platform_cache: HashMap<String, Option<String>> = HashMap::new();

        // Populate inspect cache.
        for item in items {
            let ir: ImageRef = match item.package_identifier.parse::<ImageRef>() {
                Ok(r) => r,
                Err(_) => continue,
            };
            let tag = self.config.resolved_tracked_tag(&ir.tag);
            let resolved = format!("{}:{tag}", ir.image);

            if inspect_cache.contains_key(&resolved) {
                continue;
            }

            let client = Arc::clone(&*self.docker_client.lock());
            let outcome = match client.inspect_image(&resolved).await {
                Ok(Some(d)) => Ok(Some(d)),
                Ok(None) => Ok(None),
                Err(e) => Err(e.to_string()),
            };
            inspect_cache.insert(resolved, outcome);
        }

        let mut results = Vec::with_capacity(items.len());
        for item in items {
            let ir: ImageRef = match item.package_identifier.parse::<ImageRef>() {
                Ok(r) => r,
                Err(e) => {
                    results.push(BatchDetectResult::error(
                        item.package_identifier.clone(),
                        e.to_string(),
                    ));
                    continue;
                }
            };
            let tag = self.config.resolved_tracked_tag(&ir.tag);
            let resolved = format!("{}:{tag}", ir.image);

            match inspect_cache.get(&resolved) {
                Some(Ok(Some(digest_info))) => {
                    let platform = self.config.platform.clone().or_else(|| {
                        crate::config::form_platform_string(
                            digest_info.os.as_deref(),
                            digest_info.architecture.as_deref(),
                            digest_info.variant.as_deref(),
                        )
                    });

                    if let Some(ref p) = platform {
                        let cache_key = format!("{resolved}::{p}");
                        let platform_digest = match platform_cache.entry(cache_key) {
                            std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
                            std::collections::hash_map::Entry::Vacant(e) => {
                                let result = self
                                    .registry_client
                                    .get_platform_manifest_digest(
                                        &ir.registry,
                                        &ir.repository,
                                        tag,
                                        p,
                                    )
                                    .await
                                    .ok()
                                    .flatten();
                                e.insert(result.clone());
                                result
                            }
                        };

                        match platform_digest {
                            Some(pd) => {
                                results.push(BatchDetectResult::found(
                                    item.package_identifier.clone(),
                                    Version::new(&pd),
                                ));
                                continue;
                            }
                            None => {
                                // Platform not in manifest list — treat as not found.
                                results.push(BatchDetectResult::not_found(
                                    item.package_identifier.clone(),
                                ));
                                continue;
                            }
                        }
                    }

                    results.push(BatchDetectResult::found(
                        item.package_identifier.clone(),
                        Version::new(&digest_info.digest),
                    ));
                }
                Some(Ok(None)) | None => {
                    results.push(BatchDetectResult::not_found(
                        item.package_identifier.clone(),
                    ));
                }
                Some(Err(e)) => {
                    results.push(BatchDetectResult::error(
                        item.package_identifier.clone(),
                        e.clone(),
                    ));
                }
            }
        }

        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin for DockerPlugin {
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(
        &self,
        package_identifier: &str,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<UpstreamRelease>> {
        let ir: ImageRef =
            package_identifier
                .parse()
                .map_err(|e: crate::image_ref::ParseImageRefError| {
                    uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(e.to_string())
                })?;

        let tag = self.config.resolved_tracked_tag(&ir.tag);

        let digest = if let Some(ref platform) = self.config.platform {
            match self
                .registry_client
                .get_platform_manifest_digest(&ir.registry, &ir.repository, tag, platform)
                .await
                .context_to()?
            {
                Some(d) => d,
                None => {
                    return Err(
                        uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(
                            crate::error::DockerError::PlatformNotAvailable {
                                platform: platform.clone(),
                                image: ir.image.clone(),
                                tag: tag.to_string(),
                            }
                            .to_string(),
                        )
                        .into(),
                    );
                }
            }
        } else {
            self.registry_client
                .get_manifest_digest(&ir.registry, &ir.repository, tag)
                .await
                .context_to()?
        };

        let release_url = ir.web_url(&digest);
        let release = {
            let mut r = UpstreamRelease::new(Version::new(&digest), tag.to_string(), false, "");
            r.release_url = release_url;
            r
        };

        tracing::debug!(
            digest = %digest,
            tag = %tag,
            image = %ir.image,
            platform = ?self.config.platform,
            "fetched Docker release (digest mode)"
        );
        Ok(vec![release])
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin for DockerPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_plugin_infrastructure_core::Result<String> {
        let ir: ImageRef =
            package_identifier
                .parse()
                .map_err(|e: crate::image_ref::ParseImageRefError| {
                    PluginError::PluginInternal(e.to_string())
                })?;

        let image = &ir.image;
        // Always pull by the configured tag (e.g. "latest"), not by digest.
        let tag = self.config.resolved_tracked_tag(&ir.tag);
        let full_ref = format!("{image}:{tag}");
        let mut output = String::new();

        // Pre-pull: collect running/stopped state of the containers to recreate.
        //
        // When the package identifier carries a `#container_name` qualifier (e.g.
        // `nginx:latest#web-server`), only that specific container is targeted.
        // Without a qualifier all containers using this image are managed, which
        // preserves behaviour for items created before per-container tracking was
        // introduced.
        let client = Arc::clone(&*self.docker_client.lock());
        let all_containers = client
            .list_containers_for_image(&full_ref)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    image = %full_ref,
                    error = %e,
                    "failed to list containers before pull; recreation will be skipped"
                );
                vec![]
            });

        let containers_before: Vec<_> = if let Some(ref target) = ir.container_name {
            all_containers
                .into_iter()
                .filter(|c| c.name == *target && self.container_passes_label_filter(&c.labels))
                .collect()
        } else {
            all_containers
                .into_iter()
                .filter(|c| self.container_passes_label_filter(&c.labels))
                .collect()
        };

        send_output(
            output_tx,
            &format!("Pulling Docker image {image}:{tag}"),
            OutputStreamType::Stdout,
        )
        .await;
        output.push_str(&format!("Pulling Docker image {image}:{tag}\n"));

        tracing::debug!(image = %image, tag = %tag, "pulling Docker image");
        #[cfg(feature = "daemon")]
        let auth = self.effective_auth(image).await;
        #[cfg(not(feature = "daemon"))]
        let auth: Option<crate::config::DockerAuth> = self.config.auth.clone();
        let client = Arc::clone(&*self.docker_client.lock());
        let pull_output = client
            .pull_image(image, tag, auth.as_ref(), output_tx)
            .await
            .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
        tracing::debug!("Docker image pull completed");
        output.push_str(&pull_output);

        // Run compose_restart if configured.
        // Direction: any containers running before pull -> `up -d` (recreate and start);
        // all stopped -> `up --no-start` (recreate without starting).
        if let Some(ref cr) = self.config.compose_restart {
            let any_running = containers_before.iter().any(|c| c.is_running);

            let mut parts: Vec<String> = Vec::new();

            if let Some(ref working_dir) = cr.working_dir {
                parts.push(format!("cd {}", shell_escape(working_dir)));
                parts.push("&&".to_string());
            }

            parts.push("docker".to_string());
            parts.push("compose".to_string());

            if let Some(ref file) = cr.compose_file {
                parts.push("-f".to_string());
                parts.push(shell_escape(file));
            }

            parts.push("up".to_string());
            if any_running {
                parts.push("-d".to_string());
            } else {
                parts.push("--no-start".to_string());
            }

            if let Some(ref service) = cr.service {
                parts.push(shell_escape(service));
            }

            let cmd = parts.join(" ");
            tracing::debug!(command = %cmd, "running docker compose restart");
            let compose_msg = format!("Running docker compose: {cmd}");
            send_output(output_tx, &compose_msg, OutputStreamType::Stdout).await;
            output.push_str(&compose_msg);
            output.push('\n');

            let cmd_output = self
                .executor
                .execute(&CommandSpec::shell(&cmd), output_tx)
                .await
                .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            output.push_str(&cmd_output.output);
        }

        // Run post_pull_command if configured.
        if let Some(ref cmd_str) = self.config.post_pull_command {
            // Try to get local digest for {digest} substitution.
            let client = Arc::clone(&*self.docker_client.lock());
            let digest = match client.inspect_image(&full_ref).await {
                Ok(Some(d)) => d.digest,
                _ => String::new(),
            };

            let cmd = cmd_str
                .replace("{image}", &shell_escape(image))
                .replace("{tag}", &shell_escape(tag))
                .replace("{digest}", &shell_escape(&digest));

            tracing::debug!(command = %cmd, "running post-pull command");
            send_output(
                output_tx,
                &format!("Running post-pull command: {cmd}"),
                OutputStreamType::Stdout,
            )
            .await;

            let cmd_output = self
                .executor
                .execute(&CommandSpec::shell(&cmd), output_tx)
                .await
                .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            output.push_str(&cmd_output.output);
        }

        // Auto-recreate containers when neither compose_restart nor post_pull_command
        // is configured.  Containers are recreated in-place, preserving all settings.
        // Running containers are started again; stopped containers remain stopped.
        if self.config.compose_restart.is_none() && self.config.post_pull_command.is_none() {
            for container in &containers_before {
                tracing::info!(
                    container = %container.name,
                    was_running = container.is_running,
                    "recreating container after image update"
                );
                let line = format!(
                    "Recreating container {} (was {})",
                    container.name,
                    if container.is_running {
                        "running"
                    } else {
                        "stopped"
                    }
                );
                send_output(output_tx, &line, OutputStreamType::Stdout).await;
                output.push_str(&line);
                output.push('\n');

                let client = Arc::clone(&*self.docker_client.lock());
                client
                    .recreate_container(&container.name, container.is_running)
                    .await
                    .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            }
        }

        Ok(output)
    }
}

#[cfg(all(test, feature = "daemon"))]
mod tests {
    use super::*;
    use crate::docker_client::{ContainerForImage, LocalContainerInfo, MockDockerClient};
    use uptrakit_plugin_infrastructure_core::LocalCommandExecutor;
    use uptrakit_plugin_infrastructure_core::mpsc;
    use uptrakit_plugin_infrastructure_core::{
        DiscoveryPlugin, PluginBase, UpdateExecutorPlugin, VersionDetectorPlugin,
    };

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    /// A mock executor that records commands without executing them.
    struct MockCommandExecutor;

    #[async_trait::async_trait]
    impl CommandExecutor for MockCommandExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<uptrakit_plugin_infrastructure_core::CommandOutput> {
            Ok(uptrakit_plugin_infrastructure_core::CommandOutput {
                output: String::new(),
                exit_code: 0,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<uptrakit_plugin_infrastructure_core::CommandOutput> {
            Ok(uptrakit_plugin_infrastructure_core::CommandOutput {
                output: String::new(),
                exit_code: 0,
            })
        }
    }

    fn mock_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(MockCommandExecutor)
    }

    /// A mock executor that simulates runtime detection probes.
    ///
    /// `probe_results` is a list of exit codes returned in order for each
    /// call to `execute_quiet`. Index 0 = first call (docker check),
    /// index 1 = second call (podman check), etc.
    struct DetectionMockExecutor {
        probe_results: Vec<i32>,
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl DetectionMockExecutor {
        fn new(results: Vec<i32>) -> Self {
            Self {
                probe_results: results,
                call_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl CommandExecutor for DetectionMockExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<uptrakit_plugin_infrastructure_core::CommandOutput> {
            Ok(uptrakit_plugin_infrastructure_core::CommandOutput {
                output: String::new(),
                exit_code: 0,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<uptrakit_plugin_infrastructure_core::CommandOutput> {
            let idx = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let exit_code = self.probe_results.get(idx).copied().unwrap_or(1);
            Ok(uptrakit_plugin_infrastructure_core::CommandOutput {
                output: String::new(),
                exit_code,
            })
        }
    }

    fn default_mock_client() -> Arc<dyn DockerClient> {
        Arc::new(MockDockerClient::default())
    }

    #[test]
    fn plugin_creation_succeeds_with_empty_config() {
        let config = DockerConfig::default();
        assert!(DockerPlugin::new_for_test(config, test_executor(), default_mock_client()).is_ok());
    }

    #[test]
    fn capabilities_includes_discover_local_software() {
        let plugin = DockerPlugin::new_for_test(
            DockerConfig::default(),
            test_executor(),
            default_mock_client(),
        )
        .unwrap();
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
    }

    #[test]
    fn capabilities_includes_detect_host_compatibility() {
        let plugin = DockerPlugin::new_for_test(
            DockerConfig::default(),
            test_executor(),
            default_mock_client(),
        )
        .unwrap();
        assert!(plugin.has_capability(PluginCapability::DetectHostCompatibility));
    }

    #[test]
    fn capabilities_excludes_refresh_package_index() {
        let plugin = DockerPlugin::new_for_test(
            DockerConfig::default(),
            test_executor(),
            default_mock_client(),
        )
        .unwrap();
        assert!(!plugin.has_capability(PluginCapability::RefreshPackageIndex));
    }

    // ── detect_host_compatibility ─────────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_daemon_reachable() {
        let mock = Arc::new(MockDockerClient::default());
        // Use DetectionMockExecutor that returns 0 so docker is found during Auto probe.
        let executor = Arc::new(DetectionMockExecutor::new(vec![0]));
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_daemon_unreachable() {
        let mock = Arc::new(MockDockerClient {
            ping_should_fail: true,
            ..Default::default()
        });
        // Use DetectionMockExecutor that returns 0 so docker is found during Auto probe.
        let executor = Arc::new(DetectionMockExecutor::new(vec![0]));
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert!(
                    msg.contains("Docker daemon"),
                    "reason should mention Docker daemon: {msg}"
                );
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn detect_host_compatibility_incompatible_when_daemon_times_out() {
        let mock = Arc::new(MockDockerClient {
            ping_should_hang: true,
            ..Default::default()
        });
        // Use DetectionMockExecutor that returns exit 0 for the docker probe so
        // runtime detection succeeds, then the daemon ping hangs and must time out.
        let executor = Arc::new(DetectionMockExecutor::new(vec![0]));
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
        // Spawn so we can advance virtual time while the probe is in flight.
        let check = tokio::spawn(async move { plugin.detect_host_compatibility().await });
        tokio::task::yield_now().await;
        // Advance past the 5-second COMPAT_PROBE_TIMEOUT.
        tokio::time::advance(std::time::Duration::from_secs(10)).await;
        let result = check.await.expect("join").expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert!(
                    msg.contains("timed out"),
                    "reason should mention timeout: {msg}"
                );
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }

    #[tokio::test]
    async fn detect_installed_version_returns_digest_when_image_present() {
        let digest = "sha256:abc123def456".to_string();
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some(digest.clone()),
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let result = plugin.detect_installed_version("nginx").await.unwrap();
        assert_eq!(result.map(|v| v.to_string()), Some(digest));
    }

    #[tokio::test]
    async fn detect_installed_version_returns_none_when_image_absent() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: None,
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let result = plugin.detect_installed_version("nginx").await.unwrap();
        assert!(result.is_none());
    }

    // ── execute_update ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_update_pulls_by_tag_not_digest() {
        let pull_output = "mock pull output".to_string();
        let mock = Arc::new(MockDockerClient {
            pull_output: pull_output.clone(),
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        // `to_version` is the digest — execute_update must pull by tag ("latest"), not by digest.
        let result = plugin
            .execute_update("nginx", "sha256:deadbeef", None, &tx)
            .await
            .expect("execute_update should succeed");

        assert!(
            result.contains("Pulling Docker image nginx:latest"),
            "should pull by tag, not digest: {result}"
        );
        assert!(result.contains(&pull_output));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_pull_failure_propagates_error() {
        let mock = Arc::new(MockDockerClient {
            pull_should_fail: true,
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:deadbeef", None, &tx)
            .await;

        assert!(result.is_err(), "pull failure should be propagated");

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_recreates_running_containers() {
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "my-nginx".to_string(),
                is_running: true,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update should succeed");

        assert!(result.contains("Recreating container my-nginx"));
        assert!(result.contains("running"));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_recreates_stopped_containers() {
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "stopped-nginx".to_string(),
                is_running: false,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update should succeed");

        assert!(result.contains("Recreating container stopped-nginx"));
        assert!(result.contains("stopped"));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_recreate_failure_propagates_error() {
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "bad-container".to_string(),
                is_running: true,
                labels: Default::default(),
            }],
            recreate_should_fail: true,
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await;

        assert!(result.is_err(), "recreate failure should propagate");

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_no_containers_succeeds() {
        // No containers for this image — pull succeeds, recreation loop is a no-op.
        let mock = Arc::new(MockDockerClient::default());
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("should succeed with no containers");

        assert!(result.contains("Pulling Docker image nginx:latest"));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_with_post_pull_command_skips_recreation() {
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "my-nginx".to_string(),
                is_running: true,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let config = DockerConfig {
            post_pull_command: Some("echo post-pull {image}:{tag}".to_string()),
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, test_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update with post_pull_command should succeed");

        assert!(result.contains("Pulling Docker image nginx:latest"));
        // post_pull_command is set, so auto-recreate must be skipped
        assert!(
            !result.contains("Recreating container"),
            "recreation should be skipped when post_pull_command is set"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_with_compose_restart_running_uses_detach() {
        use crate::config::ComposeRestartConfig;

        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "my-nginx".to_string(),
                is_running: true,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: Some("docker-compose.yml".to_string()),
                service: Some("myapp".to_string()),
                working_dir: None,
            }),
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, mock_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update with compose_restart should succeed");

        // When containers were running, compose command must include `-d`
        assert!(result.contains("docker compose"));
        assert!(result.contains("-d"), "running state should use -d flag");
        assert!(
            !result.contains("--no-start"),
            "should not use --no-start when containers were running"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_with_compose_restart_stopped_uses_no_start() {
        use crate::config::ComposeRestartConfig;

        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "my-nginx".to_string(),
                is_running: false,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: None,
                service: Some("myapp".to_string()),
                working_dir: None,
            }),
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, mock_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update with compose_restart should succeed");

        // When containers were stopped, compose command must include `--no-start`
        assert!(result.contains("docker compose"));
        assert!(
            result.contains("--no-start"),
            "stopped state should use --no-start flag"
        );
        assert!(
            !result.contains(" -d "),
            "should not use -d when containers were stopped"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_tracked_tag_override_respected() {
        let mock = Arc::new(MockDockerClient::default());
        let config = DockerConfig {
            tracked_tag: Some("stable".to_string()),
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, test_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("should succeed");

        assert!(
            result.contains("Pulling Docker image nginx:stable"),
            "should pull by configured tracked_tag: {result}"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    // ── discover_software ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_one_item_per_container() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["web-server".to_string()],
                    labels: Default::default(),
                },
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["api-proxy".to_string()],
                    labels: Default::default(),
                },
            ],
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let mut discoveries = plugin.discover_software().await.unwrap();
        // Two containers → two software items, even though both use the same image.
        assert_eq!(discoveries.len(), 2);

        // Both items share the same image-level package_identifier and name.
        // The container name is carried in `qualifier` and `plugin_package_identifier`.
        discoveries.sort_by(|a, b| {
            a.qualifier
                .as_deref()
                .unwrap_or("")
                .cmp(b.qualifier.as_deref().unwrap_or(""))
        });
        assert_eq!(discoveries[0].package_identifier, "nginx:latest");
        assert_eq!(discoveries[0].name, "nginx:latest");
        assert_eq!(discoveries[0].installed_version, "sha256:abc123");
        assert_eq!(discoveries[0].qualifier.as_deref(), Some("api-proxy"));
        assert_eq!(
            discoveries[0].plugin_package_identifier.as_deref(),
            Some("nginx:latest#api-proxy")
        );

        assert_eq!(discoveries[1].package_identifier, "nginx:latest");
        assert_eq!(discoveries[1].name, "nginx:latest");
        assert_eq!(discoveries[1].installed_version, "sha256:abc123");
        assert_eq!(discoveries[1].qualifier.as_deref(), Some("web-server"));
        assert_eq!(
            discoveries[1].plugin_package_identifier.as_deref(),
            Some("nginx:latest#web-server")
        );
    }

    #[tokio::test]
    async fn discover_software_single_container_uses_image_based_name() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["my-nginx".to_string()],
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();

        assert_eq!(discoveries.len(), 1);
        // Package identifier is the image reference (shared across containers).
        assert_eq!(discoveries[0].package_identifier, "nginx:latest");
        // Name is just the image reference.
        assert_eq!(discoveries[0].name, "nginx:latest");
        assert_eq!(discoveries[0].installed_version, "sha256:abc123");
        // Container name is carried in qualifier and plugin_package_identifier.
        assert_eq!(discoveries[0].qualifier.as_deref(), Some("my-nginx"));
        assert_eq!(
            discoveries[0].plugin_package_identifier.as_deref(),
            Some("nginx:latest#my-nginx")
        );
    }

    #[tokio::test]
    async fn discover_software_strips_leading_slash_from_container_name() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![LocalContainerInfo {
                image: "nginx:latest".to_string(),
                // BollardDockerClient strips the leading '/' before returning
                // LocalContainerInfo, but the mock may supply pre-stripped names.
                names: vec!["my-nginx".to_string()],
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert_eq!(discoveries.len(), 1);
        // package_identifier is the image-level identifier; qualifier holds the container name.
        assert_eq!(discoveries[0].package_identifier, "nginx:latest");
        assert_eq!(discoveries[0].qualifier.as_deref(), Some("my-nginx"));
        assert_eq!(
            discoveries[0].plugin_package_identifier.as_deref(),
            Some("nginx:latest#my-nginx")
        );
    }

    #[tokio::test]
    async fn discover_software_skips_sha_images() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![LocalContainerInfo {
                image: "sha256:deadbeef".to_string(),
                names: vec!["bare-sha-container".to_string()],
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert!(discoveries.is_empty(), "SHA images should be skipped");
    }

    #[tokio::test]
    async fn discover_software_skips_images_without_repo_digests() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: None, // No digest — locally built
            containers: vec![LocalContainerInfo {
                image: "my-local-image:dev".to_string(),
                names: vec!["local-container".to_string()],
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert!(
            discoveries.is_empty(),
            "images without RepoDigests should be skipped"
        );
    }

    // ── discover_software target emission ─────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_targets_when_default_config() {
        use uptrakit_plugin_infrastructure_core::{PluginRole, PluginType};

        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["my-nginx".to_string()],
                labels: Default::default(),
            }],
            ..Default::default()
        });
        // Default config (empty `{}`) → discover-all mode → targets must be emitted.
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();

        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].targets.len(), 1);
        let target = &discoveries[0].targets[0];
        assert_eq!(target.plugin_type, PluginType::ReleasesDocker);
        assert_eq!(target.plugin_config_name, "Docker");
        assert_eq!(target.plugin_config, serde_json::json!({}));
        assert!(target.roles.contains(&PluginRole::DetectVersion));
        assert!(target.roles.contains(&PluginRole::FetchReleases));
        assert!(target.roles.contains(&PluginRole::ExecuteUpdate));
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_custom_config() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["my-nginx".to_string()],
                labels: Default::default(),
            }],
            ..Default::default()
        });
        // Custom config still emits targets.
        let config = DockerConfig {
            docker_host: Some("unix:///var/run/docker.sock".to_string()),
            ..Default::default()
        };
        let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();

        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].targets.len(), 1);
    }

    // ── execute_update — container-qualified identifiers ──────────────────────

    #[tokio::test]
    async fn execute_update_with_container_qualifier_only_recreates_named_container() {
        // Two containers share the same image.
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![
                ContainerForImage {
                    name: "web-server".to_string(),
                    is_running: true,
                    labels: Default::default(),
                },
                ContainerForImage {
                    name: "api-proxy".to_string(),
                    is_running: true,
                    labels: Default::default(),
                },
            ],
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        // Container-qualified identifier: only "web-server" should be touched.
        let result = plugin
            .execute_update("nginx:latest#web-server", "sha256:abc", None, &tx)
            .await
            .expect("execute_update should succeed");

        assert!(
            result.contains("Recreating container web-server"),
            "web-server must be recreated: {result}"
        );
        assert!(
            !result.contains("Recreating container api-proxy"),
            "api-proxy must NOT be recreated: {result}"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_without_qualifier_recreates_all_containers() {
        // Unqualified identifier (no `#container_name`) → legacy behaviour.
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![
                ContainerForImage {
                    name: "web-server".to_string(),
                    is_running: true,
                    labels: Default::default(),
                },
                ContainerForImage {
                    name: "api-proxy".to_string(),
                    is_running: true,
                    labels: Default::default(),
                },
            ],
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx:latest", "sha256:abc", None, &tx)
            .await
            .expect("should succeed");

        assert!(
            result.contains("Recreating container web-server"),
            "web-server must be recreated: {result}"
        );
        assert!(
            result.contains("Recreating container api-proxy"),
            "api-proxy must be recreated: {result}"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_container_not_found_succeeds_silently() {
        // The named container is not in the list returned by list_containers_for_image.
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "other-container".to_string(),
                is_running: true,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx:latest#missing-container", "sha256:abc", None, &tx)
            .await
            .expect("should succeed even when container not found");

        // Pull happened but no containers were recreated.
        assert!(
            result.contains("Pulling Docker image nginx:latest"),
            "should pull the image: {result}"
        );
        assert!(
            !result.contains("Recreating"),
            "no containers should be recreated: {result}"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    // ── batch_detect_installed_version ────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_deduplicates_inspections_for_shared_image() {
        use std::sync::Arc as StdArc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Custom mock that counts inspect_image calls.
        struct CountingMockClient {
            inspect_count: StdArc<AtomicUsize>,
            digest: String,
        }

        #[async_trait::async_trait]
        impl DockerClient for CountingMockClient {
            #[cfg(feature = "daemon")]
            async fn ping(&self) -> crate::error::Result<()> {
                Ok(())
            }

            async fn pull_image(
                &self,
                _image: &str,
                _tag: &str,
                _auth: Option<&crate::config::DockerAuth>,
                _output_tx: &mpsc::Sender<UpdateOutputLine>,
            ) -> crate::error::Result<String> {
                Ok(String::new())
            }

            async fn inspect_image(
                &self,
                _full_ref: &str,
            ) -> crate::error::Result<Option<crate::docker_client::LocalImageDigest>> {
                self.inspect_count.fetch_add(1, Ordering::SeqCst);
                Ok(Some(crate::docker_client::LocalImageDigest {
                    digest: self.digest.clone(),
                    os: None,
                    architecture: None,
                    variant: None,
                }))
            }

            async fn list_containers(
                &self,
                _all: bool,
            ) -> crate::error::Result<Vec<crate::docker_client::LocalContainerInfo>> {
                Ok(vec![])
            }

            async fn list_containers_for_image(
                &self,
                _full_ref: &str,
            ) -> crate::error::Result<Vec<crate::docker_client::ContainerForImage>> {
                Ok(vec![])
            }

            async fn recreate_container(
                &self,
                _name: &str,
                _was_running: bool,
            ) -> crate::error::Result<()> {
                Ok(())
            }
        }

        let inspect_count = StdArc::new(AtomicUsize::new(0));
        let mock = Arc::new(CountingMockClient {
            inspect_count: StdArc::clone(&inspect_count),
            digest: "sha256:abc".to_string(),
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");

        // Three items all using the same image via container-qualified identifiers.
        let items = vec![
            BatchDetectItem::new("nginx:latest#web-server".to_string()),
            BatchDetectItem::new("nginx:latest#api-proxy".to_string()),
            BatchDetectItem::new("nginx:latest#worker".to_string()),
        ];

        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("batch detect should succeed");

        // All three get the digest.
        assert_eq!(results.len(), 3);
        for r in &results {
            assert_eq!(
                r.installed_version
                    .as_ref()
                    .map(|v| v.to_string())
                    .as_deref(),
                Some("sha256:abc")
            );
            assert!(r.error.is_none());
        }

        // Exactly one inspect call despite three items (deduplication).
        assert_eq!(
            inspect_count.load(Ordering::SeqCst),
            1,
            "image should be inspected only once regardless of how many containers use it"
        );
    }

    #[tokio::test]
    async fn batch_detect_returns_none_for_uninstalled_image() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: None,
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");

        let items = vec![BatchDetectItem::new("nginx:latest#web-server".to_string())];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");
        assert_eq!(results.len(), 1);
        assert!(results[0].installed_version.is_none());
        assert!(results[0].error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_handles_unqualified_identifiers() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:def456".to_string()),
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");

        let items = vec![BatchDetectItem::new("nginx".to_string())];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0]
                .installed_version
                .as_ref()
                .map(|v| v.to_string())
                .as_deref(),
            Some("sha256:def456")
        );
    }

    // ── Label filter ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_include_label_filter_skips_non_matching() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["managed-nginx".to_string()],
                    labels: [("com.example.managed".to_string(), "true".to_string())]
                        .into_iter()
                        .collect(),
                },
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["unmanaged-nginx".to_string()],
                    labels: Default::default(),
                },
            ],
            ..Default::default()
        });
        let mut config = DockerConfig::default();
        config
            .include_labels
            .insert("com.example.managed".to_string(), "true".to_string());
        let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].qualifier.as_deref(), Some("managed-nginx"));
    }

    #[tokio::test]
    async fn discover_software_exclude_label_filter_skips_excluded() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["prod-nginx".to_string()],
                    labels: Default::default(),
                },
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["dev-nginx".to_string()],
                    labels: [("env".to_string(), "dev".to_string())]
                        .into_iter()
                        .collect(),
                },
            ],
            ..Default::default()
        });
        let mut config = DockerConfig::default();
        config
            .exclude_labels
            .insert("env".to_string(), "dev".to_string());
        let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].qualifier.as_deref(), Some("prod-nginx"));
    }

    #[tokio::test]
    async fn discover_software_no_label_filter_includes_all() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["a".to_string()],
                    labels: Default::default(),
                },
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["b".to_string()],
                    labels: [("x".to_string(), "y".to_string())].into_iter().collect(),
                },
            ],
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert_eq!(discoveries.len(), 2);
    }

    // ── ContainerRuntime detection ────────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compat_auto_selects_docker_when_available() {
        // probe_results[0] = docker check returns 0 (found)
        let executor = Arc::new(DetectionMockExecutor::new(vec![0]));
        let mock = Arc::new(MockDockerClient::default());
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
        assert_eq!(
            *plugin.detected_runtime.lock(),
            Some(ContainerRuntime::Docker)
        );
        assert_eq!(
            plugin.effective_dial_stdio_command(),
            "docker system dial-stdio"
        );
    }

    #[tokio::test]
    async fn detect_host_compat_auto_selects_podman_when_only_podman_found() {
        // probe_results[0] = docker returns 1 (not found), [1] = podman returns 0
        let executor = Arc::new(DetectionMockExecutor::new(vec![1, 0]));
        let mock = Arc::new(MockDockerClient::default());
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
        assert_eq!(
            *plugin.detected_runtime.lock(),
            Some(ContainerRuntime::Podman)
        );
        assert_eq!(
            plugin.effective_dial_stdio_command(),
            "podman system dial-stdio"
        );
    }

    #[tokio::test]
    async fn detect_host_compat_auto_incompatible_when_neither_found() {
        // Both docker and podman checks fail
        let executor = Arc::new(DetectionMockExecutor::new(vec![1, 1]));
        let mock = Arc::new(MockDockerClient::default());
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert!(
                    msg.contains("container runtime"),
                    "message should mention container runtime: {msg}"
                );
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn effective_dial_stdio_command_docker_explicit() {
        let config = DockerConfig {
            container_runtime: ContainerRuntime::Docker,
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, test_executor(), default_mock_client()).unwrap();
        assert_eq!(
            plugin.effective_dial_stdio_command(),
            "docker system dial-stdio"
        );
    }

    #[test]
    fn effective_dial_stdio_command_podman_explicit() {
        let config = DockerConfig {
            container_runtime: ContainerRuntime::Podman,
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, test_executor(), default_mock_client()).unwrap();
        assert_eq!(
            plugin.effective_dial_stdio_command(),
            "podman system dial-stdio"
        );
    }

    #[test]
    fn effective_dial_stdio_command_auto_defaults_to_docker() {
        let plugin = DockerPlugin::new_for_test(
            DockerConfig::default(),
            test_executor(),
            default_mock_client(),
        )
        .unwrap();
        // No detection run yet: defaults to docker
        assert_eq!(
            plugin.effective_dial_stdio_command(),
            "docker system dial-stdio"
        );
    }
}
